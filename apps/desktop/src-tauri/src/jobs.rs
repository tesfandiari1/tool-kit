//! In-memory job queue. A run clears the queue, then submits one job per
//! matching file and polls it on a background task, capped at 4 in flight.

use serde::Serialize;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Semaphore;

use crate::conversion_service::{self, ConversionFailure, ConversionJob};
use crate::{backend_host, history, settings};

/// How patient the first poll of a recovered conversion is: 12s covers a
/// sidecar still opening its database.
const RESUME_MAX_ATTEMPTS: u32 = 6;
const RESUME_RETRY_INTERVAL: Duration = Duration::from_secs(2);

/// Every 5s for ~60 minutes, bailing after ~1 minute of unbroken errors.
const BACKEND_POLL_INTERVAL: Duration = Duration::from_secs(5);
const BACKEND_MAX_POLLS: u32 = 720;
const BACKEND_MAX_CONSECUTIVE_ERRORS: u32 = 12;

#[derive(Clone, Debug)]
pub(crate) struct BackendContext {
    pub(crate) client_run_id: String,
    pub(crate) idempotency_key: String,
    pub(crate) backend_job_id: Option<String>,
    pub(crate) profile: settings::ConversionProfile,
    /// The OCR options this submission belongs to. Reading Settings at submit
    /// time resubmits a stored key under a changed fingerprint, which 409s.
    pub(crate) ocr: conversion_service::OcrOptions,
    recovery_blocker: Option<String>,
}

