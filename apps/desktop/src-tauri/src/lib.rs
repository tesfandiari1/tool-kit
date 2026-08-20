mod backend_host;
mod conversion_service;
mod history;
mod jobs;
mod providers;
mod secrets;
mod settings;
mod workspace;

#[cfg(test)]
mod live_smoke;

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tauri::{AppHandle, Emitter, Manager, State};

use jobs::{Job, JobManager, JobType};
use settings::Settings;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SecretStatus {
    datalab: bool,
    revai: bool,
    backend: bool,
}

#[tauri::command]
fn secret_status() -> SecretStatus {
    SecretStatus {
        datalab: secrets::has_key("datalab"),
        revai: secrets::has_key("revai"),
        backend: secrets::has_key("backend"),
    }
}

#[tauri::command]
fn set_secret(provider: String, value: String) -> Result<(), String> {
    match provider.as_str() {
        "datalab" | "revai" | "backend" => secrets::set_key(&provider, value.trim()),
        _ => Err("Unknown provider".into()),
    }
}

#[tauri::command]
fn get_settings(app: AppHandle) -> Settings {
    settings::load(&app)
}

#[tauri::command]
fn save_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    settings::save(&app, &settings)
}

#[tauri::command]
fn list_jobs(state: State<JobManager>) -> Vec<Job> {
    state.list()
}

/// The folder the first-launch picker opens on: ~/Documents/Tool-Kit.
#[tauri::command]
fn suggested_workspace_path() -> String {
    workspace::suggested_workspace_path()
}

/// Whether the picked folder is already a Tool-Kit workspace, so onboarding
/// can offer "adopt" instead of "create".
#[tauri::command]
fn inspect_workspace_path(path: String) -> bool {
    workspace::inspect_workspace_path(&path)
}

/// First-launch setup. The folder is created or adopted, and the settings
/// that bind this install to it are saved here — the webview can lose a race
/// against `save_settings`, so the host persists rather than trusting it to.
#[tauri::command]
fn setup_workspace(app: AppHandle, path: String) -> Result<workspace::WorkspaceInfo, String> {
    let info = workspace::setup_workspace(&path)?;
    let mut cfg = settings::load(&app);
    cfg.workspace_path = Some(info.workspace_path.clone());
    cfg.workspace_id = Some(info.workspace_id.clone());
    cfg.active_project_id = Some(info.inbox_project_id.clone());
    cfg.onboarding_complete = true;
    settings::save(&app, &cfg)?;
    Ok(info)
}

#[tauri::command]
fn list_projects(app: AppHandle) -> Result<Vec<workspace::ProjectSummary>, String> {
    workspace::list_projects(&app)
}

/// How deep a dropped folder is walked. Deep enough for real project trees,
/// shallow enough that dropping a home folder can't wander forever.
const MAX_SCAN_DEPTH: usize = 8;

/// Expand the selected inputs (files and/or folders) into the concrete list of
/// files this job will process. Folders are walked recursively for files the
/// job accepts; individual files are included if they match.
///
/// Deliberately does **not** exclude the output folder. An earlier version did,
/// to stop a second run re-processing its own results — but since results are
/// written alongside their sources by default, that excluded the inputs
/// themselves and nothing was ever eligible. The case it guarded against needs
/// Convert with `html` output re-reading its own `.html`, which is narrow and
/// self-limiting; `write_output` numbers collisions rather than clobbering.
fn collect_input_files(inputs: &[String], jt: JobType) -> Vec<std::path::PathBuf> {
    let accepts = |p: &Path| -> bool {
        p.extension()
            .and_then(|e| e.to_str())
            .map(|e| jt.accepts(&e.to_lowercase()))
            .unwrap_or(false)
    };
    collect_files_where(inputs, &accepts)
}

/// Broad discovery only. The live capability MIME set is intersected below;
/// this function never decides that an arbitrary file is convertible.
fn collect_backend_candidates(inputs: &[String]) -> Vec<std::path::PathBuf> {
    let accepts = |path: &Path| {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        extension.as_deref() == Some("csv")
            || !extension.is_some_and(|extension| ALREADY_TEXT.contains(&extension.as_str()))
    };
    collect_files_where(inputs, &accepts)
}

fn collect_files_where(
    inputs: &[String],
    accepts: &dyn Fn(&Path) -> bool,
) -> Vec<std::path::PathBuf> {
    // Skip dotfiles and dot-directories: .git, .DS_Store, and friends are never
    // what the user meant to convert. Only applies while walking *into* a
    // folder — a dot-path dropped explicitly is still honoured.
    let hidden = |p: &Path| -> bool {
        p.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with('.'))
            .unwrap_or(false)
    };

    fn walk(
        dir: &Path,
        depth: usize,
        files: &mut Vec<std::path::PathBuf>,
        accepts: &dyn Fn(&Path) -> bool,
        hidden: &dyn Fn(&Path) -> bool,
    ) {
        if depth > MAX_SCAN_DEPTH {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if hidden(&p) {
                continue;
            }
            match entry.file_type() {
                Ok(t) if t.is_dir() => walk(&p, depth + 1, files, accepts, hidden),
                Ok(t) if t.is_file() && accepts(&p) => files.push(p),
                _ => {}
            }
        }
    }

    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for input in inputs {
        let path = Path::new(input);
        if path.is_dir() {
            walk(path, 0, &mut files, &accepts, &hidden);
        } else if path.is_file() && accepts(path) {
            files.push(path.to_path_buf());
        }
    }
    // Canonicalize before dedup so the same file reached two ways (a file plus
    // its enclosing folder) is only queued once.
    files.sort();
    files.dedup();
    let mut seen = std::collections::HashSet::new();
    files.retain(|p| seen.insert(p.canonicalize().unwrap_or_else(|_| p.clone())));
    files
}

/// Formats that are already plain text. There is nothing to extract from them,
/// so they are skipped rather than sent to a provider — but they are counted so
/// the UI can say "already text" instead of reporting an unexplained zero.
const ALREADY_TEXT: &[&str] = &[
    "txt", "md", "markdown", "text", "rst", "org", "csv", "tsv", "json",
];

