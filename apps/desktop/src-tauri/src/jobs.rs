//! In-memory job queue. A run clears the queue, then submits one job per
//! matching file and polls it on a background task, capped at 4 in flight.

use serde::Serialize;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Semaphore;

use crate::conversion_service::{self, ConversionFailure, ConversionJob};
use crate::providers::{self, PollResult, ProviderKind};
use crate::{backend_host, history, secrets, settings};

/// One client for every provider request in the process. A `Client` owns the
/// connection pool, so one per job means a TLS handshake per file. No global
/// timeout: each request sets its own (see `providers::*_TIMEOUT`).
fn provider_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new())
        })
        .clone()
}

/// How patient the first poll of a recovered conversion is: 12s covers a
/// sidecar still opening its database.
const RESUME_MAX_ATTEMPTS: u32 = 6;
const RESUME_RETRY_INTERVAL: Duration = Duration::from_secs(2);

/// Every 5s for ~60 minutes, bailing after ~1 minute of unbroken errors.
const BACKEND_POLL_INTERVAL: Duration = Duration::from_secs(5);
const BACKEND_MAX_POLLS: u32 = 720;
const BACKEND_MAX_CONSECUTIVE_ERRORS: u32 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BackendPhase {
    SubmitOrPoll,
    DatalabFallback,
}

/// A remote fallback already started for this file. The provider is recorded
/// before the request is sent, so no request id means it may be billed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FallbackContext {
    /// `datalab` or `datalab_pipeline`. A resume polls the endpoint the request
    /// went to, not whatever Settings say now.
    pub(crate) provider: String,
    pub(crate) request_id: Option<String>,
    pub(crate) check_url: Option<String>,
}

const DATALAB_PROVIDER: &str = "datalab";
const DATALAB_PIPELINE_PROVIDER: &str = "datalab_pipeline";

#[derive(Clone, Debug)]
pub(crate) struct BackendContext {
    /// The origin this job was queued against, a stand-in for the instant the
    /// host cannot answer. Every request resolves the live one through
    /// `request_origin`, because the sidecar's port moves on a restart.
    pub(crate) backend_url: String,
    pub(crate) client_run_id: String,
    pub(crate) idempotency_key: String,
    pub(crate) backend_job_id: Option<String>,
    pub(crate) profile: settings::ConversionProfile,
    /// The OCR options this submission belongs to. Reading Settings at submit
    /// time resubmits a stored key under a changed fingerprint, which 409s.
    pub(crate) ocr: conversion_service::OcrOptions,
    phase: BackendPhase,
    fallback: Option<FallbackContext>,
    recovery_blocker: Option<String>,
}

impl BackendContext {
    pub(crate) fn new(
        backend_url: String,
        client_run_id: String,
        idempotency_key: String,
        backend_job_id: Option<String>,
        profile: settings::ConversionProfile,
        ocr: conversion_service::OcrOptions,
    ) -> Self {
        Self {
            backend_url,
            client_run_id,
            idempotency_key,
            backend_job_id,
            profile,
            ocr,
            phase: BackendPhase::SubmitOrPoll,
            fallback: None,
            recovery_blocker: None,
        }
    }

    /// Resume a fallback the ledger recorded, rather than paying twice.
    pub(crate) fn resuming_fallback(mut self, fallback: FallbackContext) -> Self {
        self.phase = BackendPhase::DatalabFallback;
        self.fallback = Some(fallback);
        self
    }
}

/// The "job to be done" the user picks once for the whole run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobType {
    Convert,
    Transcribe,
}

impl JobType {
    pub fn from_id(s: &str) -> Option<Self> {
        match s {
            "convert" => Some(JobType::Convert),
            "transcribe" => Some(JobType::Transcribe),
            _ => None,
        }
    }

    /// The string this job is filed under in history. Frozen once rows exist.
    pub fn id(&self) -> &'static str {
        match self {
            JobType::Convert => "convert",
            JobType::Transcribe => "transcribe",
        }
    }

    pub fn provider(&self) -> ProviderKind {
        match self {
            JobType::Convert => ProviderKind::Datalab,
            JobType::Transcribe => ProviderKind::RevAi,
        }
    }

    pub fn accepts(&self, ext: &str) -> bool {
        match self {
            JobType::Convert => matches!(
                ext,
                "pdf"
                    | "png"
                    | "jpg"
                    | "jpeg"
                    | "webp"
                    | "tiff"
                    | "tif"
                    | "gif"
                    | "bmp"
                    | "docx"
                    | "doc"
                    | "pptx"
                    | "ppt"
                    | "xlsx"
                    | "xls"
                    | "html"
                    | "htm"
                    | "epub"
            ),
            JobType::Transcribe => matches!(
                ext,
                "mp3"
                    | "mp4"
                    | "wav"
                    | "m4a"
                    | "flac"
                    | "ogg"
                    | "oga"
                    | "aac"
                    | "mov"
                    | "avi"
                    | "mkv"
                    | "webm"
                    | "wmv"
                    | "mpeg"
                    | "mpg"
                    | "opus"
                    | "amr"
                    | "3gp"
            ),
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: u64,
    pub file_name: String,
    pub source_path: String,
    pub output_dir: String,
    pub job_type: JobType,
    pub service: String,
    pub status: String, // queued | working | processing | done | failed
    pub progress_note: String,
    pub output_path: Option<String>,
    pub error: Option<String>,
    /// Stringly typed, so a new backend value cannot break an older desktop.
    pub route: Option<String>,
    pub reason_codes: Vec<String>,
    pub warnings: Vec<String>,
    pub failure: Option<ConversionFailure>,
    /// When this job left the queue, so the elapsed timer excludes the wait.
    pub started_at: Option<u64>,
    #[serde(skip)]
    pub(crate) backend: Option<BackendContext>,
}

impl Job {
    pub fn new(id: u64, source_path: String, output_dir: String, job_type: JobType) -> Self {
        let file_name = Path::new(&source_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        Job {
            id,
            file_name,
            source_path,
            output_dir,
            job_type,
            service: job_type.provider().label().to_string(),
            status: "queued".into(),
            progress_note: "Queued".into(),
            output_path: None,
            error: None,
            route: None,
            reason_codes: Vec::new(),
            warnings: Vec::new(),
            failure: None,
            started_at: None,
            backend: None,
        }
    }

    pub(crate) fn new_backend(
        id: u64,
        source_path: String,
        output_dir: String,
        job_type: JobType,
        backend: BackendContext,
    ) -> Self {
        let mut job = Self::new(id, source_path, output_dir, job_type);
        job.service = match job_type {
            JobType::Convert => "Conversion service",
            JobType::Transcribe => "Local transcription",
        }
        .into();
        job.backend = Some(backend);
        job
    }
}

fn now_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub struct JobManager {
    jobs: Mutex<Vec<Job>>,
    counter: AtomicU64,
    sem: Arc<Semaphore>,
    /// Bumped by every new run and by Stop. A spawned task captures it and
    /// aborts once it stops matching, so a retired run stops spending.
    generation: AtomicU64,
    /// Settings snapshot at run start, so a mid-run change splits no run.
    run_config: Mutex<settings::Settings>,
}

/// Holds the jobs lock through the caller's side effects and emit, so
/// retirement cannot split a guarded mutation from its signal.
struct GenerationUpdate<'a> {
    job: Job,
    _jobs: MutexGuard<'a, Vec<Job>>,
}

impl GenerationUpdate<'_> {
    fn job(&self) -> &Job {
        &self.job
    }

    fn commit<R>(self, commit: impl FnOnce(Job) -> R) -> R {
        let Self { job, _jobs } = self;
        let result = commit(job);
        drop(_jobs);
        result
    }

    fn emit(self, app: &AppHandle) {
        self.commit(|job| emit(app, job));
    }
}

impl Default for JobManager {
    fn default() -> Self {
        Self {
            jobs: Mutex::new(Vec::new()),
            counter: AtomicU64::new(0),
            sem: Arc::new(Semaphore::new(4)), // up to 4 files in flight at once
            generation: AtomicU64::new(0),
            run_config: Mutex::new(settings::Settings::default()),
        }
    }
}