impl BackendContext {
    pub(crate) fn new(
        client_run_id: String,
        idempotency_key: String,
        backend_job_id: Option<String>,
        profile: settings::ConversionProfile,
        ocr: conversion_service::OcrOptions,
    ) -> Self {
        Self {
            client_run_id,
            idempotency_key,
            backend_job_id,
            profile,
            ocr,
            recovery_blocker: None,
        }
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

    /// Only what the bundled service converts locally: `SOURCE_FORMATS` in
    /// apps/converter/src/conversion/model.rs, row for row.
    pub fn accepts(&self, ext: &str) -> bool {
        match self {
            JobType::Convert => matches!(
                ext,
                "pdf"
                    | "docx"
                    | "doc"
                    | "pptx"
                    | "ppt"
                    | "xlsx"
                    | "xls"
                    | "epub"
                    | "odt"
                    | "ods"
                    | "odp"
                    | "rtf"
                    | "csv"
                    | "docm"
                    | "xlsm"
                    | "pptm"
                    | "ppsx"
                    | "ppsm"
                    | "pps"
                    | "pot"
                    | "png"
                    | "jpg"
                    | "jpeg"
                    | "webp"
                    | "tiff"
                    | "tif"
                    | "gif"
                    | "bmp"
            ),
            JobType::Transcribe => matches!(ext, "mp3" | "mp4" | "wav" | "m4a" | "flac" | "mov"),
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

    /// Neither `done` nor `failed`: a run that holds one is joined, a quit confirms.
    pub fn is_active(&self) -> bool {
        matches!(self.status.as_str(), "queued" | "working" | "processing")
    }

    pub(crate) fn new_backend(
        id: u64,
        source_path: String,
        output_dir: String,
        job_type: JobType,
        backend: BackendContext,
    ) -> Self {
        let mut job = Self::new(id, source_path, output_dir, job_type);
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
    /// aborts once it stops matching, so a retired run stops.
    generation: AtomicU64,
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
}

/// What "the same output" means when deciding whether a file is already done.
/// Frozen once rows exist: `markdown` is what Convert filed before the service.
pub fn output_format_for(jt: JobType) -> &'static str {
    match jt {
        JobType::Transcribe => "text",
        JobType::Convert => "markdown",
    }
}

/// Every extension a finished result may carry, which is what the tree's
/// pairing rule has to ask. The service writes Markdown for both jobs, and an
/// older build's `.txt` transcript stays paired so it never runs again.
pub fn result_extensions_for(jt: JobType) -> &'static [&'static str] {
    match jt {
        JobType::Transcribe => &["md", "txt"],
        JobType::Convert => &["md"],
    }
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
    let output_format = match job.backend {
        Some(_) => "backend:markdown",
        None => output_format_for(job.job_type),
    };
    history::record(
        app,
        &history::Finished {
            file_name: &job.file_name,
            source_path: &job.source_path,
            output_path: job.output_path.as_deref(),
            job_type: job.job_type.id(),
            output_format,
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

/// Satisfy a job from a result an earlier run produced, rather than converting
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
        "" => return "This file could not be converted on this Mac.".into(),
        "mixed_pdf" => "some pages are scanned images",
        "image_based_pdf" | "scanned_pdf" => "every page is a scanned image",
        "ocr_required" => "the text layer is missing or unreadable",
        "garbled_text" => "the text layer is garbled",
        "local_quality_failed" => "no readable text was found",
        "output_too_large" => "the result was too large",
        other => other,
    };
    format!("This file could not be converted on this Mac because {reason}.")
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
            // Read per request: the sidecar gets a new port at every restart.
            let result = match backend_host::backend_origin(&app) {
                Ok(origin) => {
                    conversion_service::poll_conversion(&origin, &token, &backend_job_id).await
                }
                Err(error) => Err(error),
            };
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
        let result = match backend_host::backend_origin(&app) {
            Ok(origin) => {
                conversion_service::submit_conversion(
                    &origin,
                    &token,
                    &job.source_path,
                    &backend.client_run_id,
                    backend.profile.id(),
                    &backend.ocr,
                    &backend.idempotency_key,
                )
                .await
            }
            Err(error) => Err(error),
        };
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
                let output = match backend_host::backend_origin(&app) {
                    Ok(origin) => {
                        conversion_service::download_markdown(
                            &origin,
                            &token,
                            &view.id,
                            &job.output_dir,
                            &job.file_name,
                        )
                        .await
                    }
                    Err(error) => Err(error),
                };
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
            // Nothing leaves the Mac, so the service's refusal is the answer.
            BackendAction::NeedsRemote => {
                let error = match job.job_type {
                    // The service never sends this for audio.
                    JobType::Transcribe => {
                        "This recording could not be transcribed on this Mac.".to_string()
                    }
                    JobType::Convert => local_only_failure(&view),
                };
                fail(&app, id, generation, &error);
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
        let result = match backend_host::backend_origin(&app) {
            Ok(origin) => conversion_service::poll_conversion(&origin, &token, &view.id).await,
            Err(error) => Err(error),
        };
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

/// Clear the recovery blocker the last launch left once the service answers.
/// Nothing else clears it, so Retry re-fails all session.
fn refresh_for_retry(context: &mut BackendContext, origin: Result<&str, String>) {
    context.recovery_blocker = origin.err();
}

/// Re-persist a backend job's durable row before a retry. Stop deletes the
/// ledger row while the job stays retryable, so without this a crash during the
/// resubmit loses the job.
pub fn restore_in_flight(app: &AppHandle, id: u64) {
    let manager = app.state::<JobManager>();
    // Retry-all runs this over every failed row, so copies leave here.
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
    // Stop deleted the row, so put back the backend job it had reached.
    if let Some(job_id) = &backend.backend_job_id {
        history::attach_backend_job(app, &backend.idempotency_key, job_id);
    }
}

/// Spawn the full lifecycle for one job. If the manager has moved past
/// `generation`, the task exits without touching the row.
pub fn run_job(app: AppHandle, id: u64, generation: u64) {
    tauri::async_runtime::spawn(async move {
        let Some(job) = app.state::<JobManager>().get(id) else {
            return;
        };
        // Only a copy of an earlier result has no backend, and Retry cannot
        // redo a copy whose source it never kept.
        if job.backend.is_none() {
            fail(
                &app,
                id,
                generation,
                "Copying the earlier result failed. Run this file again.",
            );
            return;
        }
        run_backend_job(app, id, generation, job).await;
    });
}

const REMOTE_REMOVED: &str =
    "Remote conversion was removed. Run this file again to convert it locally.";

/// `live` is the origin the resumed requests go to, not what the row records:
/// in Sidecar mode the ledger holds an alias, and an alias is not a URL.
fn validate_recovery_entry(
    entry: &history::InFlightEntry,
    live: &str,
) -> Result<settings::ConversionProfile, String> {
    // An older build sent this file out. That route is gone, and polling it
    // would be the request this build exists not to make.
    if entry.fallback_provider.is_some() {
        return Err(REMOTE_REMOVED.into());
    }
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
    let Ok(deployment) = backend_host::deployment(&app) else {
        return;
    };
    let configured_origin = backend_host::ledger_origin(&deployment).to_string();
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
            .unwrap_or(settings::ConversionProfile::LocalOnly);
        let recovery_error = validation.err();
        let durable_key = entry.idempotency_key.clone();
        let mut context = BackendContext::new(
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
        context.recovery_blocker.clone_from(&recovery_error);
        let id = manager.next_id();
        let job_type = JobType::from_id(&entry.job_type).unwrap_or(JobType::Convert);
        let mut job = Job::new_backend(id, entry.source_path, entry.output_dir, job_type, context);
        job.file_name = entry.file_name;
        manager.insert(job.clone());
        emit(&app, job);

        if let Some(error) = recovery_error {
            history::delete_in_flight(&app, &durable_key);
            // Retry would replay the job that needed the removed route.
            if error == REMOTE_REMOVED {
                fail(&app, id, generation, &error);
            } else {
                fail_backend_retryable(&app, id, generation, &error);
            }
        } else {
            run_job(app.clone(), id, generation);
        }
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
            conversion_profile: "standard".into(),
            job_type: "convert".into(),
            ocr_language_correction: true,
            ocr_custom_words: String::new(),
            speaker_count: None,
            source_mtime: 1,
            created_at: 1,
        }
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
            "This file could not be converted on this Mac because some pages are scanned images."
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
    fn recovery_context_keeps_stable_keys_off_ipc() {
        let context = BackendContext::new(
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
            job.backend.as_ref().unwrap().idempotency_key,
            context.idempotency_key
        );
        let serialized = serde_json::to_value(&job).unwrap();
        assert!(serialized.get("backend").is_none());
        assert!(serialized.get("outputText").is_none());
    }

    /// The service writes `.md`, and an older build's `.txt` transcript must
    /// stay paired or its recording reads as unconverted.
    #[test]
    fn a_transcript_pairs_on_markdown_and_on_the_older_text() {
        assert_eq!(result_extensions_for(JobType::Transcribe), ["md", "txt"]);
    }

    /// `accepts` copies the converter's table by hand. A row added there and
    /// missed here converts on Run while the tree calls the file unconvertible.
    #[test]
    fn every_converter_format_belongs_to_exactly_one_job() {
        let table = include_str!("../../../converter/src/conversion/model.rs");
        let extensions: Vec<&str> = table
            .lines()
            .filter_map(|line| line.trim().strip_prefix("extension: \""))
            .filter_map(|rest| rest.split('"').next())
            .collect();
        assert!(extensions.len() >= 34, "{extensions:?}");
        for ext in extensions {
            assert!(
                JobType::Convert.accepts(ext) != JobType::Transcribe.accepts(ext),
                "{ext}"
            );
        }
    }

    /// The OCR options are in the replay fingerprint, so reading Settings 409s.
    #[test]
    fn a_recovered_context_carries_the_recorded_ocr_options() {
        let recorded =
            conversion_service::OcrOptions::from_wire(false, "Uniwise\nTool-Kit", Some(2));
        let context = BackendContext::new(
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
        assert_eq!(context.ocr.custom_words, ["Uniwise", "Tool-Kit"]);
        assert_eq!(context.ocr.custom_words_wire(), "Uniwise\nTool-Kit");
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
        // A row an older build sent out fails before anything is asked.
        entry.fallback_provider = Some("datalab".into());
        assert_eq!(
            validate_recovery_entry(&entry, live),
            Err(REMOTE_REMOVED.into())
        );
        entry.fallback_provider = None;
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
    fn a_retry_drops_the_verdict_the_last_launch_left() {
        let mut context = BackendContext::new(
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

        // A service that is not up replaces the verdict rather than retrying.
        refresh_for_retry(
            &mut context,
            Err("The conversion service is starting.".into()),
        );
        assert_eq!(
            context.recovery_blocker.as_deref(),
            Some("The conversion service is starting.")
        );
    }
}