/// These formats have no local engine and intentionally bypass the backend
/// forever. Every other decision comes from live `capabilities.inputFormats`,
/// images included: the Vision engine only exists on macOS 26 and up, so the
/// same build has to route an image either way depending on what the service
/// it is talking to actually advertises.
const PERMANENT_DIRECT_FORMATS: &[&str] = &["html", "htm"];

fn is_permanent_direct(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            PERMANENT_DIRECT_FORMATS.contains(&extension.to_ascii_lowercase().as_str())
        })
}

fn media_type(path: &Path) -> String {
    mime_guess::from_path(path)
        .first_or_octet_stream()
        .essence_str()
        .to_ascii_lowercase()
}

#[derive(Default)]
struct NativeConversionPlan {
    backend: Vec<std::path::PathBuf>,
    direct: Vec<std::path::PathBuf>,
}

fn route_conversion_candidates(
    candidates: Vec<std::path::PathBuf>,
    supported: &HashSet<String>,
) -> NativeConversionPlan {
    let mut plan = NativeConversionPlan::default();
    for file in candidates {
        if is_permanent_direct(&file) {
            plan.direct.push(file);
        } else if supported.contains(&media_type(&file)) {
            plan.backend.push(file);
        } else if JobType::Convert.accepts(
            file.extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase()
                .as_str(),
        ) {
            plan.direct.push(file);
        }
    }
    plan
}

fn require_backend_capacity(
    plan: NativeConversionPlan,
    accepting_jobs: bool,
) -> Result<NativeConversionPlan, String> {
    if !accepting_jobs && !plan.backend.is_empty() {
        Err("The local conversion service is not accepting jobs".into())
    } else {
        Ok(plan)
    }
}

