//! In-memory job queue. A "run" clears the queue, then one job per matching
//! file is submitted to its provider and polled on a background task, with a
//! concurrency cap so we don't hammer the upstream APIs.

use serde::Serialize;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Semaphore;

use crate::conversion_service::{self, ConversionFailure, ConversionJob};
use crate::providers::{self, PollResult, ProviderKind};
use crate::{history, secrets, settings};

const BACKEND_POLL_INTERVAL: Duration = Duration::from_secs(5);
const BACKEND_MAX_POLLS: u32 = 720;
const BACKEND_MAX_CONSECUTIVE_ERRORS: u32 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BackendPhase {
    SubmitOrPoll,
    DatalabFallback,
}

#[derive(Clone, Debug)]
pub(crate) struct BackendContext {
    pub(crate) backend_url: String,
    pub(crate) client_run_id: String,
    pub(crate) idempotency_key: String,
    pub(crate) backend_job_id: Option<String>,
    pub(crate) profile: settings::ConversionProfile,
    phase: BackendPhase,
    recovery_blocker: Option<String>,
}

impl BackendContext {
    pub(crate) fn new(
        backend_url: String,
        client_run_id: String,
        idempotency_key: String,
        backend_job_id: Option<String>,
        profile: settings::ConversionProfile,
    ) -> Self {
        Self {
            backend_url,
            client_run_id,
            idempotency_key,
            backend_job_id,
            profile,
            phase: BackendPhase::SubmitOrPoll,
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

    pub fn label(&self) -> &'static str {
        match self {
            JobType::Convert => "Convert",
            JobType::Transcribe => "Transcribe",
        }
    }

    /// The stable string this job is filed under in the history database.
    /// Matches `from_id`, and must not change once rows exist.
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

    /// Does this job accept a file with the given lowercase extension?
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
    pub output_text: Option<String>,
    pub error: Option<String>,
    /// Backend-provided metadata is intentionally stringly typed: new route,
    /// warning, failure, and status values must not break an older desktop.
    pub route: Option<String>,
    pub reason_codes: Vec<String>,
    pub warnings: Vec<String>,
    pub failure: Option<ConversionFailure>,
    pub created_at: u64,
    /// When this job actually left the queue. The elapsed timer counts from
    /// here so a file waiting behind the concurrency cap doesn't appear to
    /// have been processing for the whole wait.
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
            output_text: None,
            error: None,
            route: None,
            reason_codes: Vec::new(),
            warnings: Vec::new(),
            failure: None,
            created_at: now_secs(),
            started_at: None,
            backend: None,
        }
    }

    pub(crate) fn new_backend(
        id: u64,
        source_path: String,
        output_dir: String,
        backend: BackendContext,
    ) -> Self {
        let mut job = Self::new(id, source_path, output_dir, JobType::Convert);
        job.service = "Conversion service".into();
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
    /// Bumped by every new run and by Stop. A spawned task captures the value
    /// at spawn time and aborts as soon as it no longer matches, so starting a
    /// second run (or hitting Stop) reliably retires the previous run's tasks
    /// instead of leaving them to spend credits and write files invisibly.
    generation: AtomicU64,
    /// Settings snapshot taken when a run starts, so changing Settings mid-run
    /// can't split one run across two output formats or two models.
    run_config: Mutex<settings::Settings>,
}

/// Keeps the jobs lock through the caller's durable side effects and event
/// emission, so retirement cannot split a guarded mutation from its signal.
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
///
/// The pipeline id is folded in on purpose: pinning a `pl_…` pipeline changes
/// what Convert produces, so a plain-convert result is not a substitute for it
/// and the file must run again.
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

fn emit(app: &AppHandle, job: Job) {
    let _ = app.emit("job-updated", job);
}

/// File a terminal job in the history. Best-effort by design: `history::record`
/// swallows storage errors so a finished conversion is still a success.
///
/// User-initiated stops are skipped — `stop_run` marks cancelled jobs with the
/// error "Stopped", and a cancelled 200-file run would otherwise bury the log.
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