impl JobManager {
    pub fn next_id(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::SeqCst) + 1
    }
    pub fn insert(&self, job: Job) {
        self.jobs.lock().unwrap().push(job);
    }
    pub fn list(&self) -> Vec<Job> {
        self.jobs.lock().unwrap().clone()
    }
    pub fn get(&self, id: u64) -> Option<Job> {
        self.jobs
            .lock()
            .unwrap()
            .iter()
            .find(|j| j.id == id)
            .cloned()
    }
    pub fn clear(&self) {
        self.jobs.lock().unwrap().clear();
    }
    pub fn semaphore(&self) -> Arc<Semaphore> {
        self.sem.clone()
    }
    pub fn update<F: FnOnce(&mut Job)>(&self, id: u64, f: F) -> Option<Job> {
        let mut jobs = self.jobs.lock().unwrap();
        let job = jobs.iter_mut().find(|j| j.id == id)?;
        f(job);
        Some(job.clone())
    }

    fn update_if_generation<F: FnOnce(&mut Job)>(
        &self,
        id: u64,
        expected_generation: u64,
        f: F,
    ) -> Option<GenerationUpdate<'_>> {
        let mut jobs = self.jobs.lock().unwrap();
        if self.generation() != expected_generation {
            return None;
        }
        let updated = {
            let job = jobs.iter_mut().find(|job| job.id == id)?;
            f(job);
            job.clone()
        };
        Some(GenerationUpdate {
            job: updated,
            _jobs: jobs,
        })
    }

    /// Retire every in-flight task and return the new generation.
    pub fn new_generation(&self) -> u64 {
        let jobs = self.jobs.lock().unwrap();
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        drop(jobs);
        generation
    }
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }
    pub fn set_run_config(&self, cfg: settings::Settings) {
        *self.run_config.lock().unwrap() = cfg;
    }
    pub fn run_config(&self) -> settings::Settings {
        self.run_config.lock().unwrap().clone()
    }
}

/// What "the same output" means when deciding whether a file is already done.
/// The pipeline id is folded in: it changes what Convert produces.
pub fn output_format_for(jt: JobType, cfg: &settings::Settings) -> String {
    match jt {
        JobType::Transcribe => "text".to_string(),
        JobType::Convert => match cfg
            .datalab_pipeline_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(pid) => format!("{}@{pid}", cfg.datalab_format),
            None => cfg.datalab_format.clone(),
        },
    }
}

/// The extension a finished result carries. Read by both writers and by the
/// tree's pairing rule, so a third copy would let them disagree. Not
/// `output_format_for`: an extension has no room for a pipeline id.
pub fn output_extension_for(jt: JobType, cfg: &settings::Settings) -> &'static str {
    match jt {
        JobType::Transcribe => "txt",
        JobType::Convert => match cfg.datalab_format.as_str() {
            "html" => "html",
            "json" | "chunks" => "json",
            _ => "md",
        },
    }
}

/// Every extension a finished result may carry on the route in force, which is
/// what the tree's pairing rule has to ask. The Backend route writes Markdown
/// whatever the format or the job says, and only Convert's Datalab fallback
/// writes the chosen format, so pairing on the format alone never matches a
/// service result. Transcribe's Direct `.txt` stays paired for older runs.
pub fn result_extensions_for(jt: JobType, cfg: &settings::Settings) -> Vec<&'static str> {
    // Route-independent: both are transcripts of the same source, and dropping
    // `.md` on a route flip unpairs every local transcript and re-bills it.
    if jt == JobType::Transcribe {
        return vec!["md", "txt"];
    }
    let chosen = output_extension_for(jt, cfg);
    if cfg.conversion_route == settings::ConversionRoute::Backend {
        // Markdown first: the service writes it, the fallback is the odd one.
        let mut both = vec!["md"];
        if chosen != "md" {
            both.push(chosen);
        }
        return both;
    }
    vec![chosen]
}

fn emit(app: &AppHandle, job: Job) {
    let _ = app.emit("job-updated", job);
}

/// File a terminal job in the history. Best-effort: `history::record` swallows
/// storage errors. A user Stop is skipped, or a cancelled run buries the log.
fn log_history(app: &AppHandle, job: &Job, status: &str, error: Option<&str>) {
    if error == Some("Stopped") {
        return;
    }
    let cfg = app.state::<JobManager>().run_config();
    let output_format = match job.backend.as_ref().map(|backend| backend.phase) {
        Some(BackendPhase::SubmitOrPoll) => "backend:markdown".to_string(),
        Some(BackendPhase::DatalabFallback) => {
            format!("backend_fallback:{}", output_format_for(job.job_type, &cfg))
        }
        None => output_format_for(job.job_type, &cfg),
    };
    history::record(
        app,
        &history::Finished {
            file_name: &job.file_name,
            source_path: &job.source_path,
            output_path: job.output_path.as_deref(),
            job_type: job.job_type.id(),
            output_format: &output_format,
            status,
            error,
        },
    );
}

/// Move a job's visible state, only while it still belongs to the current run.
/// A false answer means "stop here": a stopped row must not be relabelled.
/// Emitting is conditional, because the poll repeats one note every 5s.
fn set_status(app: &AppHandle, id: u64, generation: u64, status: &str, note: &str) -> bool {
    let manager = app.state::<JobManager>();
    let mut changed = false;
    let Some(updated) = manager.update_if_generation(id, generation, |job| {
        changed = job.status != status || job.progress_note != note;
        if changed {
            job.status = status.to_string();
            job.progress_note = note.to_string();
        }
    }) else {
        return false;
    };
    if changed {
        updated.emit(app);
    }
    true
}

/// Same emit rule as `set_status`: the poll repeats one view until it moves.
fn apply_backend_view(app: &AppHandle, id: u64, generation: u64, view: &ConversionJob) -> bool {
    let manager = app.state::<JobManager>();
    let kind = view.route.as_ref().map(|route| route.kind.clone());
    let reason_codes = view
        .route
        .as_ref()
        .map(|route| route.reason_codes.clone())
        .unwrap_or_default();
    let mut changed = false;
    let Some(updated) = manager.update_if_generation(id, generation, |job| {
        changed = job.route != kind
            || job.reason_codes != reason_codes
            || job.warnings != view.warnings
            || job.failure != view.failure;
        if changed {
            job.route = kind;
            job.reason_codes = reason_codes;
            job.warnings = view.warnings.clone();
            job.failure = view.failure.clone();
        }
    }) else {
        return false;
    };
    if changed {
        updated.emit(app);
    }
    true
}

fn fail_backend_retryable(app: &AppHandle, id: u64, generation: u64, err: &str) {
    if let Some(updated) = app
        .state::<JobManager>()
        .update_if_generation(id, generation, |job| {
            job.status = "failed".into();
            job.progress_note = "Retry to continue this conversion".into();
            job.error = Some(err.to_string());
        })
    {
        updated.emit(app);
    }
}

/// Generation-guarded: a submit can outlive Stop and file a false failure.
fn fail(app: &AppHandle, id: u64, generation: u64, err: &str) {
    if let Some(updated) = app
        .state::<JobManager>()
        .update_if_generation(id, generation, |job| {
            job.status = "failed".into();
            job.progress_note.clear();
            job.error = Some(err.to_string());
        })
    {
        let job = updated.job();
        if let Some(backend) = &job.backend {
            history::delete_in_flight(app, &backend.idempotency_key);
        }
        log_history(app, job, "failed", Some(err));
        updated.emit(app);
    }
}

/// The converted text is already on disk, so the job row carries only its path.
/// Copy reads the file rather than shipping every result across IPC. If Stop
/// retired the task while the file was written, remove exactly that new path.
fn finish(app: &AppHandle, id: u64, generation: u64, output_path: String) {
    let cleanup_path = output_path.clone();
    if let Some(updated) = app
        .state::<JobManager>()
        .update_if_generation(id, generation, |job| {
            job.status = "done".into();
            job.progress_note.clear();
            job.output_path = Some(output_path);
            job.error = None;
            job.failure = None;
        })
    {
        let job = updated.job();
        if let Some(backend) = &job.backend {
            history::delete_in_flight(app, &backend.idempotency_key);
        }
        log_history(app, job, "done", None);
        updated.emit(app);
    } else {
        let _ = std::fs::remove_file(cleanup_path);
    }
}