async fn plan_backend_conversion_files(
    inputs: &[String],
    backend_url: &str,
) -> Result<NativeConversionPlan, String> {
    let candidates = collect_backend_candidates(inputs);
    if candidates.iter().all(|file| is_permanent_direct(file)) {
        return Ok(route_conversion_candidates(candidates, &HashSet::new()));
    }

    let capabilities = conversion_service::fetch_capabilities(backend_url)
        .await
        .map_err(|error| format!("The local conversion service is unavailable: {error}"))?;
    let accepting_jobs = capabilities.accepting_jobs;
    let supported = capabilities
        .input_formats
        .into_iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    require_backend_capacity(
        route_conversion_candidates(candidates, &supported),
        accepting_jobs,
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScannedConversionFile {
    source_path: String,
    media_type: String,
    reuse: ReuseDisposition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ReuseDisposition {
    Pending,
    AlreadyHere,
    Reusable,
}

#[derive(Default)]
struct ReuseSummary {
    already_here: usize,
    reusable: usize,
    by_source: HashMap<String, ReuseDisposition>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Scan {
    /// Matching file count per job, so the UI can pick the job that fits the
    /// selection and show what each one would process.
    convert: usize,
    transcribe: usize,
    /// Files whose result is already sitting in the chosen output folder.
    /// Nothing at all happens to these. Reported per job so switching job
    /// reads a number already in hand rather than triggering a fresh scan.
    already_here_convert: usize,
    already_here_transcribe: usize,
    /// Files whose result exists, but in some other folder. These are copied
    /// rather than sent to the provider again — real work, but free.
    reusable_convert: usize,
    reusable_transcribe: usize,
    /// Concrete Convert files and their MIME/reuse disposition. This is
    /// metadata only: no file bytes cross IPC. The webview uses it to plan
    /// each pending file against live backend capabilities.
    convert_files: Vec<ScannedConversionFile>,
    /// Files skipped because they are already text.
    already_text: usize,
    /// The folder to default the output to: the dropped folder itself, or the
    /// folder holding the dropped files when they all share one.
    suggested_output: Option<String>,
}

/// Count what each job would process across the selected inputs, and how much
/// of that has already been done.
///
/// `(async)` on a sync fn moves the body off the main thread. This runs on
/// every drop and walks the tree three times, then canonicalizes and stats
/// every match — fine on a local SSD, but on a network volume the per-file
/// round-trips would freeze the window while it ran.
#[tauri::command]
async fn scan_inputs(app: AppHandle, inputs: Vec<String>) -> Result<Scan, String> {
    let cfg = settings::load(&app);
    let convert = if cfg.conversion_route == settings::ConversionRoute::Backend {
        // A service still starting reads the same here as one that cannot
        // answer: either way the scan falls back to direct-eligible metadata.
        let planned = match backend_host::backend_origin(&app) {
            Ok(origin) => plan_backend_conversion_files(&inputs, &origin).await,
            Err(error) => Err(error),
        };
        match planned {
            Ok(mut plan) => {
                plan.direct.append(&mut plan.backend);
                plan.direct
            }
            // The frontend's independent capability probe owns the actionable
            // outage message. Preserve direct-eligible scan metadata here so
            // it is not masked by an empty selection.
            Err(_) => collect_input_files(&inputs, JobType::Convert),
        }
    } else {
        collect_input_files(&inputs, JobType::Convert)
    };
    let transcribe = collect_input_files(&inputs, JobType::Transcribe);
    let convert_reuse = split_reusable(&app, &convert, JobType::Convert, &cfg);
    let transcribe_reuse = split_reusable(&app, &transcribe, JobType::Transcribe, &cfg);
    let convert_files = describe_conversion_files(&convert, &convert_reuse);
    Ok(Scan {
        already_here_convert: convert_reuse.already_here,
        already_here_transcribe: transcribe_reuse.already_here,
        reusable_convert: convert_reuse.reusable,
        reusable_transcribe: transcribe_reuse.reusable,
        convert_files,
        convert: convert.len(),
        transcribe: transcribe.len(),
        already_text: count_matching(&inputs, ALREADY_TEXT),
        suggested_output: suggested_output_dir(&inputs),
    })
}

/// Split the files that need no provider call into (already in the output
/// folder, copyable from elsewhere). The two are counted apart because they
/// mean different things to the user: the first is "nothing happens", the
/// second is "a file appears, for free".
fn split_reusable(
    app: &AppHandle,
    files: &[std::path::PathBuf],
    jt: JobType,
    cfg: &Settings,
) -> ReuseSummary {
    if files.is_empty() {
        return ReuseSummary::default();
    }
    let found = history::reusable(app, files, jt.id(), &jobs::output_format_for(jt, cfg));
    let mut summary = ReuseSummary::default();
    for (source, output) in found {
        let disposition = if cfg
            .output_dir
            .as_deref()
            .is_some_and(|dir| history::is_in_dir(&output, dir))
        {
            summary.already_here += 1;
            ReuseDisposition::AlreadyHere
        } else {
            summary.reusable += 1;
            ReuseDisposition::Reusable
        };
        summary.by_source.insert(source, disposition);
    }
    summary
}

fn describe_conversion_files(
    files: &[std::path::PathBuf],
    reuse: &ReuseSummary,
) -> Vec<ScannedConversionFile> {
    files
        .iter()
        .map(|path| {
            let source_path = path.to_string_lossy().into_owned();
            let media_type = mime_guess::from_path(path)
                .first_or_octet_stream()
                .essence_str()
                .to_string();
            ScannedConversionFile {
                reuse: reuse
                    .by_source
                    .get(&source_path)
                    .copied()
                    .unwrap_or(ReuseDisposition::Pending),
                source_path,
                media_type,
            }
        })
        .collect()
}

/// Newest first. `query` filters on file name or folder; empty means everything.
#[tauri::command]
fn list_history(app: AppHandle, query: String, limit: u32) -> Vec<history::Entry> {
    history::list(&app, &query, limit)
}

#[tauri::command]
fn clear_history(app: AppHandle) -> Result<(), String> {
    history::clear(&app)
}

/// Count selected files whose extension is in `exts`, walking folders the same
/// way `collect_input_files` does so the two counts describe the same set.
fn count_matching(inputs: &[String], exts: &[&str]) -> usize {
    fn walk(dir: &Path, depth: usize, exts: &[&str], n: &mut usize) {
        if depth > MAX_SCAN_DEPTH {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.starts_with('.'))
            {
                continue;
            }
            match entry.file_type() {
                Ok(t) if t.is_dir() => walk(&p, depth + 1, exts, n),
                Ok(t) if t.is_file() && has_ext(&p, exts) => *n += 1,
                _ => {}
            }
        }
    }
    fn has_ext(p: &Path, exts: &[&str]) -> bool {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| exts.contains(&e.to_lowercase().as_str()))
    }

    let mut n = 0;
    for input in inputs {
        let path = Path::new(input);
        if path.is_dir() {
            walk(path, 0, exts, &mut n);
        } else if path.is_file() && has_ext(path, exts) {
            n += 1;
        }
    }
    n
}

/// Where results should land by default: alongside the input. A dropped folder
/// is its own answer; dropped files use their containing folder, but only when
/// they agree, so a mixed selection doesn't silently pick one at random.
fn suggested_output_dir(inputs: &[String]) -> Option<String> {
    let mut candidate: Option<std::path::PathBuf> = None;
    for input in inputs {
        let path = Path::new(input);
        let dir = if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent()?.to_path_buf()
        };
        match &candidate {
            None => candidate = Some(dir),
            Some(existing) if *existing == dir => {}
            Some(_) => return None,
        }
    }
    candidate.map(|p| p.to_string_lossy().to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RunResult {
    count: usize,
    /// Files left alone: their result is already in the output folder.
    skipped: usize,
    /// Files satisfied by copying a result an earlier run produced elsewhere.
    copied: usize,
}

fn backend_inflight_keys(jobs: &[Job]) -> Vec<&str> {
    jobs.iter()
        .filter_map(|job| {
            job.backend
                .as_ref()
                .map(|backend| backend.idempotency_key.as_str())
        })
        .collect()
}

fn delete_backend_inflight(app: &AppHandle, jobs: &[Job]) {
    for key in backend_inflight_keys(jobs) {
        history::delete_in_flight(app, key);
    }
}

#[tauri::command]
async fn run_pipeline(
    app: AppHandle,
    state: State<'_, JobManager>,
    inputs: Vec<String>,
    output_dir: String,
    job_type: String,
) -> Result<RunResult, String> {
    let jt = JobType::from_id(&job_type).ok_or("Unknown job type")?;
    if inputs.is_empty() {
        return Err("Choose at least one file or folder".into());
    }
    if !Path::new(&output_dir).is_dir() {
        return Err("Choose an output folder first".into());
    }
    // Loaded once and reused: the same config decides collection, routing,
    // history reuse, and what the run itself produces.
    let cfg = settings::load(&app);
    let mut backend_files = Vec::new();
    // Read once for the whole run. Every ledger row and every job context has
    // to name the origin the submit actually goes to, and in Sidecar mode that
    // is a port this launch was handed rather than anything in Settings.
    let mut backend_origin = String::new();
    let mut files =
        if jt == JobType::Convert && cfg.conversion_route == settings::ConversionRoute::Backend {
            backend_origin = backend_host::backend_origin(&app)?;
            let plan = plan_backend_conversion_files(&inputs, &backend_origin).await?;
            if cfg.conversion_profile == settings::ConversionProfile::LocalOnly
                && !plan.direct.is_empty()
            {
                let file_name = plan.direct[0]
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("This file");
                return Err(format!(
                    "{file_name} requires Datalab and cannot run with the Local-only profile"
                ));
            }
            backend_files = plan.backend;
            let mut routed = plan.direct;
            routed.extend(backend_files.iter().cloned());
            routed
        } else {
            collect_input_files(&inputs, jt)
        };
    if files.is_empty() {
        return Err(format!(
            "No {} files in your selection",
            jt.label().to_lowercase()
        ));
    }

    // Three buckets. A file whose result is already in the output folder is
    // left alone; one whose result exists elsewhere is copied, because the
    // source is unchanged and the format matches, so paying the provider again
    // would buy bytes we already have; everything else runs.
    let mut to_copy: Vec<(std::path::PathBuf, String)> = Vec::new();
    let mut skipped = 0;
    if cfg.skip_already_done
        && !(jt == JobType::Convert && cfg.conversion_route == settings::ConversionRoute::Backend)
    {
        let found = history::reusable(&app, &files, jt.id(), &jobs::output_format_for(jt, &cfg));
        if !found.is_empty() {
            files.retain(|p| match found.get(p.to_string_lossy().as_ref()) {
                None => true,
                Some(existing) if history::is_in_dir(existing, &output_dir) => {
                    skipped += 1;
                    false
                }
                Some(existing) => {
                    to_copy.push((p.clone(), existing.clone()));
                    false
                }
            });
        }
    }
    // Nothing to run and nothing to copy. Say so rather than starting an empty
    // run that finishes instantly and looks like a bug.
    if files.is_empty() && to_copy.is_empty() {
        return Err(format!(
            "Already done — all {skipped} file{} already have results in this folder. \
             Turn off “Skip files already done” in Settings to run them again.",
            if skipped == 1 { "" } else { "s" }
        ));
    }

    let backend_needed = files.iter().any(|file| backend_files.contains(file));
    let direct_needed = files.iter().any(|file| !backend_files.contains(file));
    if backend_needed {
        // Through the accessor, so Sidecar mode says the service is starting
        // rather than telling the user to paste a token it mints itself.
        backend_host::backend_token(&app)?;
    }
    if direct_needed {
        let provider = jt.provider();
        if !secrets::has_key(provider.key_name()) {
            return Err(format!("Add your {} API key in Settings", provider.label()));
        }
    }

    // Fresh slate per run. Bumping the generation retires any task still alive
    // from a previous run so it can't keep spending credits or writing files.
    let generation = state.new_generation();
    delete_backend_inflight(&app, &state.list());
    // Set before any copy lands: `finish` reads this config to decide what
    // format to file the new history row under.
    state.set_run_config(cfg.clone());
    state.clear();

    // Copies first, so the free results are on screen before the paid ones
    // start crawling.
    let mut copied = 0;
    for (source, existing) in &to_copy {
        let id = state.next_id();
        let job = Job::new(
            id,
            source.to_string_lossy().to_string(),
            output_dir.clone(),
            jt,
        );
        state.insert(job.clone());
        let _ = app.emit("job-updated", job);
        if jobs::reuse_result(&app, id, generation, existing) {
            copied += 1;
        }
    }

    // Once per run, beside the origin. Which deployment this is decides only
    // what the ledger records, and re-reading the file per file would let a
    // half-written override split one run across two recorded origins.
    let deployment = backend_host::deployment(&app)?;
    let client_run_id = uuid::Uuid::new_v4().to_string();
    // From the run snapshot, so editing the OCR settings mid-run cannot split
    // one run across two recognizers. Carried on each job and recorded on its
    // ledger row, because the service folds both into the replay fingerprint.
    let ocr = conversion_service::OcrOptions {
        language_correction: cfg.language_correction,
        custom_words: cfg.custom_words.clone(),
    };
    let ocr_custom_words = ocr.custom_words_wire();
    for source in &files {
        let id = state.next_id();
        let source_path = source.to_string_lossy().into_owned();
        let job = if backend_files.contains(source) {
            let idempotency_key = uuid::Uuid::new_v4().to_string();
            let file_name = source
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("file")
                .to_string();
            history::upsert_in_flight(
                &app,
                &history::NewInFlight {
                    source_path: &source_path,
                    file_name: &file_name,
                    output_dir: &output_dir,
                    backend_url: backend_host::ledger_origin(&deployment),
                    client_run_id: &client_run_id,
                    idempotency_key: &idempotency_key,
                    conversion_profile: cfg.conversion_profile.id(),
                    ocr_language_correction: ocr.language_correction,
                    ocr_custom_words: &ocr_custom_words,
                },
            );
            Job::new_backend(
                id,
                source_path,
                output_dir.clone(),
                jobs::BackendContext::new(
                    backend_origin.clone(),
                    client_run_id.clone(),
                    idempotency_key,
                    None,
                    cfg.conversion_profile,
                    ocr.clone(),
                ),
            )
        } else {
            Job::new(id, source_path, output_dir.clone(), jt)
        };
        state.insert(job.clone());
        let _ = app.emit("job-updated", job);
        jobs::run_job(app.clone(), id, generation);
    }
    Ok(RunResult {
        count: files.len(),
        skipped,
        copied,
    })
}

/// Stop the current run: retire in-flight tasks and mark anything unfinished
/// as stopped. Work already submitted upstream still costs what it cost, but
/// nothing further is started and no more results are written.
#[tauri::command]
fn stop_run(app: AppHandle, state: State<JobManager>) -> Result<usize, String> {
    state.new_generation();
    let mut stopped = 0;
    let jobs = state.list();
    delete_backend_inflight(&app, &jobs);
    for job in jobs {
        if matches!(job.status.as_str(), "queued" | "working" | "processing") {
            if let Some(updated) = state.update(job.id, |j| {
                j.status = "failed".into();
                j.progress_note = String::new();
                j.error = Some("Stopped".into());
            }) {
                let _ = app.emit("job-updated", updated);
                stopped += 1;
            }
        }
    }
    Ok(stopped)
}

#[tauri::command]
fn retry_job(app: AppHandle, state: State<JobManager>, id: u64) -> Result<(), String> {
    // Retrying joins the current run, so a later Stop cancels it too.
    let generation = state.generation();
    let updated = state
        .update(id, |j| {
            j.status = "queued".into();
            j.progress_note = "Queued".into();
            j.error = None;
            j.started_at = None;
        })
        .ok_or_else(|| "Job not found".to_string())?;
    // Emit immediately: acquiring a concurrency permit can take minutes, and
    // without this the row keeps its failed state and invites a second click.
    let _ = app.emit("job-updated", updated);
    jobs::restore_in_flight(&app, id);
    jobs::run_job(app.clone(), id, generation);
    Ok(())
}

/// Re-queue every failed job. A transient upstream blip can fail a handful of
/// files in a large batch, and retrying them one row at a time is busywork.
#[tauri::command]
fn retry_failed(app: AppHandle, state: State<JobManager>) -> Result<usize, String> {
    let generation = state.generation();
    let mut ids = Vec::new();
    for job in state.list() {
        if job.status == "failed" {
            if let Some(updated) = state.update(job.id, |j| {
                j.status = "queued".into();
                j.progress_note = "Queued".into();
                j.error = None;
                j.started_at = None;
            }) {
                let _ = app.emit("job-updated", updated);
                ids.push(job.id);
            }
        }
    }
    for id in &ids {
        jobs::restore_in_flight(&app, *id);
        jobs::run_job(app.clone(), *id, generation);
    }
    Ok(ids.len())
}

/// Quit for real. The close button only hides the window (this is a menu-bar
/// app), so "Stop and quit" needs an explicit way out.
#[tauri::command]
async fn quit_app(app: AppHandle) {
    // The sidecar goes down here rather than in `RunEvent::Exit`, which runs
    // inside applicationWillTerminate where a wait reads to the user as a hang.
    backend_host::stop(&app, backend_host::STOP_BUDGET).await;
    app.exit(0);
}

/// Show something in Finder. `reveal_item_in_dir` selects an item *inside its
/// parent*, which is right for a result file but opens the level above when
/// handed a folder — so directories are opened directly instead.
#[tauri::command]
fn reveal_path(app: AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    if Path::new(&path).is_dir() {
        app.opener()
            .open_path(path, None::<&str>)
            .map_err(|e| e.to_string())
    } else {
        app.opener()
            .reveal_item_in_dir(path)
            .map_err(|e| e.to_string())
    }
}

/// Read a result file into the viewer. Capped so a huge dump can't freeze the
/// webview; UTF-8 only, because this path is Markdown and transcripts.
const MAX_PREVIEW_BYTES: u64 = 2 * 1024 * 1024;

#[tauri::command]
fn read_text_file(path: String) -> Result<String, String> {
    let p = Path::new(&path);
    let meta = std::fs::metadata(p).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("Not a file".into());
    }
    if meta.len() > MAX_PREVIEW_BYTES {
        return Err("File is too large to preview".into());
    }
    std::fs::read_to_string(p).map_err(|e| e.to_string())
}

/// Reading and writing an editable document. Kept free of Tauri types so the
/// filesystem rules — the size cap, UTF-8 only, and the mtime check that stops
/// a save from overwriting an edit made outside the app — are unit-testable.
mod document_io {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::time::UNIX_EPOCH;

    #[derive(Debug)]
    pub struct Document {
        pub text: String,
        pub mtime_ms: u64,
    }

    fn mtime_ms(meta: &std::fs::Metadata) -> Result<u64, String> {
        let modified = meta.modified().map_err(|e| e.to_string())?;
        let since_epoch = modified
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?;
        u64::try_from(since_epoch.as_millis()).map_err(|e| e.to_string())
    }

    fn file_meta(path: &Path) -> Result<std::fs::Metadata, String> {
        let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
        if meta.is_file() {
            Ok(meta)
        } else {
            Err("Not a file".into())
        }
    }

    /// A hidden sibling so the rename stays on the same filesystem (a temp-dir
    /// staging file would fall back to a copy, which is not atomic) and a
    /// failed write doesn't leave visible cruft in the user's folder.
    fn temp_sibling(path: &Path) -> Result<PathBuf, String> {
        let name = path.file_name().ok_or_else(|| "Not a file".to_string())?;
        let mut tmp = OsString::from(".");
        tmp.push(name);
        tmp.push(".tmp");
        Ok(path.with_file_name(tmp))
    }

    pub fn read(path: &Path, max_bytes: u64) -> Result<Document, String> {
        let meta = file_meta(path)?;
        if meta.len() > max_bytes {
            return Err("File is too large to open".into());
        }
        let text = std::fs::read_to_string(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::InvalidData {
                "File is not UTF-8 text".to_string()
            } else {
                e.to_string()
            }
        })?;
        Ok(Document {
            text,
            mtime_ms: mtime_ms(&meta)?,
        })
    }

    /// Save, refusing when the file has changed since it was read. Written to a
    /// temp file and renamed in: `fs::write` truncates the destination first, so
    /// a crash or a full disk mid-write would leave the user's document
    /// half-length with no copy of the original anywhere.
    pub fn write(path: &Path, text: &str, expected_mtime_ms: u64) -> Result<u64, String> {
        let meta = file_meta(path)?;
        if mtime_ms(&meta)? != expected_mtime_ms {
            return Err("The file changed on disk. Reopen it to get the newer version.".into());
        }

        let tmp = temp_sibling(path)?;
        std::fs::write(&tmp, text.as_bytes()).map_err(|e| e.to_string())?;
        if let Err(e) = std::fs::rename(&tmp, path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.to_string());
        }
        mtime_ms(&file_meta(path)?)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentPayload {
    text: String,
    mtime_ms: u64,
}

#[tauri::command]
fn read_document(path: String) -> Result<DocumentPayload, String> {
    let doc = document_io::read(Path::new(&path), MAX_PREVIEW_BYTES)?;
    Ok(DocumentPayload {
        text: doc.text,
        mtime_ms: doc.mtime_ms,
    })
}

/// Copy reads the result off disk rather than a copy held in the webview, so it
/// needs a ceiling of its own. `MAX_PREVIEW_BYTES` exists to keep the editor
/// responsive, and a result too large to edit is still worth copying. This one
/// matches the backend's output ceiling, the largest result the app produces.
const MAX_COPY_BYTES: u64 = 50 * 1024 * 1024;

#[tauri::command]
fn read_document_text(path: String) -> Result<String, String> {
    Ok(document_io::read(Path::new(&path), MAX_COPY_BYTES)?.text)
}

#[tauri::command]
fn write_document(path: String, text: String, expected_mtime_ms: u64) -> Result<u64, String> {
    document_io::write(Path::new(&path), &text, expected_mtime_ms)
}

#[cfg(desktop)]
fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Quit from the tray, asking first if a run is in flight.
///
/// The red close button already asks (it only hides the window, so leaving a
/// run going is the point). But once the window is hidden the tray is the
/// *only* way out, and that is exactly the state a background run sits in —
/// so quitting there silently discarded work the provider had already billed
/// for. Stop can't refund that either, but the user should get to decide.
#[cfg(desktop)]
fn quit_with_confirm(app: &AppHandle) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

    let active = app
        .state::<JobManager>()
        .list()
        .iter()
        .filter(|j| matches!(j.status.as_str(), "queued" | "working" | "processing"))
        .count();
    if active == 0 {
        app.exit(0);
        return;
    }

    // Clicking a menu-bar item doesn't activate the app, so an unparented
    // alert can open behind whatever is frontmost and Quit looks like a no-op.
    show_main_window(app);
    let handle = app.clone();
    app.dialog()
        .message(format!(
            "{active} file{} still processing. The provider has already been billed for them. \
             Backend conversions resume when you reopen Tool-Kit; direct ones do not.",
            if active == 1 { " is" } else { "s are" }
        ))
        .title("Quit Tool-Kit?")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Quit anyway".into(),
            "Keep working".into(),
        ))
        // `show` is non-blocking, which is what makes this safe to call from
        // the menu event on the main thread; `blocking_show` would deadlock.
        .show(move |quit| {
            if quit {
                handle.exit(0);
            }
        });
}

