//! In-memory job queue. A "run" clears the queue, then one job per matching
//! file is submitted to its provider and polled on a background task, with a
//! concurrency cap so we don't hammer the upstream APIs.

use serde::Serialize;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Semaphore;

use crate::providers::{self, PollResult, ProviderKind};
use crate::{history, secrets, settings};

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
    pub created_at: u64,
    /// When this job actually left the queue. The elapsed timer counts from
    /// here so a file waiting behind the concurrency cap doesn't appear to
    /// have been processing for the whole wait.
    pub started_at: Option<u64>,
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
            created_at: now_secs(),
            started_at: None,
        }
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

    /// Retire every in-flight task and return the new generation.
    pub fn new_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
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
    history::record(
        app,
        &history::Finished {
            file_name: &job.file_name,
            source_path: &job.source_path,
            output_path: job.output_path.as_deref(),
            job_type: job.job_type.id(),
            output_format: &output_format_for(job.job_type, &cfg),
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
