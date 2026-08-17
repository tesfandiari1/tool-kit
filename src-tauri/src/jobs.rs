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
use crate::{secrets, settings};

/// The "job to be done" the user picks once for the whole run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobType {
    Convert,
    Transcribe,
    Summarize,
}

impl JobType {
    pub fn from_id(s: &str) -> Option<Self> {
        match s {
            "convert" => Some(JobType::Convert),
            "transcribe" => Some(JobType::Transcribe),
            "summarize" => Some(JobType::Summarize),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            JobType::Convert => "Convert",
            JobType::Transcribe => "Transcribe",
            JobType::Summarize => "Summarize",
        }
    }

    pub fn provider(&self) -> ProviderKind {
        match self {
            JobType::Convert => ProviderKind::Datalab,
            JobType::Transcribe => ProviderKind::RevAi,
            JobType::Summarize => ProviderKind::Anthropic,
        }
    }

    /// Does this job accept a file with the given lowercase extension?
    pub fn accepts(&self, ext: &str) -> bool {
        match self {
            JobType::Convert => matches!(
                ext,
                "pdf" | "png" | "jpg" | "jpeg" | "webp" | "tiff" | "tif" | "gif" | "bmp" | "docx"
                    | "doc" | "pptx" | "ppt" | "xlsx" | "xls" | "html" | "htm" | "epub"
            ),
            JobType::Transcribe => matches!(
                ext,
                "mp3" | "mp4" | "wav" | "m4a" | "flac" | "ogg" | "oga" | "aac" | "mov" | "avi"
                    | "mkv" | "webm" | "wmv" | "mpeg" | "mpg" | "opus" | "amr" | "3gp"
            ),
            JobType::Summarize => matches!(ext, "txt" | "md" | "markdown" | "text" | "rtf"),
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
}

impl Default for JobManager {
    fn default() -> Self {
        Self {
            jobs: Mutex::new(Vec::new()),
            counter: AtomicU64::new(0),
            sem: Arc::new(Semaphore::new(4)), // up to 4 files in flight at once
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
        self.jobs.lock().unwrap().iter().find(|j| j.id == id).cloned()
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
}

fn emit(app: &AppHandle, job: Job) {
    let _ = app.emit("job-updated", job);
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
        emit(app, job);
    }
}

/// Write `text` into the job's output folder, avoiding clobbering.
fn write_output(output_dir: &str, file_name: &str, ext: &str, suffix: &str, text: &str) -> Option<String> {
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let base = std::path::PathBuf::from(output_dir);
    let mut candidate = base.join(format!("{stem}{suffix}.{ext}"));
    let mut n = 1;
    while candidate.exists() {
        candidate = base.join(format!("{stem}{suffix} ({n}).{ext}"));
        n += 1;
    }
    std::fs::write(&candidate, text.as_bytes())
        .ok()
        .map(|_| candidate.to_string_lossy().to_string())
}

const MAX_SUMMARIZE_CHARS: usize = 500_000;

/// Slice `s` to at most `max_chars` characters, always at a valid UTF-8
/// boundary — plain byte slicing (`&s[..n]`) panics on multibyte input.
fn truncate_chars(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

/// Spawn the full lifecycle for one job on the async runtime.
pub fn run_job(app: AppHandle, id: u64) {
    tauri::async_runtime::spawn(async move {
        let job = match app.state::<JobManager>().get(id) {
            Some(j) => j,
            None => return,
        };
        let provider = job.job_type.provider();

        let api_key = match secrets::get_key(provider.key_name()) {
            Some(k) if !k.is_empty() => k,
            _ => {
                fail(&app, id, &format!("No API key set for {}. Add it in Settings.", provider.label()));
                return;
            }
        };

        // Respect the concurrency cap for the whole job lifetime.
        let sem = app.state::<JobManager>().semaphore();
        let _permit = sem.acquire_owned().await;

        let cfg = settings::load(&app);
        // Bound hung connections so a stuck request can't hold a concurrency
        // permit forever (reqwest has no default timeout).
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .connect_timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        match job.job_type {
            JobType::Summarize => {
                set_status(&app, id, "processing", "Reading…");
                let raw = match tokio::fs::read_to_string(&job.source_path).await {
                    Ok(t) => t,
                    Err(e) => {
                        fail(&app, id, &format!("Could not read file: {e}"));
                        return;
                    }
                };
                let text = truncate_chars(&raw, MAX_SUMMARIZE_CHARS);
                set_status(&app, id, "processing", "Summarizing…");
                match providers::anthropic_summarize(&client, &api_key, &cfg.summarize_model, text).await {
                    Ok(summary) => {
                        let path = write_output(&job.output_dir, &job.file_name, "md", ".summary", &summary);
                        finish(&app, id, summary, path);
                    }
                    Err(e) => fail(&app, id, &e),
                }
            }
            JobType::Convert | JobType::Transcribe => {
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
                                    &client, &api_key, pid, &job.source_path, &datalab_format,
                                )
                                .await
                            }
                            None => {
                                providers::datalab_submit(&client, &api_key, &job.source_path, &datalab_format).await
                            }
                        };
                        match res {
                            Ok(s) => s,
                            Err(e) => {
                                fail(&app, id, &e);
                                return;
                            }
                        }
                    }
                    _ => match providers::revai_submit(&client, &api_key, &job.source_path).await {
                        Ok(s) => s,
                        Err(e) => {
                            fail(&app, id, &e);
                            return;
                        }
                    },
                };

                set_status(&app, id, "processing", "Processing…");
                let convert_check_url = submitted.check_url.clone().unwrap_or_else(|| {
                    format!("https://www.datalab.to/api/v1/convert/{}", submitted.remote_id)
                });

                let max_attempts: u32 = 720; // ~60 min at 5s
                let mut attempt: u32 = 0;
                let text = loop {
                    attempt += 1;
                    if attempt > max_attempts {
                        fail(&app, id, "Timed out waiting for the result.");
                        return;
                    }
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    let poll = match job.job_type {
                        JobType::Convert => match &pipeline {
                            Some(_) => {
                                providers::datalab_pipeline_poll(&client, &api_key, &submitted.remote_id).await
                            }
                            None => {
                                providers::datalab_poll(&client, &api_key, &convert_check_url, &datalab_format).await
                            }
                        },
                        _ => providers::revai_poll(&client, &api_key, &submitted.remote_id).await,
                    };
                    match poll {
                        Ok(PollResult::Done(t)) => break t,
                        Ok(PollResult::Failed(e)) => {
                            fail(&app, id, &e);
                            return;
                        }
                        Ok(PollResult::Pending) => continue,
                        Err(_) => continue, // transient network error; keep polling
                    }
                };

                let ext = match job.job_type {
                    JobType::Convert => match datalab_format.as_str() {
                        "html" => "html",
                        "json" | "chunks" => "json",
                        _ => "md",
                    },
                    _ => "txt",
                };
                let path = write_output(&job.output_dir, &job.file_name, ext, "", &text);
                finish(&app, id, text, path);
            }
        }
    });
}