#[cfg(desktop)]
fn toggle_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let visible = w.is_visible().unwrap_or(false);
        let focused = w.is_focused().unwrap_or(false);
        if visible && focused {
            let _ = w.hide();
        } else {
            let _ = w.show();
            let _ = w.unminimize();
            let _ = w.set_focus();
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(JobManager::default())
        .manage(backend_host::BackendHost::new())
        .invoke_handler(tauri::generate_handler![
            secret_status,
            set_secret,
            get_settings,
            save_settings,
            list_jobs,
            scan_inputs,
            run_pipeline,
            stop_run,
            retry_job,
            reveal_path,
            read_text_file,
            read_document,
            read_document_text,
            write_document,
            quit_app,
            retry_failed,
            list_history,
            clear_history,
            suggested_workspace_path,
            inspect_workspace_path,
            setup_workspace,
            list_projects,
            conversion_service::service_request,
            conversion_service::download_conversion_markdown,
            backend_host::backend_status,
            backend_host::restart_backend,
            backend_host::open_backend_log
        ])
        .setup(|app| {
            // Opened once and held for the process. A database that can't be
            // opened degrades to "no history" rather than failing startup.
            app.manage(history::init(app.handle()));
            // Manual only. In Sidecar mode there is no port yet, and the
            // resume path fails a job on one refused connection, so recovery
            // waits for the first service that answers (see `backend_host`).
            // A broken override reports itself through `start` below, and
            // recovering against a backend we cannot name would be worse.
            if matches!(
                backend_host::deployment(app.handle()),
                Ok(backend_host::Deployment::Manual { .. })
            ) {
                jobs::recover_in_flight(app.handle().clone());
            }
            // Nothing waits on this: the sidecar comes up on its own task and
            // the window opens whether or not it made it.
            backend_host::start(app.handle());

            #[cfg(desktop)]
            {
                use tauri::menu::{Menu, MenuItem};
                use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

                let show_i = MenuItem::with_id(app, "show", "Show Tool-Kit", true, None::<&str>)?;
                let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&show_i, &quit_i])?;

                // The menu bar wants a monochrome template image, not the full
                // colour app icon — macOS then tints it for light/dark menu
                // bars and inverts it while the menu is open.
                let tray_icon =
                    tauri::image::Image::from_bytes(include_bytes!("../icons/tray@2x.png"));

                let mut tray = TrayIconBuilder::new()
                    .menu(&menu)
                    .icon_as_template(true)
                    .show_menu_on_left_click(false)
                    .on_menu_event(|app, event| match event.id.as_ref() {
                        "show" => show_main_window(app),
                        "quit" => quit_with_confirm(app),
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            toggle_main_window(tray.app_handle());
                        }
                    });
                match tray_icon {
                    Ok(icon) => tray = tray.icon(icon),
                    Err(e) => {
                        eprintln!("[tool-kit] tray icon failed to load ({e}); using the app icon");
                        if let Some(icon) = app.default_window_icon() {
                            tray = tray.icon(icon.clone());
                        }
                    }
                }
                tray.build(app)?;

                use tauri_plugin_global_shortcut::{
                    Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
                };
                // Not SUPER|SHIFT+V: that is macOS "Paste and Match Style",
                // which this would hijack system-wide in every app.
                let toggle = Shortcut::new(Some(Modifiers::ALT | Modifiers::SUPER), Code::KeyV);
                let toggle_for_handler = toggle;
                app.handle().plugin(
                    tauri_plugin_global_shortcut::Builder::new()
                        .with_handler(move |app, scut, event| {
                            if scut == &toggle_for_handler
                                && event.state() == ShortcutState::Pressed
                            {
                                toggle_main_window(app);
                            }
                        })
                        .build(),
                )?;
                if let Err(e) = app.global_shortcut().register(toggle) {
                    eprintln!("[tool-kit] could not register global shortcut: {e}");
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Not ExitRequested: it never fires on ⌘Q here, because the window
            // hides instead of being destroyed. This is the last point at which
            // the sidecar can be taken down with us.
            if let tauri::RunEvent::Exit = event {
                backend_host::stop_on_exit(app);
            }
            // Closing the window only hides it, so the Dock icon has to be a
            // way back in — otherwise the app looks dead but is still running.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                show_main_window(app);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}

#[cfg(test)]
mod scan_tests {
    use super::*;
    use std::fs;

    /// Build a temp tree and return its root.
    fn tree(name: &str, files: &[&str]) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("toolkit-scan-{name}"));
        let _ = fs::remove_dir_all(&root);
        for rel in files {
            let p = root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, b"x").unwrap();
        }
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn names(v: &[std::path::PathBuf]) -> Vec<String> {
        let mut n: Vec<String> = v
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        n.sort();
        n
    }

    /// The regression that shipped: a file whose folder is also the output
    /// folder must still be eligible. Results land beside their sources by
    /// default, so excluding the output folder excluded every input.
    #[test]
    fn file_in_the_output_folder_is_still_collected() {
        let root = tree("output-overlap", &["report.pdf"]);
        let input = root.join("report.pdf").to_string_lossy().to_string();
        let found = collect_input_files(&[input], JobType::Convert);
        assert_eq!(
            names(&found),
            vec!["report.pdf"],
            "input excluded by its own folder"
        );
    }

    #[test]
    fn folders_are_walked_recursively() {
        let root = tree(
            "recursive",
            &["a.pdf", "sub/b.pdf", "sub/deep/c.pdf", "sub/notes.txt"],
        );
        let found = collect_input_files(&[root.to_string_lossy().to_string()], JobType::Convert);
        assert_eq!(names(&found), vec!["a.pdf", "b.pdf", "c.pdf"]);
    }

    #[test]
    fn dot_directories_are_skipped_when_walking() {
        let root = tree(
            "hidden",
            &["keep.pdf", ".git/objects/junk.pdf", ".DS_Store"],
        );
        let found = collect_input_files(&[root.to_string_lossy().to_string()], JobType::Convert);
        assert_eq!(names(&found), vec!["keep.pdf"]);
    }

    #[test]
    fn a_file_and_its_folder_together_queue_it_once() {
        let root = tree("dedup", &["only.pdf"]);
        let inputs = vec![
            root.to_string_lossy().to_string(),
            root.join("only.pdf").to_string_lossy().to_string(),
        ];
        assert_eq!(collect_input_files(&inputs, JobType::Convert).len(), 1);
    }

    #[test]
    fn each_job_sees_only_its_own_extensions() {
        let root = tree("bytype", &["doc.pdf", "talk.mp3", "clip.mov", "sheet.xlsx"]);
        let inputs = vec![root.to_string_lossy().to_string()];
        assert_eq!(
            names(&collect_input_files(&inputs, JobType::Convert)),
            vec!["doc.pdf", "sheet.xlsx"]
        );
        assert_eq!(
            names(&collect_input_files(&inputs, JobType::Transcribe)),
            vec!["clip.mov", "talk.mp3"]
        );
    }

    #[test]
    fn extension_match_is_case_insensitive() {
        let root = tree("case", &["SCAN.PDF", "Audio.MP3"]);
        let inputs = vec![root.to_string_lossy().to_string()];
        assert_eq!(
            names(&collect_input_files(&inputs, JobType::Convert)),
            vec!["SCAN.PDF"]
        );
        assert_eq!(
            names(&collect_input_files(&inputs, JobType::Transcribe)),
            vec!["Audio.MP3"]
        );
    }

    #[test]
    fn only_html_formats_bypass_capabilities() {
        for name in ["a.html", "a.htm", "A.HTML"] {
            assert!(is_permanent_direct(Path::new(name)), "{name}");
        }
        // Images have a local engine now, so they must reach the capabilities
        // lookup instead of being billed to a remote provider on sight.
        for name in [
            "a.png", "a.jpg", "a.jpeg", "a.webp", "a.tiff", "a.tif", "a.gif", "a.bmp", "a.pdf",
            "a.docx", "a.epub", "a.xlsx", "a.pptx",
        ] {
            assert!(!is_permanent_direct(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn backend_routes_are_the_union_of_live_support_and_direct_convert() {
        let root = tree(
            "backend-capabilities-own-formats",
            &[
                "table.csv",
                "document.odt",
                "legacy.docx",
                "image.png",
                "movie.mp4",
                "program.exe",
                "already.md",
            ],
        );
        let inputs = [root.to_string_lossy().into_owned()];
        let candidates = collect_backend_candidates(&inputs);
        assert!(names(&candidates).contains(&"table.csv".into()));
        assert!(!names(&candidates).contains(&"already.md".into()));

        // A service with the Vision engine advertises image/png, so the image
        // belongs to the backend rather than to Datalab.
        let supported = HashSet::from([
            "text/csv".to_string(),
            "application/vnd.oasis.opendocument.text".to_string(),
            "image/png".to_string(),
        ]);
        let plan = route_conversion_candidates(candidates, &supported);
        assert_eq!(
            names(&plan.backend),
            vec!["document.odt", "image.png", "table.csv"]
        );
        assert_eq!(names(&plan.direct), vec!["legacy.docx"]);
        assert!(!names(&plan.backend).contains(&"movie.mp4".into()));
        assert!(!names(&plan.direct).contains(&"movie.mp4".into()));
        assert!(!names(&plan.backend).contains(&"program.exe".into()));
        assert!(!names(&plan.direct).contains(&"program.exe".into()));

        assert!(require_backend_capacity(plan, false).is_err());
        // No advertised image support — a Linux deployment, or a Mac below
        // macOS 26 — and the image still falls through to the direct route.
        let direct_only = route_conversion_candidates(
            vec![root.join("legacy.docx"), root.join("image.png")],
            &HashSet::new(),
        );
        assert_eq!(names(&direct_only.direct), vec!["image.png", "legacy.docx"]);
        assert!(require_backend_capacity(direct_only, false).is_ok());
    }

    #[test]
    fn a_new_run_identifies_every_retained_backend_context_for_cleanup() {
        let direct = Job::new(1, "/tmp/image.png".into(), "/tmp".into(), JobType::Convert);
        let backend = Job::new_backend(
            2,
            "/tmp/report.pdf".into(),
            "/tmp".into(),
            jobs::BackendContext::new(
                "http://127.0.0.1:8080".into(),
                "11111111-1111-4111-8111-111111111111".into(),
                "22222222-2222-4222-8222-222222222222".into(),
                None,
                settings::ConversionProfile::Standard,
                conversion_service::OcrOptions::default(),
            ),
        );
        assert_eq!(
            backend_inflight_keys(&[direct, backend]),
            ["22222222-2222-4222-8222-222222222222"]
        );
    }

    #[test]
    fn conversion_scan_metadata_reports_mime_and_reuse_per_file() {
        let root = tree("route-metadata", &["report.pdf", "image.png", "page.html"]);
        let files = collect_input_files(&[root.to_string_lossy().to_string()], JobType::Convert);
        let report = root.join("report.pdf").to_string_lossy().into_owned();
        let reuse = ReuseSummary {
            already_here: 1,
            reusable: 0,
            by_source: HashMap::from([(report.clone(), ReuseDisposition::AlreadyHere)]),
        };

        let scanned = describe_conversion_files(&files, &reuse);
        let by_name = |name: &str| {
            scanned
                .iter()
                .find(|file| file.source_path.ends_with(name))
                .unwrap()
        };
        assert_eq!(by_name("report.pdf").media_type, "application/pdf");
        assert_eq!(by_name("report.pdf").reuse, ReuseDisposition::AlreadyHere);
        assert_eq!(by_name("image.png").media_type, "image/png");
        assert_eq!(by_name("image.png").reuse, ReuseDisposition::Pending);
        assert_eq!(by_name("page.html").media_type, "text/html");
    }

    #[test]
    fn suggested_output_is_the_shared_parent_and_none_when_mixed() {
        let root = tree("suggest", &["one/a.pdf", "one/b.pdf", "two/c.pdf"]);
        let one = root.join("one");
        let same = vec![
            one.join("a.pdf").to_string_lossy().to_string(),
            one.join("b.pdf").to_string_lossy().to_string(),
        ];
        assert_eq!(
            suggested_output_dir(&same),
            Some(one.to_string_lossy().to_string())
        );

        // A dropped folder is its own answer.
        let folder = vec![one.to_string_lossy().to_string()];
        assert_eq!(
            suggested_output_dir(&folder),
            Some(one.to_string_lossy().to_string())
        );

        // Two different parents: don't guess.
        let mixed = vec![
            one.join("a.pdf").to_string_lossy().to_string(),
            root.join("two/c.pdf").to_string_lossy().to_string(),
        ];
        assert_eq!(suggested_output_dir(&mixed), None);
    }
}

#[cfg(test)]
mod already_text_tests {
    use super::*;
    use std::fs;

    #[test]
    fn text_files_are_counted_but_never_queued() {
        let root = std::env::temp_dir().join("toolkit-alreadytext");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sub")).unwrap();
        for rel in ["notes.md", "raw.txt", "deck.pdf", "sub/log.TXT"] {
            fs::write(root.join(rel), b"x").unwrap();
        }
        let inputs = vec![root.to_string_lossy().to_string()];
        // Already-text files never become jobs...
        assert_eq!(collect_input_files(&inputs, JobType::Convert).len(), 1);
        assert_eq!(collect_input_files(&inputs, JobType::Transcribe).len(), 0);
        // ...but are counted so the UI can explain the skip.
        assert_eq!(count_matching(&inputs, ALREADY_TEXT), 3);
    }
}

#[cfg(test)]
mod document_io_tests {
    use super::*;
    use std::fs;

    /// Seed a document and return its path.
    fn doc(name: &str, body: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("toolkit-doc-{name}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let path = root.join("note.md");
        fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn write_then_read_returns_the_saved_text() {
        let path = doc("roundtrip", "first\n");
        let opened = document_io::read(&path, MAX_PREVIEW_BYTES).unwrap();
        assert_eq!(opened.text, "first\n");

        let saved_mtime = document_io::write(&path, "second\n", opened.mtime_ms).unwrap();
        let reopened = document_io::read(&path, MAX_PREVIEW_BYTES).unwrap();
        assert_eq!(reopened.text, "second\n");
        assert_eq!(reopened.mtime_ms, saved_mtime);
    }

    /// The whole point of the mtime handshake: an edit made outside the app
    /// must not be silently replaced by the pane's stale copy.
    #[test]
    fn write_refuses_when_the_file_changed_on_disk() {
        let path = doc("stale", "original\n");
        let opened = document_io::read(&path, MAX_PREVIEW_BYTES).unwrap();

        let err = document_io::write(&path, "from the pane\n", opened.mtime_ms + 1).unwrap_err();
        assert!(err.contains("changed on disk"), "unexpected error: {err}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "original\n");
    }

    /// Overwriting a long document with a short one must leave the short one,
    /// not the short one followed by the tail of the old bytes — the failure a
    /// truncate-in-place write produces when it stops halfway.
    #[test]
    fn overwriting_leaves_no_tail_of_the_previous_contents() {
        let long = "x".repeat(4096);
        let path = doc("overwrite", &long);
        let opened = document_io::read(&path, MAX_PREVIEW_BYTES).unwrap();

        document_io::write(&path, "short\n", opened.mtime_ms).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "short\n");

        let leftovers: Vec<String> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| Some(e.ok()?.file_name().to_string_lossy().to_string()))
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp file left behind: {leftovers:?}");
    }

    #[test]
    fn read_rejects_a_directory_and_an_oversized_file() {
        let path = doc("limits", "body\n");
        let dir = path.parent().unwrap();
        assert_eq!(
            document_io::read(dir, MAX_PREVIEW_BYTES).unwrap_err(),
            "Not a file"
        );
        assert!(document_io::read(&path, 2).is_err(), "size cap not applied");
    }
}