/// Claim a free name in `output_dir` and hand back the file holding it.
/// `create_new` makes the claim and the existence check one syscall, so two
/// jobs finishing together cannot agree on a name and nothing is overwritten.
/// Shared by the writer and the importer, so both number a collision alike.
pub(crate) fn claim_path(
    output_dir: &str,
    file_name: &str,
    ext: &str,
) -> Result<(std::fs::File, std::path::PathBuf), String> {
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    // A source can carry no extension, so this arm only runs for an import.
    let dotted = if ext.is_empty() {
        String::new()
    } else {
        format!(".{ext}")
    };
    let base = std::path::PathBuf::from(output_dir);

    for n in 0..1000 {
        let candidate = if n == 0 {
            base.join(format!("{stem}{dotted}"))
        } else {
            base.join(format!("{stem} ({n}){dotted}"))
        };
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(f) => return Ok((f, candidate)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Could not write to the output folder: {e}")),
        }
    }
    Err(format!(
        "Could not find a free filename for {stem}{dotted} in the output folder."
    ))
}

fn write_output(
    output_dir: &str,
    file_name: &str,
    ext: &str,
    text: &str,
) -> Result<String, String> {
    use std::io::Write;

    let (mut f, candidate) = claim_path(output_dir, file_name, ext)?;
    f.write_all(text.as_bytes())
        .and_then(|_| f.sync_all())
        .map(|_| candidate.to_string_lossy().to_string())
        .map_err(|e| format!("Could not write {}: {e}", candidate.display()))
}

/// Copy a source file into a folder, byte for byte, and say where it landed.
/// Bytes, not `read_to_string`: this carries PDFs, office documents and video.
///
/// The modification time travels with the bytes. `tree::pair_results` refuses a
/// result older than its source, and `history` refuses a source whose mtime
/// moved, so a re-stamped copy is paid for twice.
pub fn import_source(dir: &str, source: &Path) -> Result<String, String> {
    let file_name = source
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("{} has no file name", source.display()))?;
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();

    let mut reader = std::fs::File::open(source)
        .map_err(|e| format!("Could not read {}: {e}", source.display()))?;
    let modified = reader.metadata().and_then(|m| m.modified()).ok();

    let (mut f, candidate) = claim_path(dir, file_name, ext)?;
    std::io::copy(&mut reader, &mut f)
        .and_then(|_| f.sync_all())
        .map_err(|e| format!("Could not write {}: {e}", candidate.display()))?;
    // Best effort: a filesystem that refuses the stamp still holds the bytes.
    if let Some(when) = modified {
        let _ = f.set_modified(when);
    }
    Ok(candidate.to_string_lossy().to_string())
}