fn set_status(app: &AppHandle, id: u64, status: &str, note: &str) {
    if let Some(job) = app.state::<JobManager>().update(id, |j| {
        j.status = status.to_string();
        j.progress_note = note.to_string();
    }) {
        emit(app, job);
    }
}

fn set_backend_status(app: &AppHandle, id: u64, generation: u64, status: &str, note: &str) -> bool {
    let manager = app.state::<JobManager>();
    let Some(updated) = manager.update_if_generation(id, generation, |job| {
        job.status = status.to_string();
        job.progress_note = note.to_string();
    }) else {
        return false;
    };
    updated.emit(app);
    true
}

fn fail(app: &AppHandle, id: u64, err: &str) {
    if let Some(job) = app.state::<JobManager>().update(id, |j| {
        j.status = "failed".to_string();
        j.progress_note = String::new();
        j.error = Some(err.to_string());
    }) {
        log_history(app, &job, "failed", Some(err));
        emit(app, job);
    }
}

fn finish(app: &AppHandle, id: u64, text: String, output_path: Option<String>) {
    if let Some(job) = app.state::<JobManager>().update(id, |j| {
        j.status = "done".to_string();
        j.progress_note = String::new();
        j.output_text = Some(text);
        j.output_path = output_path;
        j.error = None;
    }) {
        log_history(app, &job, "done", None);
        emit(app, job);
    }
}