/// Satisfy a job from a result an earlier run produced, rather than paying for
/// it again. Reached when the history holds a still-valid result for this file,
/// job and format in another folder. Returns whether the copy succeeded: a
/// failure lands on the job row, and Retry then runs it for real.
pub fn reuse_result(app: &AppHandle, id: u64, generation: u64, existing: &str) -> bool {
    let Some(job) = app.state::<JobManager>().get(id) else {
        return false;
    };
    let text = match std::fs::read_to_string(existing) {
        Ok(t) => t,
        Err(e) => {
            fail(
                app,
                id,
                generation,
                &format!("Could not read the earlier result: {e}"),
            );
            return false;
        }
    };
    // The extension comes from the file being copied, so a result written
    // under an older format setting keeps its own suffix.
    let ext = Path::new(existing)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("txt");
    match write_output(&job.output_dir, &job.file_name, ext, &text) {
        Ok(path) => {
            finish(app, id, generation, path);
            true
        }
        Err(e) => {
            fail(app, id, generation, &e);
            false
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum BackendAction {
    Pending(&'static str),
    Succeeded,
    Failed(String),
    NeedsRemote,
}

/// An allowlist: a status a newer backend adds stays pending, not terminal.
fn backend_action(view: &ConversionJob) -> BackendAction {
    match view.status.as_str() {
        "succeeded" => BackendAction::Succeeded,
        "failed" => {
            let message = view.failure.as_ref().map_or_else(
                || "Conversion failed without details".to_string(),
                |failure| format!("{}: {}", failure.code, failure.message),
            );
            BackendAction::Failed(message)
        }
        "needs_remote" => BackendAction::NeedsRemote,
        "queued" => BackendAction::Pending("Queued by conversion service…"),
        "converting_local" => BackendAction::Pending("Converting locally…"),
        "finalizing" => BackendAction::Pending("Finalizing…"),
        _ => BackendAction::Pending("Processing…"),
    }
}

/// A `needs_remote` view carries one engine reason code. Name it in plain words.
fn local_only_failure(view: &ConversionJob) -> String {
    let code = view
        .route
        .as_ref()
        .and_then(|route| route.reason_codes.first())
        .map_or("", String::as_str);
    let reason = match code {
        "" => return "Local-only conversion could not finish locally. Choose Standard to allow Datalab fallback.".into(),
        "mixed_pdf" => "some pages are scanned images",
        "image_based_pdf" | "scanned_pdf" => "every page is a scanned image",
        "ocr_required" => "the text layer is missing or unreadable",
        "garbled_text" => "the text layer is garbled",
        "local_quality_failed" => "no readable text was found",
        "output_too_large" => "the result was too large",
        other => other,
    };
    format!("Local-only conversion could not finish because {reason}. Choose Standard to allow Datalab fallback.")
}

/// The origin one request goes to, resolved at the moment of the request. The
/// kernel hands the sidecar a new port at every restart, so a queued origin is
/// refused once the child relaunches. `held` covers only the mid-restart gap.
fn request_origin(live: Result<String, String>, held: &str) -> String {
    live.unwrap_or_else(|_| held.to_string())
}

async fn run_backend_job(app: AppHandle, id: u64, generation: u64, job: Job) {
    let stale = |app: &AppHandle| app.state::<JobManager>().generation() != generation;
    let Some(backend) = job.backend.clone() else {
        return;
    };

    let sem = app.state::<JobManager>().semaphore();
    let permit = sem.acquire_owned().await;
    if stale(&app) {
        return;
    }
    let Ok(_permit) = permit else {
        fail_backend_retryable(&app, id, generation, "The conversion queue is unavailable");
        return;
    };
    let manager = app.state::<JobManager>();
    let Some(updated) = manager.update_if_generation(id, generation, |current| {
        current.started_at = Some(now_secs());
    }) else {
        return;
    };
    updated.emit(&app);

    if let Some(error) = &backend.recovery_blocker {
        fail_backend_retryable(&app, id, generation, error);
        return;
    }
    if backend.backend_job_id.is_none()
        && history::in_flight_source_for_key_is_current(&app, &backend.idempotency_key)
            == Some(false)
    {
        fail_backend_retryable(
            &app,
            id,
            generation,
            "The source changed after this conversion was queued; stop it before starting a new conversion",
        );
        return;
    }

    if backend.phase == BackendPhase::DatalabFallback {
        run_datalab_fallback(&app, id, generation, &job).await;
        return;
    }

    let token = match backend_host::backend_token(&app) {
        Ok(token) => token,
        Err(error) => {
            fail_backend_retryable(&app, id, generation, &error);
            return;
        }
    };

    let mut view = if let Some(backend_job_id) = backend.backend_job_id.clone() {
        if !set_status(&app, id, generation, "processing", "Resuming conversion…") {
            return;
        }
        // The first request can land before the service answers, and one
        // refused connection must not fail work it has already done.
        let mut attempts = 0u32;
        loop {
            let origin = request_origin(backend_host::backend_origin(&app), &backend.backend_url);
            let result =
                conversion_service::poll_conversion(&origin, &token, &backend_job_id).await;
            if stale(&app) {
                return;
            }
            match result {
                Ok(view) => break view,
                Err(error) => {
                    attempts += 1;
                    if attempts >= RESUME_MAX_ATTEMPTS {
                        fail_backend_retryable(&app, id, generation, &error);
                        return;
                    }
                }
            }
            tokio::time::sleep(RESUME_RETRY_INTERVAL).await;
            if stale(&app) {
                return;
            }
        }
    } else {
        if !set_status(
            &app,
            id,
            generation,
            "working",
            "Uploading to conversion service…",
        ) {
            return;
        }
        let origin = request_origin(backend_host::backend_origin(&app), &backend.backend_url);
        let result = conversion_service::submit_conversion(
            &origin,
            &token,
            &job.source_path,
            &backend.client_run_id,
            backend.profile.id(),
            &backend.ocr,
            &backend.idempotency_key,
        )
        .await;
        if stale(&app) {
            return;
        }
        match result {
            Ok(view) => {
                if app
                    .state::<JobManager>()
                    .update_if_generation(id, generation, |current| {
                        if let Some(context) = &mut current.backend {
                            context.backend_job_id = Some(view.id.clone());
                        }
                    })
                    .is_none()
                {
                    return;
                }
                history::attach_backend_job(&app, &backend.idempotency_key, &view.id);
                view
            }
            Err(error) => {
                fail_backend_retryable(&app, id, generation, &error);
                return;
            }
        }
    };

    let mut polls = 0u32;
    let mut consecutive_errors = 0u32;
    loop {
        if stale(&app) {
            return;
        }
        if !apply_backend_view(&app, id, generation, &view) {
            return;
        }
        match backend_action(&view) {
            BackendAction::Succeeded => {
                if !set_status(&app, id, generation, "processing", "Saving Markdown…") {
                    return;
                }
                let origin =
                    request_origin(backend_host::backend_origin(&app), &backend.backend_url);
                let output = conversion_service::download_markdown(
                    &origin,
                    &token,
                    &view.id,
                    &job.output_dir,
                    &job.file_name,
                )
                .await;
                match output {
                    Ok(path) => finish(&app, id, generation, path),
                    Err(error) => fail_backend_retryable(&app, id, generation, &error),
                }
                return;
            }
            BackendAction::Failed(error) => {
                fail(&app, id, generation, &error);
                return;
            }
            BackendAction::NeedsRemote => {
                // The service never sends this for audio. The guard is what
                // keeps a recording away from Datalab if it ever does.
                if job.job_type == JobType::Transcribe {
                    fail(
                        &app,
                        id,
                        generation,
                        "Local transcription could not finish. Switch the route to Direct to use Rev.ai.",
                    );
                    return;
                }
                if backend.profile == settings::ConversionProfile::LocalOnly {
                    fail(&app, id, generation, &local_only_failure(&view));
                    return;
                }
                if app
                    .state::<JobManager>()
                    .update_if_generation(id, generation, |current| {
                        current.service = "Datalab fallback".into();
                        if let Some(context) = &mut current.backend {
                            context.phase = BackendPhase::DatalabFallback;
                        }
                    })
                    .is_none()
                {
                    return;
                }
                run_datalab_fallback(&app, id, generation, &job).await;
                return;
            }
            BackendAction::Pending(note) => {
                if !set_status(&app, id, generation, "processing", note) {
                    return;
                }
            }
        }

        polls += 1;
        if polls > BACKEND_MAX_POLLS {
            fail_backend_retryable(
                &app,
                id,
                generation,
                "Timed out waiting for the conversion service",
            );
            return;
        }
        tokio::time::sleep(BACKEND_POLL_INTERVAL).await;
        if stale(&app) {
            return;
        }
        // `poll_conversion` refuses an answer about any other id, so the view
        // always names this job.
        let origin = request_origin(backend_host::backend_origin(&app), &backend.backend_url);
        let result = conversion_service::poll_conversion(&origin, &token, &view.id).await;
        if stale(&app) {
            return;
        }
        match result {
            Ok(polled) => {
                consecutive_errors = 0;
                view = polled;
            }
            Err(error) => {
                consecutive_errors += 1;
                if consecutive_errors >= BACKEND_MAX_CONSECUTIVE_ERRORS {
                    fail_backend_retryable(
                        &app,
                        id,
                        generation,
                        &format!("Lost contact with the conversion service: {error}"),
                    );
                    return;
                }
            }
        }
    }
}

async fn run_datalab_fallback(app: &AppHandle, id: u64, generation: u64, original: &Job) {
    let stale = |app: &AppHandle| app.state::<JobManager>().generation() != generation;
    if stale(app) {
        return;
    }
    let Some(backend) = original.backend.clone() else {
        return;
    };
    // A recorded provider with no request id means the process died between
    // the ledger write and the outcome. Datalab cannot be asked whether it took
    // the upload, so a resubmit risks a second charge.
    if let Some(fallback) = &backend.fallback {
        if fallback.request_id.is_none() {
            history::delete_in_flight(app, &backend.idempotency_key);
            fail(
                app,
                id,
                generation,
                "A Datalab fallback for this file was interrupted and may already have been \
                 charged. Convert it again only if the earlier attempt produced nothing.",
            );
            return;
        }
    }
    let api_key = match secrets::get_key("datalab") {
        Some(key) if !key.trim().is_empty() => key,
        _ => {
            fail_backend_retryable(
                app,
                id,
                generation,
                "Datalab fallback is required. Add its API key in Settings, then retry.",
            );
            return;
        }
    };
    let cfg = app.state::<JobManager>().run_config();
    let client = provider_client();
    let pipeline = cfg
        .datalab_pipeline_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    // A resumed request is polled the way it was submitted, whatever Settings
    // say about the pipeline id now.
    let via_pipeline = match backend.fallback.as_ref() {
        Some(fallback) => fallback.provider == DATALAB_PIPELINE_PROVIDER,
        None => pipeline.is_some(),
    };
    let provider = if via_pipeline {
        DATALAB_PIPELINE_PROVIDER
    } else {
        DATALAB_PROVIDER
    };

    if stale(app) {
        return;
    }
    if !set_status(
        app,
        id,
        generation,
        "working",
        "Uploading to Datalab fallback…",
    ) {
        return;
    }
    // Resume an accepted request instead of buying a second one.
    let resumed = backend
        .fallback
        .as_ref()
        .and_then(|fallback| fallback.request_id.clone());
    let submitted = match resumed {
        Some(remote_id) => providers::Submitted {
            check_url: backend
                .fallback
                .as_ref()
                .and_then(|fallback| fallback.check_url.clone()),
            remote_id,
        },
        None => {
            // Two writes before any billable request: the ledger survives a
            // crash, the in-memory context survives a Retry. In-memory first,
            // so a retired run leaves no ledger row for a submit never sent.
            if app
                .state::<JobManager>()
                .update_if_generation(id, generation, |current| {
                    if let Some(context) = &mut current.backend {
                        context.fallback = Some(FallbackContext {
                            provider: provider.to_owned(),
                            request_id: None,
                            check_url: None,
                        });
                    }
                })
                .is_none()
            {
                return;
            }
            if !history::begin_fallback(app, &backend.idempotency_key, provider) {
                // Nothing was sent, so drop the claim. Left set, every later
                // Retry meets the uncertainty guard.
                app.state::<JobManager>()
                    .update_if_generation(id, generation, |current| {
                        if let Some(context) = &mut current.backend {
                            context.fallback = None;
                        }
                    });
                fail(
                    app,
                    id,
                    generation,
                    "Cannot record the Datalab fallback before sending it, so it will not be \
                     sent. Retry once the conversion history is writable.",
                );
                return;
            }
            let result = providers::datalab_submit_any(
                &client,
                &api_key,
                pipeline,
                &original.source_path,
                &cfg.datalab_format,
                cfg.datalab_high_accuracy,
            )
            .await;
            if stale(app) {
                return;
            }
            match result {
                Ok(value) => value,
                // Datalab answered and refused, so the claim above is a claim
                // on nothing. Drop both halves, or Retry meets the guard.
                Err(providers::SubmitError::Refused(error)) => {
                    app.state::<JobManager>()
                        .update_if_generation(id, generation, |current| {
                            if let Some(context) = &mut current.backend {
                                context.fallback = None;
                            }
                        });
                    history::clear_fallback(app, &backend.idempotency_key);
                    fail_backend_retryable(app, id, generation, error.as_str());
                    return;
                }
                // No answer, so the upload may have landed with no id to poll.
                // The claim stands and the row says so.
                Err(providers::SubmitError::Uncertain(error)) => {
                    fail(
                        app,
                        id,
                        generation,
                        &format!(
                            "{error}. The upload may already have been charged. Convert this \
                             file again only if that attempt produced nothing."
                        ),
                    );
                    return;
                }
            }
        }
    };
    let check_url = submitted.datalab_check_url();
    history::attach_fallback_request(
        app,
        &backend.idempotency_key,
        &submitted.remote_id,
        &check_url,
    );
    if app
        .state::<JobManager>()
        .update_if_generation(id, generation, |current| {
            if let Some(context) = &mut current.backend {
                context.fallback = Some(FallbackContext {
                    provider: provider.to_owned(),
                    request_id: Some(submitted.remote_id.clone()),
                    check_url: Some(check_url.clone()),
                });
            }
        })
        .is_none()
    {
        return;
    }
    if !set_status(
        app,
        id,
        generation,
        "processing",
        "Processing with Datalab fallback…",
    ) {
        return;
    }

    let mut attempts = 0u32;
    let mut consecutive_errors = 0u32;
    let text = loop {
        attempts += 1;
        if attempts > BACKEND_MAX_POLLS {
            fail_backend_retryable(
                app,
                id,
                generation,
                "Timed out waiting for Datalab fallback",
            );
            return;
        }
        tokio::time::sleep(BACKEND_POLL_INTERVAL).await;
        if stale(app) {
            return;
        }
        let result = providers::datalab_poll_any(
            &client,
            &api_key,
            via_pipeline,
            &submitted.remote_id,
            &check_url,
            &cfg.datalab_format,
        )
        .await;
        if stale(app) {
            return;
        }
        match result {
            Ok(PollResult::Done(text)) => break text,
            // `Failed` is terminal. Filing it as retryable leaves a ledger row
            // that resurrects on every launch, and no History entry.
            Ok(PollResult::Failed(error)) => {
                fail(app, id, generation, &error);
                return;
            }
            Ok(PollResult::Pending) => consecutive_errors = 0,
            Err(error) => {
                consecutive_errors += 1;
                if consecutive_errors >= BACKEND_MAX_CONSECUTIVE_ERRORS {
                    fail_backend_retryable(
                        app,
                        id,
                        generation,
                        &format!("Lost contact with Datalab fallback: {error}"),
                    );
                    return;
                }
            }
        }
    };

    let extension = output_extension_for(JobType::Convert, &cfg);
    if stale(app) {
        return;
    }
    match write_output(&original.output_dir, &original.file_name, extension, &text) {
        Ok(path) => finish(app, id, generation, path),
        Err(error) => fail_backend_retryable(app, id, generation, &error),
    }
}

/// Point a context at the service running now, and clear the recovery blocker
/// the last launch left. Nothing else clears it, so Retry re-fails all session.
fn refresh_for_retry(context: &mut BackendContext, origin: Result<&str, String>) {
    match origin {
        Ok(origin) => {
            context.backend_url = origin.to_string();
            context.recovery_blocker = None;
        }
        Err(error) => context.recovery_blocker = Some(error),
    }
}

/// Re-persist a backend job's durable row before a retry. Stop deletes the
/// ledger row while the job stays retryable, so without this a crash during the
/// resubmit loses the job.
pub fn restore_in_flight(app: &AppHandle, id: u64) {
    let manager = app.state::<JobManager>();
    // Retry-all runs this over every failed row, so direct jobs leave here.
    if manager.get(id).is_none_or(|job| job.backend.is_none()) {
        return;
    }
    // Read fresh: a restarted sidecar is on a different port.
    let origin = backend_host::backend_origin(app);
    let Some(job) = manager.update(id, |current| {
        if let Some(context) = &mut current.backend {
            refresh_for_retry(context, origin.as_deref().map_err(Clone::clone));
        }
    }) else {
        return;
    };
    let Some(backend) = job.backend else {
        return;
    };
    let Ok(deployment) = backend_host::deployment(app) else {
        return;
    };
    let ocr_custom_words = backend.ocr.custom_words_wire();
    if !history::upsert_in_flight(
        app,
        &history::NewInFlight {
            source_path: &job.source_path,
            file_name: &job.file_name,
            output_dir: &job.output_dir,
            backend_url: backend_host::ledger_origin(&deployment),
            client_run_id: &backend.client_run_id,
            idempotency_key: &backend.idempotency_key,
            conversion_profile: backend.profile.id(),
            job_type: job.job_type.id(),
            ocr_language_correction: backend.ocr.language_correction,
            ocr_custom_words: &ocr_custom_words,
            speaker_count: backend.ocr.speaker_count,
        },
    ) {
        return;
    }
    for step in ledger_replay(&backend) {
        match step {
            LedgerReplay::AttachBackendJob(job_id) => {
                history::attach_backend_job(app, &backend.idempotency_key, &job_id);
            }
            LedgerReplay::BeginFallback(provider) => {
                history::begin_fallback(app, &backend.idempotency_key, &provider);
            }
            LedgerReplay::AttachFallbackRequest {
                request_id,
                check_url,
            } => {
                history::attach_fallback_request(
                    app,
                    &backend.idempotency_key,
                    &request_id,
                    &check_url,
                );
            }
        }
    }
}

/// One ledger write needed to rebuild the remote state Stop deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LedgerReplay {
    AttachBackendJob(String),
    BeginFallback(String),
    AttachFallbackRequest {
        request_id: String,
        check_url: String,
    },
}

/// The writes that put a rebuilt in-flight row back where the deleted one was.
/// `upsert_in_flight` inserts a bare row, so without this replay Stop, Retry
/// then a crash resubmits a request Datalab already billed. Pure, so a unit
/// test reaches it without an app handle.
fn ledger_replay(backend: &BackendContext) -> Vec<LedgerReplay> {
    let mut steps = Vec::new();
    if let Some(job_id) = &backend.backend_job_id {
        steps.push(LedgerReplay::AttachBackendJob(job_id.clone()));
    }
    if let Some(fallback) = &backend.fallback {
        // The provider goes back even with no request id: an uncertain
        // fallback must stay uncertain.
        steps.push(LedgerReplay::BeginFallback(fallback.provider.clone()));
        if let (Some(request_id), Some(check_url)) = (&fallback.request_id, &fallback.check_url) {
            steps.push(LedgerReplay::AttachFallbackRequest {
                request_id: request_id.clone(),
                check_url: check_url.clone(),
            });
        }
    }
    steps
}

/// Spawn the full lifecycle for one job. If the manager has moved past
/// `generation`, the task exits without spending money.
pub fn run_job(app: AppHandle, id: u64, generation: u64) {
    tauri::async_runtime::spawn(async move {
        let stale = |app: &AppHandle| app.state::<JobManager>().generation() != generation;

        let job = match app.state::<JobManager>().get(id) {
            Some(j) => j,
            None => return,
        };
        if job.backend.is_some() {
            run_backend_job(app, id, generation, job).await;
            return;
        }
        let provider = job.job_type.provider();

        let api_key = match secrets::get_key(provider.key_name()) {
            Some(k) if !k.is_empty() => k,
            _ => {
                fail(
                    &app,
                    id,
                    generation,
                    &format!(
                        "No API key set for {}. Add it in Settings.",
                        provider.label()
                    ),
                );
                return;
            }
        };

        let sem = app.state::<JobManager>().semaphore();
        let _permit = sem.acquire_owned().await;
        // Waiting for a permit can take minutes, so the run may be long gone.
        if stale(&app) {
            return;
        }

        // Guarded: plain `update` drops the jobs lock before the emit, so a
        // Stop in that window ships a stale "queued" row and `running` sticks.
        let manager = app.state::<JobManager>();
        let Some(updated) = manager.update_if_generation(id, generation, |current| {
            current.started_at = Some(now_secs());
        }) else {
            return;
        };
        updated.emit(&app);

        // The run snapshot, so a mid-run Settings change splits no run.
        let cfg = app.state::<JobManager>().run_config();
        let client = provider_client();

        if !set_status(&app, id, generation, "working", "Uploading…") {
            return;
        }
        let datalab_format = cfg.datalab_format.clone();
        let pipeline = cfg
            .datalab_pipeline_id
            .clone()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let submitted = match job.job_type {
            JobType::Convert => {
                let res = providers::datalab_submit_any(
                    &client,
                    &api_key,
                    pipeline.as_deref(),
                    &job.source_path,
                    &datalab_format,
                    cfg.datalab_high_accuracy,
                )
                .await;
                match res {
                    Ok(s) => s,
                    Err(e) => {
                        if !stale(&app) {
                            fail(&app, id, generation, e.message());
                        }
                        return;
                    }
                }
            }
            JobType::Transcribe => {
                match providers::revai_submit(&client, &api_key, &job.source_path).await {
                    Ok(s) => s,
                    Err(e) => {
                        if !stale(&app) {
                            fail(&app, id, generation, &e);
                        }
                        return;
                    }
                }
            }
        };

        // A submit runs for up to half an hour, so Stop lands inside one. An
        // unguarded write here puts "processing" on a stopped row that no task
        // will ever finish, and `running` sticks on. Same for the two arms
        // above: a stopped job must not be relabelled with a network error.
        if stale(&app) {
            return;
        }
        if !set_status(&app, id, generation, "processing", "Processing…") {
            return;
        }
        let convert_check_url = submitted.datalab_check_url();

        let mut attempt: u32 = 0;
        // A blip is fine, an endless stream should fail rather than burn 60min.
        let mut consecutive_errors: u32 = 0;
        let text = loop {
            attempt += 1;
            if attempt > BACKEND_MAX_POLLS {
                fail(&app, id, generation, "Timed out waiting for the result.");
                return;
            }
            tokio::time::sleep(BACKEND_POLL_INTERVAL).await;
            if stale(&app) {
                return;
            }
            let poll = match job.job_type {
                JobType::Convert => {
                    providers::datalab_poll_any(
                        &client,
                        &api_key,
                        pipeline.is_some(),
                        &submitted.remote_id,
                        &convert_check_url,
                        &datalab_format,
                    )
                    .await
                }
                JobType::Transcribe => {
                    providers::revai_poll(&client, &api_key, &submitted.remote_id).await
                }
            };
            match poll {
                Ok(PollResult::Done(t)) => break t,
                Ok(PollResult::Failed(e)) => {
                    fail(&app, id, generation, &e);
                    return;
                }
                Ok(PollResult::Pending) => {
                    consecutive_errors = 0;
                    continue;
                }
                Err(e) => {
                    consecutive_errors += 1;
                    if consecutive_errors >= BACKEND_MAX_CONSECUTIVE_ERRORS {
                        fail(
                            &app,
                            id,
                            generation,
                            &format!("Lost contact while waiting for the result: {e}"),
                        );
                        return;
                    }
                    continue;
                }
            }
        };

        let ext = output_extension_for(job.job_type, &cfg);
        if stale(&app) {
            return;
        }
        match write_output(&job.output_dir, &job.file_name, ext, &text) {
            Ok(path) => finish(&app, id, generation, path),
            Err(e) => fail(&app, id, generation, &e),
        }
    });
}

/// `live` is the origin the resumed requests go to, not what the row records:
/// in Sidecar mode the ledger holds an alias, and an alias is not a URL.
fn validate_recovery_entry(
    entry: &history::InFlightEntry,
    live: &str,
) -> Result<settings::ConversionProfile, String> {
    conversion_service::validate_base_url(live)
        .map_err(|error| format!("Cannot recover conversion: {error}"))?;
    if let Some(job_id) = entry.backend_job_id.as_deref() {
        conversion_service::validate_uuid(job_id, "conversion id")
            .map_err(|error| format!("Cannot recover conversion: {error}"))?;
    }
    if !history::in_flight_source_is_current(entry) {
        return Err("Cannot recover conversion because the source changed or is missing".into());
    }
    if !Path::new(&entry.output_dir).is_dir() {
        return Err("Cannot recover conversion because its output folder is missing".into());
    }
    settings::ConversionProfile::from_id(&entry.conversion_profile)
        .ok_or_else(|| "Cannot recover conversion with an unknown profile".into())
}

/// The keychain holds one backend token, so it fits a recovered row only while
/// the configured origin still matches the row's. Both sides go through
/// `backend_host::ledger_origin`, so in Sidecar mode both are the alias.
fn recovery_origin_still_configured(entry: &history::InFlightEntry, configured: &str) -> bool {
    entry.backend_url.trim_end_matches('/') == configured.trim_end_matches('/')
}

/// Recreate durable backend rows after startup. Invalid origins and changed
/// sources stay visible and stopped, with their durable rows removed. In
/// Sidecar mode this runs on the first service that answers.
pub(crate) fn recover_in_flight(app: AppHandle) {
    let Some(entries) = history::list_in_flight(&app) else {
        return;
    };
    if entries.is_empty() {
        return;
    }
    // No origin, no recovery: deleting the rows throws away conversions the
    // service is still holding.
    let Ok(live_origin) = backend_host::backend_origin(&app) else {
        return;
    };

    let manager = app.state::<JobManager>();
    let config = settings::load(&app);
    let Ok(deployment) = backend_host::deployment(&app) else {
        return;
    };
    let configured_origin = backend_host::ledger_origin(&deployment).to_string();
    manager.set_run_config(config);
    let generation = manager.generation();
    for entry in entries {
        let validation = validate_recovery_entry(&entry, &live_origin).and_then(|profile| {
            if recovery_origin_still_configured(&entry, &configured_origin) {
                Ok(profile)
            } else {
                Err(
                    "Cannot recover conversion because its backend URL is no longer configured"
                        .into(),
                )
            }
        });
        let profile = validation
            .as_ref()
            .copied()
            .unwrap_or(settings::ConversionProfile::Standard);
        let recovery_error = validation.err();
        let durable_key = entry.idempotency_key.clone();
        let resumed_fallback = entry
            .fallback_provider
            .clone()
            .map(|provider| FallbackContext {
                provider,
                request_id: entry.fallback_request_id.clone(),
                check_url: entry.fallback_check_url.clone(),
            });
        let mut context = BackendContext::new(
            // The live origin, never the recorded alias `endpoint_url` refuses.
            live_origin.clone(),
            entry.client_run_id,
            entry.idempotency_key,
            entry.backend_job_id,
            profile,
            conversion_service::OcrOptions::from_wire(
                entry.ocr_language_correction,
                &entry.ocr_custom_words,
                entry.speaker_count,
            ),
        );
        if let Some(fallback) = resumed_fallback {
            context = context.resuming_fallback(fallback);
        }
        context.recovery_blocker.clone_from(&recovery_error);
        let id = manager.next_id();
        let job_type = JobType::from_id(&entry.job_type).unwrap_or(JobType::Convert);
        let mut job = Job::new_backend(id, entry.source_path, entry.output_dir, job_type, context);
        job.file_name = entry.file_name;
        manager.insert(job.clone());
        emit(&app, job);

        if let Some(error) = recovery_error {
            history::delete_in_flight(&app, &durable_key);
            fail_backend_retryable(&app, id, generation, &error);
        } else {
            run_job(app.clone(), id, generation);
        }
    }
}

#[cfg(test)]
mod import_tests {
    use super::*;

    #[test]
    fn an_import_carries_the_bytes_and_the_date() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("deck.pdf");
        let into = dir.path().join("Inbox");
        std::fs::create_dir(&into).unwrap();
        std::fs::write(&src, b"\x25PDF-1.7 not utf-8 \xff\xfe").unwrap();
        let when = std::fs::metadata(&src).unwrap().modified().unwrap();

        let landed = import_source(into.to_str().unwrap(), &src).unwrap();

        assert_eq!(landed, into.join("deck.pdf").to_string_lossy());
        assert_eq!(
            std::fs::read(&landed).unwrap(),
            std::fs::read(&src).unwrap()
        );
        // The tree refuses a result older than its source, history a source
        // whose mtime moved.
        assert_eq!(
            std::fs::metadata(&landed).unwrap().modified().unwrap(),
            when
        );
    }

    #[test]
    fn an_import_never_overwrites_what_is_already_there() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("deck.pdf");
        let into = dir.path().join("Inbox");
        std::fs::create_dir(&into).unwrap();
        std::fs::write(&src, b"new").unwrap();
        std::fs::write(into.join("deck.pdf"), b"do not lose me").unwrap();

        let landed = import_source(into.to_str().unwrap(), &src).unwrap();

        assert_eq!(landed, into.join("deck (1).pdf").to_string_lossy());
        assert_eq!(
            std::fs::read(into.join("deck.pdf")).unwrap(),
            b"do not lose me"
        );
    }

    #[test]
    fn importing_a_project_marker_cannot_break_the_folder_it_lands_in() {
        // `create_new` is the guard: an overwritten marker orphans the folder.
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("project.json");
        let into = dir.path().join("Inbox");
        std::fs::create_dir(&into).unwrap();
        std::fs::write(&src, b"{\"id\":\"impostor\"}").unwrap();
        std::fs::write(into.join("project.json"), b"{\"id\":\"real\"}").unwrap();

        import_source(into.to_str().unwrap(), &src).unwrap();

        assert_eq!(
            std::fs::read_to_string(into.join("project.json")).unwrap(),
            "{\"id\":\"real\"}"
        );
    }

    #[test]
    fn a_file_with_no_extension_keeps_its_bare_name() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("NOTES");
        let into = dir.path().join("Inbox");
        std::fs::create_dir(&into).unwrap();
        std::fs::write(&src, b"x").unwrap();

        let landed = import_source(into.to_str().unwrap(), &src).unwrap();

        // Not "NOTES.": the dot only appears when there is an extension.
        assert_eq!(landed, into.join("NOTES").to_string_lossy());
    }
}

#[cfg(test)]
mod backend_tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn ledger_entry(recorded_origin: &str) -> history::InFlightEntry {
        history::InFlightEntry {
            source_path: "/tmp/report.pdf".into(),
            file_name: "report.pdf".into(),
            output_dir: "/tmp".into(),
            backend_url: recorded_origin.into(),
            client_run_id: "11111111-1111-4111-8111-111111111111".into(),
            idempotency_key: "22222222-2222-4222-8222-222222222222".into(),
            backend_job_id: None,
            fallback_provider: None,
            fallback_request_id: None,
            fallback_check_url: None,
            conversion_profile: "standard".into(),
            job_type: "convert".into(),
            ocr_language_correction: true,
            ocr_custom_words: String::new(),
            speaker_count: None,
            source_mtime: 1,
            created_at: 1,
        }
    }

    fn context(job_id: Option<&str>) -> BackendContext {
        BackendContext::new(
            "http://127.0.0.1:8080".into(),
            "11111111-1111-4111-8111-111111111111".into(),
            "22222222-2222-4222-8222-222222222222".into(),
            job_id.map(str::to_owned),
            settings::ConversionProfile::Standard,
            conversion_service::OcrOptions::default(),
        )
    }

    fn view(status: &str) -> ConversionJob {
        ConversionJob {
            id: "11111111-1111-4111-8111-111111111111".into(),
            status: status.into(),
            route: None,
            warnings: Vec::new(),
            failure: None,
        }
    }

    #[test]
    fn local_only_failure_names_the_reason() {
        let mut needs_remote = view("needs_remote");
        needs_remote.route = Some(conversion_service::ConversionRoute {
            kind: "local_pdf".into(),
            reason_codes: vec!["mixed_pdf".into()],
        });
        assert_eq!(
            local_only_failure(&needs_remote),
            "Local-only conversion could not finish because some pages are scanned images. Choose Standard to allow Datalab fallback."
        );
    }

    #[test]
    fn only_known_backend_terminal_statuses_are_terminal() {
        assert_eq!(backend_action(&view("succeeded")), BackendAction::Succeeded);
        assert_eq!(
            backend_action(&view("needs_remote")),
            BackendAction::NeedsRemote
        );
        assert!(matches!(
            backend_action(&view("paused_by_future_backend")),
            BackendAction::Pending("Processing…")
        ));

        let mut failed = view("failed");
        failed.failure = Some(ConversionFailure {
            code: "bad_document".into(),
            message: "Document cannot be converted".into(),
        });
        assert_eq!(
            backend_action(&failed),
            BackendAction::Failed("bad_document: Document cannot be converted".into())
        );
    }

    /// A submit whose outcome is unknown leaves the provider recorded with no
    /// request id. Both the ledger and the in-memory context carry that, so a
    /// restart and a same-process Retry both refuse to resubmit.
    #[test]
    fn a_rebuilt_ledger_row_replays_the_remote_state_stop_deleted() {
        // An accepted fallback replays provider and request, so a restart
        // resumes polling rather than paying.
        assert_eq!(
            ledger_replay(&context(None).resuming_fallback(FallbackContext {
                provider: DATALAB_PROVIDER.to_owned(),
                request_id: Some("req-1".to_owned()),
                check_url: Some("https://example.test/1".to_owned()),
            })),
            vec![
                LedgerReplay::BeginFallback(DATALAB_PROVIDER.to_owned()),
                LedgerReplay::AttachFallbackRequest {
                    request_id: "req-1".to_owned(),
                    check_url: "https://example.test/1".to_owned(),
                },
            ],
        );

        // An uncertain fallback replays the provider alone, or it reads as
        // never started.
        assert_eq!(
            ledger_replay(&context(None).resuming_fallback(FallbackContext {
                provider: DATALAB_PROVIDER.to_owned(),
                request_id: None,
                check_url: None,
            })),
            vec![LedgerReplay::BeginFallback(DATALAB_PROVIDER.to_owned())],
        );

        // No fallback, but a known backend job still has to come back.
        assert_eq!(
            ledger_replay(&context(Some("job-9"))),
            vec![LedgerReplay::AttachBackendJob("job-9".to_owned())],
        );

        // A plain submit has nothing to replay beyond the base row.
        assert!(ledger_replay(&context(None)).is_empty());
    }

    /// The ledger alone leaves Retry able to pay again with no restart.
    #[test]
    fn an_uncertain_fallback_is_visible_to_both_restart_and_retry() {
        let uncertain = FallbackContext {
            provider: DATALAB_PROVIDER.to_owned(),
            request_id: None,
            check_url: None,
        };
        let accepted = FallbackContext {
            provider: DATALAB_PROVIDER.to_owned(),
            request_id: Some("req-1".to_owned()),
            check_url: Some("https://example.test/1".to_owned()),
        };

        let resumed = context(None).resuming_fallback(uncertain.clone());
        assert_eq!(resumed.phase, BackendPhase::DatalabFallback);
        assert_eq!(resumed.fallback, Some(uncertain));

        // The recorded provider decides which endpoint a resume polls.
        let pipeline_resume = context(None).resuming_fallback(FallbackContext {
            provider: DATALAB_PIPELINE_PROVIDER.to_owned(),
            ..accepted
        });
        assert_eq!(
            pipeline_resume.fallback.unwrap().provider,
            DATALAB_PIPELINE_PROVIDER
        );
    }

    #[test]
    fn retirement_waits_for_guarded_commit_and_then_rejects_stale_updates() {
        let manager = Arc::new(JobManager::default());
        manager.insert(Job::new(
            1,
            "/tmp/report.pdf".into(),
            "/tmp".into(),
            JobType::Convert,
        ));
        let task_generation = manager.generation();
        let (guarded_tx, guarded_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (event_tx, event_rx) = std::sync::mpsc::channel();

        let worker_manager = manager.clone();
        let worker_events = event_tx.clone();
        let worker = std::thread::spawn(move || {
            let updated = worker_manager
                .update_if_generation(1, task_generation, |job| {
                    job.status = "working".into();
                })
                .unwrap();
            guarded_tx.send(()).unwrap();
            updated.commit(|_| {
                release_rx.recv().unwrap();
                worker_events.send("emitted").unwrap();
            });
        });
        guarded_rx.recv().unwrap();

        let retire_manager = manager.clone();
        let retire_events = event_tx.clone();
        let (retire_started_tx, retire_started_rx) = std::sync::mpsc::channel();
        let retire = std::thread::spawn(move || {
            retire_started_tx.send(()).unwrap();
            let next_generation = retire_manager.new_generation();
            retire_events.send("retired").unwrap();
            next_generation
        });
        retire_started_rx.recv().unwrap();
        assert!(event_rx.recv_timeout(Duration::from_millis(50)).is_err());

        release_tx.send(()).unwrap();
        assert_eq!(event_rx.recv().unwrap(), "emitted");
        assert_eq!(event_rx.recv().unwrap(), "retired");
        worker.join().unwrap();
        assert_eq!(retire.join().unwrap(), task_generation + 1);

        manager.update(1, |job| {
            job.status = "failed".into();
            job.error = Some("Stopped".into());
        });

        assert!(manager
            .update_if_generation(1, task_generation, |job| {
                job.status = "done".into();
                job.error = None;
            })
            .is_none());
        let stopped = manager.get(1).unwrap();
        assert_eq!(stopped.status, "failed");
        assert_eq!(stopped.error.as_deref(), Some("Stopped"));
    }

    #[test]
    fn recovery_context_keeps_original_url_and_stable_keys_off_ipc() {
        let context = BackendContext::new(
            "http://127.0.0.1:9123".into(),
            "22222222-2222-4222-8222-222222222222".into(),
            "33333333-3333-4333-8333-333333333333".into(),
            Some("44444444-4444-4444-8444-444444444444".into()),
            settings::ConversionProfile::Standard,
            conversion_service::OcrOptions::default(),
        );
        let job = Job::new_backend(
            1,
            "/tmp/report.pdf".into(),
            "/tmp".into(),
            JobType::Convert,
            context.clone(),
        );

        assert_eq!(
            job.backend.as_ref().unwrap().backend_url,
            "http://127.0.0.1:9123"
        );
        assert_eq!(
            job.backend.as_ref().unwrap().idempotency_key,
            context.idempotency_key
        );
        let serialized = serde_json::to_value(&job).unwrap();
        assert!(serialized.get("backend").is_none());
        assert!(serialized.get("outputText").is_none());
    }

    /// The service writes `.md` for a transcript too, so pairing on `.txt`
    /// alone would leave every backend transcript looking unconverted.
    #[test]
    fn a_backend_transcript_pairs_on_markdown_and_on_the_direct_text() {
        let mut cfg = settings::Settings::default();

        assert_eq!(
            result_extensions_for(JobType::Transcribe, &cfg),
            ["md", "txt"]
        );
        assert_eq!(output_extension_for(JobType::Transcribe, &cfg), "txt");

        // A route flip must not unpair a transcript already on disk.
        cfg.conversion_route = settings::ConversionRoute::Direct;
        assert_eq!(
            result_extensions_for(JobType::Transcribe, &cfg),
            ["md", "txt"]
        );
    }

    #[test]
    fn a_backend_job_is_labelled_by_the_job_it_runs() {
        let job = |jt| {
            Job::new_backend(
                1,
                "/tmp/interview.m4a".into(),
                "/tmp".into(),
                jt,
                context(None),
            )
        };

        assert_eq!(job(JobType::Convert).service, "Conversion service");
        assert_eq!(job(JobType::Transcribe).service, "Local transcription");
        assert_eq!(job(JobType::Transcribe).job_type, JobType::Transcribe);
    }

    /// The OCR options are in the replay fingerprint, so reading Settings 409s.
    #[test]
    fn a_recovered_context_carries_the_recorded_ocr_options() {
        let recorded =
            conversion_service::OcrOptions::from_wire(false, "Uniwise\nDatalab", Some(2));
        let context = BackendContext::new(
            "http://127.0.0.1:9123".into(),
            "22222222-2222-4222-8222-222222222222".into(),
            "33333333-3333-4333-8333-333333333333".into(),
            None,
            settings::ConversionProfile::Standard,
            recorded,
        );

        // Settings are back at the defaults, and the context must not follow.
        let now = settings::Settings::default();
        assert!(now.language_correction && now.custom_words.is_empty());
        assert!(!context.ocr.language_correction);
        assert_eq!(context.ocr.custom_words, ["Uniwise", "Datalab"]);
        assert_eq!(context.ocr.custom_words_wire(), "Uniwise\nDatalab");
    }

    #[test]
    fn invalid_recovery_rows_are_rejected_before_resume_and_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let source = root.join("report.pdf");
        std::fs::write(&source, b"pdf").unwrap();
        let source = std::fs::canonicalize(source).unwrap();
        let source_mtime = std::fs::metadata(&source)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let mut entry = ledger_entry("http://127.0.0.1:8080");
        entry.source_path = source.to_string_lossy().into_owned();
        entry.output_dir = root.to_string_lossy().into_owned();
        entry.backend_job_id = Some("33333333-3333-4333-8333-333333333333".into());
        entry.source_mtime = source_mtime;
        let live = "http://127.0.0.1:8080";
        assert!(validate_recovery_entry(&entry, live).is_ok());
        // One slot, one token: a replaced origin must not get the new one.
        assert!(recovery_origin_still_configured(
            &entry,
            "http://127.0.0.1:8080/"
        ));
        assert!(!recovery_origin_still_configured(
            &entry,
            "http://other.host:8080"
        ));

        entry.backend_job_id = Some("not-a-uuid".into());
        assert!(validate_recovery_entry(&entry, live)
            .unwrap_err()
            .contains("conversion id"));
        entry.backend_job_id = None;
        entry.conversion_profile = "future_profile".into();
        assert!(validate_recovery_entry(&entry, live)
            .unwrap_err()
            .contains("unknown profile"));
        entry.conversion_profile = "standard".into();
        std::fs::remove_file(&source).unwrap();
        assert!(validate_recovery_entry(&entry, live)
            .unwrap_err()
            .contains("source changed or is missing"));
    }

    /// A different port every launch, so comparing live URLs abandons every row.
    #[test]
    fn a_sidecar_row_survives_the_port_the_next_launch_is_given() {
        // A property of the type now. An arm reaching for a live URL fails here.
        let recorded = backend_host::ledger_origin(&backend_host::Deployment::Sidecar);
        assert_eq!(recorded, backend_host::SIDECAR_ALIAS);
        let entry = ledger_entry(recorded);

        let now = backend_host::ledger_origin(&backend_host::Deployment::Sidecar);

        assert!(recovery_origin_still_configured(&entry, now));
    }

    /// Manual mode names the service, so a row bound to one the user moved away
    /// from must not be handed the current token.
    #[test]
    fn a_manual_row_still_refuses_a_service_the_user_replaced() {
        let manual = |url: &str| backend_host::Deployment::Manual {
            origin: url.to_string(),
        };
        let entry = ledger_entry(backend_host::ledger_origin(&manual(
            "http://127.0.0.1:8080",
        )));

        assert!(recovery_origin_still_configured(
            &entry,
            backend_host::ledger_origin(&manual("http://127.0.0.1:8080/"))
        ));
        assert!(!recovery_origin_still_configured(
            &entry,
            backend_host::ledger_origin(&manual("http://other.host:8080"))
        ));
    }

    /// The alias names the service without locating it, so recovery validates
    /// the origin the resumed requests go to.
    #[test]
    fn recovery_validates_the_live_origin_and_never_the_alias() {
        let entry = ledger_entry(backend_host::SIDECAR_ALIAS);

        assert!(validate_recovery_entry(&entry, backend_host::SIDECAR_ALIAS)
            .unwrap_err()
            .contains("Backend URL"));
        // Fails later, on the missing source, which proves the origin passed.
        assert!(validate_recovery_entry(&entry, "http://127.0.0.1:64707")
            .unwrap_err()
            .contains("source changed or is missing"));
    }

    #[test]
    fn a_request_follows_the_service_to_the_port_it_restarted_on() {
        // A job queued against the old port has to follow the restart.
        assert_eq!(
            request_origin(
                Ok("http://127.0.0.1:64707".into()),
                "http://127.0.0.1:51001"
            ),
            "http://127.0.0.1:64707"
        );

        // Mid-restart there is no port, so the queued origin stands.
        assert_eq!(
            request_origin(
                Err("The conversion service is starting.".into()),
                "http://127.0.0.1:51001"
            ),
            "http://127.0.0.1:51001"
        );
    }

    #[test]
    fn a_retry_drops_the_verdict_the_last_launch_left_and_follows_the_new_port() {
        let mut context = BackendContext::new(
            "http://127.0.0.1:51001".into(),
            "11111111-1111-4111-8111-111111111111".into(),
            "22222222-2222-4222-8222-222222222222".into(),
            None,
            settings::ConversionProfile::Standard,
            conversion_service::OcrOptions::default(),
        );
        context.recovery_blocker = Some("Cannot recover conversion".into());

        refresh_for_retry(&mut context, Ok("http://127.0.0.1:64707"));

        // Nothing else clears this, so without it Retry re-fails all session.
        assert_eq!(context.recovery_blocker, None);
        assert_eq!(context.backend_url, "http://127.0.0.1:64707");

        // A service that is not up replaces the verdict rather than retrying.
        refresh_for_retry(
            &mut context,
            Err("The conversion service is starting.".into()),
        );
        assert_eq!(
            context.recovery_blocker.as_deref(),
            Some("The conversion service is starting.")
        );
        assert_eq!(context.backend_url, "http://127.0.0.1:64707");
    }
}