fn apply_backend_view(app: &AppHandle, id: u64, generation: u64, view: &ConversionJob) -> bool {
    let manager = app.state::<JobManager>();
    let Some(updated) = manager.update_if_generation(id, generation, |job| {
        job.route = view.route.as_ref().map(|route| route.kind.clone());
        job.reason_codes = view
            .route
            .as_ref()
            .map(|route| route.reason_codes.clone())
            .unwrap_or_default();
        job.warnings = view.warnings.clone();
        job.failure = view.failure.clone();
    }) else {
        return false;
    };
    updated.emit(app);
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

fn fail_backend_terminal(app: &AppHandle, id: u64, generation: u64, err: &str) {
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

fn finish_backend(app: &AppHandle, id: u64, generation: u64, output_path: String) {
    let cleanup_path = output_path.clone();
    if let Some(updated) = app
        .state::<JobManager>()
        .update_if_generation(id, generation, |job| {
            job.status = "done".into();
            job.progress_note.clear();
            job.output_text = None;
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

/// A backend download or fallback write creates its collision-safe file before
/// control returns to the job task. If Stop retired the task meanwhile, remove
/// exactly that newly-created path and leave the stopped row untouched.
fn completed_output_if_current(
    current_generation: u64,
    expected_generation: u64,
    output: Result<String, String>,
) -> Option<Result<String, String>> {
    if current_generation == expected_generation {
        return Some(output);
    }
    if let Ok(path) = output {
        let _ = std::fs::remove_file(path);
    }
    None
}

/// Write `text` into the job's output folder without clobbering an existing
/// file. Uses `create_new` so two concurrent jobs whose sources share a
/// basename can't both win the same candidate name and overwrite each other.
fn write_output(
    output_dir: &str,
    file_name: &str,
    ext: &str,
    suffix: &str,
    text: &str,
) -> Result<String, String> {
    use std::io::Write;

    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let base = std::path::PathBuf::from(output_dir);

    for n in 0..1000 {
        let candidate = if n == 0 {
            base.join(format!("{stem}{suffix}.{ext}"))
        } else {
            base.join(format!("{stem}{suffix} ({n}).{ext}"))
        };
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(mut f) => {
                return f
                    .write_all(text.as_bytes())
                    .and_then(|_| f.sync_all())
                    .map(|_| candidate.to_string_lossy().to_string())
                    .map_err(|e| format!("Could not write {}: {e}", candidate.display()));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Could not write to the output folder: {e}")),
        }
    }
    Err(format!(
        "Could not find a free filename for {stem}{suffix}.{ext} in the output folder."
    ))
}

/// Satisfy a job from a result an earlier run already produced, instead of
/// paying the provider for it again.
///
/// Reached when the history has a still-valid result for this exact file, job
/// and format, but it lives in a different folder from the one now selected.
/// The source is unchanged and the format matches, so the bytes a re-run would
/// produce are the bytes we already have — copying is the same outcome for
/// free. Goes through `write_output`, so it inherits the no-clobber rule.
///
/// Returns whether the copy succeeded; a failure is reported on the job like
/// any other, and Retry then runs it for real.
pub fn reuse_result(app: &AppHandle, id: u64, existing: &str) -> bool {
    let Some(job) = app.state::<JobManager>().get(id) else {
        return false;
    };
    let text = match std::fs::read_to_string(existing) {
        Ok(t) => t,
        Err(e) => {
            fail(app, id, &format!("Could not read the earlier result: {e}"));
            return false;
        }
    };
    // Take the extension from the file we are copying, so a result written
    // under an older format setting keeps its own suffix.
    let ext = Path::new(existing)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("txt");
    match write_output(&job.output_dir, &job.file_name, ext, "", &text) {
        Ok(path) => {
            finish(app, id, text, Some(path));
            true
        }
        Err(e) => {
            fail(app, id, &e);
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

#[derive(Debug, PartialEq, Eq)]
enum NeedsRemoteDecision {
    DatalabFallback,
    RejectLocalOnly,
}

fn needs_remote_decision(profile: settings::ConversionProfile) -> NeedsRemoteDecision {
    match profile {
        settings::ConversionProfile::Standard => NeedsRemoteDecision::DatalabFallback,
        settings::ConversionProfile::LocalOnly => NeedsRemoteDecision::RejectLocalOnly,
    }
}

/// Terminal detection is deliberately an allowlist. A status added by a newer
/// backend remains pending in this desktop instead of being misclassified.
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

async fn run_backend_job(app: AppHandle, id: u64, generation: u64, job: Job) {
    let stale = |app: &AppHandle| app.state::<JobManager>().generation() != generation;
    let Some(mut backend) = job.backend.clone() else {
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

    let token = match secrets::get_key("backend") {
        Some(token) if !token.trim().is_empty() => token,
        _ => {
            fail_backend_retryable(
                &app,
                id,
                generation,
                "Backend token is unavailable. Add it in Settings, then retry.",
            );
            return;
        }
    };

    let mut view = if let Some(backend_job_id) = backend.backend_job_id.clone() {
        if !set_backend_status(&app, id, generation, "processing", "Resuming conversion…") {
            return;
        }
        let result =
            conversion_service::poll_conversion(&backend.backend_url, &token, &backend_job_id)
                .await;
        if stale(&app) {
            return;
        }
        match result {
            Ok(view) => view,
            Err(error) => {
                fail_backend_retryable(&app, id, generation, &error);
                return;
            }
        }
    } else {
        if !set_backend_status(
            &app,
            id,
            generation,
            "working",
            "Uploading to conversion service…",
        ) {
            return;
        }
        let result = conversion_service::submit_conversion(
            &backend.backend_url,
            &token,
            &job.source_path,
            &backend.client_run_id,
            backend.profile.id(),
            &backend.idempotency_key,
        )
        .await;
        if stale(&app) {
            return;
        }
        match result {
            Ok(view) => {
                backend.backend_job_id = Some(view.id.clone());
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
                if !set_backend_status(&app, id, generation, "processing", "Saving Markdown…") {
                    return;
                }
                let output = conversion_service::download_markdown(
                    &backend.backend_url,
                    &token,
                    &view.id,
                    &job.output_dir,
                    &job.file_name,
                )
                .await;
                let Some(output) = completed_output_if_current(
                    app.state::<JobManager>().generation(),
                    generation,
                    output,
                ) else {
                    return;
                };
                match output {
                    Ok(path) => finish_backend(&app, id, generation, path),
                    Err(error) => fail_backend_retryable(&app, id, generation, &error),
                }
                return;
            }
            BackendAction::Failed(error) => {
                fail_backend_terminal(&app, id, generation, &error);
                return;
            }
            BackendAction::NeedsRemote => {
                if needs_remote_decision(backend.profile) == NeedsRemoteDecision::RejectLocalOnly {
                    fail_backend_terminal(
                        &app,
                        id,
                        generation,
                        "Local-only conversion could not finish locally. Choose Standard to allow Datalab fallback.",
                    );
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
                if !set_backend_status(&app, id, generation, "processing", note) {
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
        let Some(backend_job_id) = backend.backend_job_id.as_deref() else {
            fail_backend_retryable(
                &app,
                id,
                generation,
                "Conversion service returned no job id",
            );
            return;
        };
        let result =
            conversion_service::poll_conversion(&backend.backend_url, &token, backend_job_id).await;
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
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let pipeline = cfg
        .datalab_pipeline_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if stale(app) {
        return;
    }
    if !set_backend_status(
        app,
        id,
        generation,
        "working",
        "Uploading to Datalab fallback…",
    ) {
        return;
    }
    let submitted = match pipeline {
        Some(pipeline_id) => {
            providers::datalab_pipeline_submit(
                &client,
                &api_key,
                pipeline_id,
                &original.source_path,
                &cfg.datalab_format,
            )
            .await
        }
        None => {
            providers::datalab_submit(
                &client,
                &api_key,
                &original.source_path,
                &cfg.datalab_format,
                cfg.datalab_high_accuracy,
            )
            .await
        }
    };
    if stale(app) {
        return;
    }
    let submitted = match submitted {
        Ok(value) => value,
        Err(error) => {
            fail_backend_retryable(app, id, generation, &error);
            return;
        }
    };
    let check_url = submitted.check_url.clone().unwrap_or_else(|| {
        format!(
            "https://www.datalab.to/api/v1/convert/{}",
            submitted.remote_id
        )
    });
    if !set_backend_status(
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
        let result = match pipeline {
            Some(_) => {
                providers::datalab_pipeline_poll(&client, &api_key, &submitted.remote_id).await
            }
            None => {
                providers::datalab_poll(&client, &api_key, &check_url, &cfg.datalab_format).await
            }
        };
        if stale(app) {
            return;
        }
        match result {
            Ok(PollResult::Done(text)) => break text,
            Ok(PollResult::Failed(error)) => {
                fail_backend_retryable(app, id, generation, &error);
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

    let extension = match cfg.datalab_format.as_str() {
        "html" => "html",
        "json" | "chunks" => "json",
        _ => "md",
    };
    if stale(app) {
        return;
    }
    let output = write_output(
        &original.output_dir,
        &original.file_name,
        extension,
        "",
        &text,
    );
    let Some(output) =
        completed_output_if_current(app.state::<JobManager>().generation(), generation, output)
    else {
        return;
    };
    match output {
        Ok(path) => finish_backend(app, id, generation, path),
        Err(error) => fail_backend_retryable(app, id, generation, &error),
    }
}

/// Spawn the full lifecycle for one job on the async runtime. `generation` is
/// the run this job belongs to; if the manager has moved on (a new run started,
/// or the user hit Stop) the task exits without touching state or spending money.
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
                    &format!(
                        "No API key set for {}. Add it in Settings.",
                        provider.label()
                    ),
                );
                return;
            }
        };

        // Respect the concurrency cap for the whole job lifetime.
        let sem = app.state::<JobManager>().semaphore();
        let _permit = sem.acquire_owned().await;
        // Waiting for a permit can take minutes; the run may be long gone.
        if stale(&app) {
            return;
        }

        if let Some(updated) = app.state::<JobManager>().update(id, |j| {
            j.started_at = Some(now_secs());
        }) {
            emit(&app, updated);
        }

        // Snapshot taken when the run started, so a Settings change mid-run
        // can't give half the files a different output format.
        let cfg = app.state::<JobManager>().run_config();
        // No global timeout: submits carry whole files and need far longer
        // than polls, so each request sets its own (see providers::*_TIMEOUT).
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        {
            set_status(&app, id, "working", "Uploading…");
            let datalab_format = cfg.datalab_format.clone();
            let pipeline = cfg
                .datalab_pipeline_id
                .clone()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());

            let submitted = match job.job_type {
                JobType::Convert => {
                    let res = match &pipeline {
                        Some(pid) => {
                            providers::datalab_pipeline_submit(
                                &client,
                                &api_key,
                                pid,
                                &job.source_path,
                                &datalab_format,
                            )
                            .await
                        }
                        None => {
                            providers::datalab_submit(
                                &client,
                                &api_key,
                                &job.source_path,
                                &datalab_format,
                                cfg.datalab_high_accuracy,
                            )
                            .await
                        }
                    };
                    match res {
                        Ok(s) => s,
                        Err(e) => {
                            if !stale(&app) {
                                fail(&app, id, &e);
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
                                fail(&app, id, &e);
                            }
                            return;
                        }
                    }
                }
            };

            // A submit carries the whole file and can run for half an hour,
            // so Stop is very likely to land during one. Without this check
            // the finished upload writes "processing" back over a row the
            // user already stopped, and since the poll loop then bails on
            // its own stale check, nothing ever moves that row to a
            // terminal state: Run stays disabled behind a job with no task.
            // The two arms above are guarded for the same reason — a
            // stopped job must not be relabelled with a network error, or
            // it lands in the history as a genuine failure.
            if stale(&app) {
                return;
            }
            set_status(&app, id, "processing", "Processing…");
            let convert_check_url = submitted.check_url.clone().unwrap_or_else(|| {
                format!(
                    "https://www.datalab.to/api/v1/convert/{}",
                    submitted.remote_id
                )
            });

            let max_attempts: u32 = 720; // ~60 min at 5s
            let mut attempt: u32 = 0;
            // Consecutive network/parse errors. Transient blips are fine to
            // ride out, but an endless stream of them should fail the job
            // rather than silently burn the full 60-minute budget.
            let mut consecutive_errors: u32 = 0;
            let text = loop {
                attempt += 1;
                if attempt > max_attempts {
                    fail(&app, id, "Timed out waiting for the result.");
                    return;
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
                if stale(&app) {
                    return;
                }
                let poll = match job.job_type {
                    JobType::Convert => match &pipeline {
                        Some(_) => {
                            providers::datalab_pipeline_poll(
                                &client,
                                &api_key,
                                &submitted.remote_id,
                            )
                            .await
                        }
                        None => {
                            providers::datalab_poll(
                                &client,
                                &api_key,
                                &convert_check_url,
                                &datalab_format,
                            )
                            .await
                        }
                    },
                    JobType::Transcribe => {
                        providers::revai_poll(&client, &api_key, &submitted.remote_id).await
                    }
                };
                match poll {
                    Ok(PollResult::Done(t)) => break t,
                    Ok(PollResult::Failed(e)) => {
                        fail(&app, id, &e);
                        return;
                    }
                    Ok(PollResult::Pending) => {
                        consecutive_errors = 0;
                        continue;
                    }
                    Err(e) => {
                        consecutive_errors += 1;
                        if consecutive_errors >= 12 {
                            // ~1 minute of unbroken failure.
                            fail(
                                &app,
                                id,
                                &format!("Lost contact while waiting for the result: {e}"),
                            );
                            return;
                        }
                        continue;
                    }
                }
            };

            let ext = match job.job_type {
                JobType::Convert => match datalab_format.as_str() {
                    "html" => "html",
                    "json" | "chunks" => "json",
                    _ => "md",
                },
                JobType::Transcribe => "txt",
            };
            if stale(&app) {
                return;
            }
            match write_output(&job.output_dir, &job.file_name, ext, "", &text) {
                Ok(path) => finish(&app, id, text, Some(path)),
                Err(e) => fail(&app, id, &e),
            }
        }
    });
}

fn validate_recovery_entry(
    entry: &history::InFlightEntry,
) -> Result<settings::ConversionProfile, String> {
    conversion_service::validate_base_url(&entry.backend_url)
        .map_err(|error| format!("Cannot recover conversion: {error}"))?;
    if let Some(job_id) = entry.backend_job_id.as_deref() {
        conversion_service::validate_conversion_id(job_id)
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

/// Recreate durable backend rows after startup. Invalid legacy origins and
/// changed sources stay visible and stopped, but their durable rows are
/// removed so they cannot resurrect forever or bind to current settings.
pub(crate) fn recover_in_flight(app: AppHandle) {
    let Some(entries) = history::list_in_flight(&app) else {
        return;
    };
    if entries.is_empty() {
        return;
    }

    let manager = app.state::<JobManager>();
    manager.set_run_config(settings::load(&app));
    let generation = manager.generation();
    for entry in entries {
        let validation = validate_recovery_entry(&entry);
        let profile = validation
            .as_ref()
            .copied()
            .unwrap_or(settings::ConversionProfile::Standard);
        let recovery_error = validation.err();
        let durable_key = entry.idempotency_key.clone();
        let mut context = BackendContext::new(
            entry.backend_url,
            entry.client_run_id,
            entry.idempotency_key,
            entry.backend_job_id,
            profile,
        );
        context.recovery_blocker.clone_from(&recovery_error);
        let id = manager.next_id();
        let mut job = Job::new_backend(id, entry.source_path, entry.output_dir, context);
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
mod backend_tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

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
    fn needs_remote_falls_back_only_for_standard() {
        assert_eq!(
            needs_remote_decision(settings::ConversionProfile::Standard),
            NeedsRemoteDecision::DatalabFallback
        );
        assert_eq!(
            needs_remote_decision(settings::ConversionProfile::LocalOnly),
            NeedsRemoteDecision::RejectLocalOnly
        );
    }

    #[test]
    fn stale_completed_output_removes_only_the_new_collision_safe_path() {
        let root = std::env::temp_dir().join(format!(
            "tool-kit-stale-backend-output-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let existing = root.join("report.md");
        let created = root.join("report (1).md");
        std::fs::write(&existing, b"existing").unwrap();
        std::fs::write(&created, b"new").unwrap();

        assert_eq!(
            completed_output_if_current(2, 1, Ok(created.to_string_lossy().into_owned())),
            None
        );
        assert!(!created.exists());
        assert_eq!(std::fs::read(&existing).unwrap(), b"existing");

        let current = root.join("report (2).md");
        std::fs::write(&current, b"current").unwrap();
        let current_path = current.to_string_lossy().into_owned();
        assert_eq!(
            completed_output_if_current(2, 2, Ok(current_path.clone())),
            Some(Ok(current_path))
        );
        assert!(current.exists());
        assert_eq!(
            completed_output_if_current(2, 1, Err("stale network error".into())),
            None
        );

        let _ = std::fs::remove_dir_all(root);
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
        );
        let job = Job::new_backend(1, "/tmp/report.pdf".into(), "/tmp".into(), context.clone());

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
        assert_eq!(serialized["outputText"], serde_json::Value::Null);
    }

    #[test]
    fn invalid_recovery_rows_are_rejected_before_resume_and_cleanup() {
        let root = std::env::temp_dir().join(format!(
            "tool-kit-recovery-validation-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
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
        let mut entry = history::InFlightEntry {
            source_path: source.to_string_lossy().into_owned(),
            file_name: "report.pdf".into(),
            output_dir: root.to_string_lossy().into_owned(),
            backend_url: "http://127.0.0.1:8080".into(),
            client_run_id: "11111111-1111-4111-8111-111111111111".into(),
            idempotency_key: "22222222-2222-4222-8222-222222222222".into(),
            backend_job_id: Some("33333333-3333-4333-8333-333333333333".into()),
            conversion_profile: "standard".into(),
            source_mtime,
            created_at: 1,
        };
        assert!(validate_recovery_entry(&entry).is_ok());

        entry.backend_job_id = Some("not-a-uuid".into());
        assert!(validate_recovery_entry(&entry)
            .unwrap_err()
            .contains("conversion id"));
        entry.backend_job_id = None;
        entry.conversion_profile = "future_profile".into();
        assert!(validate_recovery_entry(&entry)
            .unwrap_err()
            .contains("unknown profile"));
        entry.conversion_profile = "standard".into();
        std::fs::remove_file(&source).unwrap();
        assert!(validate_recovery_entry(&entry)
            .unwrap_err()
            .contains("source changed or is missing"));
        let _ = std::fs::remove_dir_all(root);
    }
}
