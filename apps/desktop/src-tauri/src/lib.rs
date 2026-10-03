mod backend_host;
mod conversion_service;
mod history;
mod jobs;
mod secrets;
mod settings;
mod tree;
mod workspace;

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use tauri::{AppHandle, Emitter, Manager, State};

use jobs::{Job, JobManager, JobType};
use settings::Settings;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SecretStatus {
    backend: bool,
}

#[tauri::command]
fn secret_status() -> SecretStatus {
    SecretStatus {
        backend: secrets::has_key("backend"),
    }
}

#[tauri::command]
fn set_secret(app: AppHandle, provider: String, value: String) -> Result<(), String> {
    match provider.as_str() {
        // The child validates against the token `ensure_token` mints at
        // launch, and nothing re-mints. Writing it here 401s every conversion.
        "backend" if backend_host::app_owns_backend(app) => Err(
            "Tool-Kit mints the backend token itself while it runs the conversion service".into(),
        ),
        "backend" => secrets::set_key(&provider, value.trim()),
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

/// Whether the picked folder is already a Tool-Kit workspace, for "adopt".
#[tauri::command]
fn inspect_workspace_path(path: String) -> bool {
    workspace::inspect_workspace_path(&path)
}

/// First-launch setup: create or adopt the folder and return its ids. Nothing
/// is persisted here: the gate binds the workspace at its last beat. A quit at
/// that beat leaves a seeded welcome file unread, so a retry reports it again.
#[tauri::command]
fn setup_workspace(app: AppHandle, path: String) -> Result<workspace::WorkspaceInfo, String> {
    let mut info = workspace::setup_workspace(&path)?;
    if info.welcome_path.is_none() {
        info.welcome_path = workspace::seeded_welcome(&path);
    }
    workspace::migrate_settings(&app);
    Ok(info)
}

/// Adopt the configured workspace on an ordinary launch. Reports a welcome file
/// only when this call wrote it, so an already-bound workspace still gets one.
#[tauri::command]
fn ensure_workspace(app: AppHandle) -> Result<Option<workspace::WorkspaceInfo>, String> {
    let Some(path) = settings::load(&app).workspace_path else {
        return Ok(None);
    };
    let info = workspace::adopt_workspace(&path)?;
    // After the folder rename, never before: the settings name that path.
    workspace::migrate_settings(&app);
    Ok(Some(info))
}

#[tauri::command]
fn list_projects(app: AppHandle) -> Result<Vec<workspace::ProjectSummary>, String> {
    workspace::list_projects(&app)
}

#[tauri::command]
fn create_project(app: AppHandle, title: String) -> Result<workspace::ProjectSummary, String> {
    workspace::create_project(&app, &title)
}

/// One directory level of the library, lazily. `rel` is workspace-relative.
/// On the blocking pool: a sync command runs inline on the AppKit main thread.
#[tauri::command]
async fn list_project_files(
    app: AppHandle,
    rel: String,
) -> Result<tree::DirListing, tree::ListError> {
    // Not `gone`: onboarding is still writing, and `gone` drops expandedPaths.
    let workspace = settings::load(&app)
        .workspace_path
        .ok_or_else(|| tree::ListError::transient("No workspace configured"))?;
    tauri::async_runtime::spawn_blocking(move || tree::list(Path::new(&workspace), &rel))
        .await
        .map_err(|e| tree::ListError::transient(e.to_string()))?
}

/// One folder the webview has already listed, and the mtime it holds for it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct KnownDir {
    rel: String,
    modified_ms: u64,
}

/// Which of these folders moved since the webview last listed them. One `stat`
/// each, not the `read_dir` plus per-entry `stat` a listing costs. A folder that
/// cannot be stat'd reads as changed.
#[tauri::command]
async fn changed_project_dirs(app: AppHandle, known: Vec<KnownDir>) -> Result<Vec<String>, String> {
    let workspace = settings::load(&app)
        .workspace_path
        .ok_or_else(|| "No workspace configured".to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let root = Path::new(&workspace);
        known
            .into_iter()
            .filter(|dir| tree::modified(root, &dir.rel) != Some(dir.modified_ms))
            .map(|dir| dir.rel)
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

/// How deep a dropped folder is walked, so a dropped home folder cannot
/// wander forever.
const MAX_SCAN_DEPTH: usize = 8;

/// Expand the selected inputs into the concrete file list this job processes.
/// Never excludes the output folder: results land beside their sources.
fn collect_input_files(inputs: &[String], jt: JobType) -> Vec<std::path::PathBuf> {
    let accepts = |p: &Path| -> bool {
        p.extension()
            .and_then(|e| e.to_str())
            .map(|e| jt.accepts(&e.to_lowercase()))
            .unwrap_or(false)
    };
    collect_files_where(inputs, &accepts)
}

fn collect_files_where(
    inputs: &[String],
    accepts: &dyn Fn(&Path) -> bool,
) -> Vec<std::path::PathBuf> {
    fn walk(
        dir: &Path,
        depth: usize,
        files: &mut Vec<std::path::PathBuf>,
        accepts: &dyn Fn(&Path) -> bool,
    ) {
        if depth > MAX_SCAN_DEPTH {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            // Dotfiles only while walking in. A dropped dot-path is still honoured.
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let p = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => walk(&p, depth + 1, files, accepts),
                Ok(t) if t.is_file() && accepts(&p) => files.push(p),
                _ => {}
            }
        }
    }

    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for input in inputs {
        let path = Path::new(input);
        if path.is_dir() {
            walk(path, 0, &mut files, &accepts);
        } else if path.is_file() && accepts(path) {
            files.push(path.to_path_buf());
        }
    }
    // Canonicalize so a file reached twice, itself and its folder, queues once.
    files.sort();
    let mut seen = std::collections::HashSet::new();
    files.retain(|p| seen.insert(p.canonicalize().unwrap_or_else(|_| p.clone())));
    files
}

/// Formats that are already plain text. Counted, never converted. The
/// tree reads the same list to decide which rows open in the document pane.
pub(crate) const ALREADY_TEXT: &[&str] = &[
    "txt", "md", "markdown", "text", "rst", "org", "csv", "tsv", "json",
];

pub(crate) fn media_type(path: &Path) -> String {
    // mime_guess answers audio/m4a, which the contract does not list, so an
    // m4a upload is refused 415.
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("m4a"))
    {
        return "audio/mp4".to_string();
    }
    mime_guess::from_path(path)
        .first_or_octet_stream()
        .essence_str()
        .to_ascii_lowercase()
}

/// The candidates the live service can convert. Anything else is dropped:
/// nothing leaves the Mac.
fn route_conversion_candidates(
    candidates: Vec<std::path::PathBuf>,
    supported: &HashSet<String>,
) -> Vec<std::path::PathBuf> {
    candidates
        .into_iter()
        .filter(|file| supported.contains(&media_type(file)))
        .collect()
}

/// The engine options a new submission carries. A speaker count rides only on
/// a recording, and one outside the contract's 1 to 20 is dropped so the
/// diarizer guesses, rather than the service refusing every file with a 422.
fn submission_options(cfg: &Settings, jt: JobType) -> conversion_service::OcrOptions {
    conversion_service::OcrOptions {
        language_correction: cfg.language_correction,
        custom_words: cfg.custom_words.clone(),
        speaker_count: cfg
            .speaker_count
            .filter(|count| jt == JobType::Transcribe && (1..=20).contains(count)),
    }
}

/// Named because `convert_one` matches on it to build a reason code.
const BACKEND_NOT_ACCEPTING: &str = "The local conversion service is not accepting jobs";

fn require_backend_capacity(
    plan: Vec<std::path::PathBuf>,
    accepting_jobs: bool,
) -> Result<Vec<std::path::PathBuf>, String> {
    if !accepting_jobs && !plan.is_empty() {
        Err(BACKEND_NOT_ACCEPTING.into())
    } else {
        Ok(plan)
    }
}

/// `origin` is the host's answer, so a service that is not running reaches the
/// one rule below that decides whether the outage matters.
async fn plan_backend_conversion_files(
    inputs: &[String],
    origin: &Result<String, String>,
    jt: JobType,
) -> Result<Vec<std::path::PathBuf>, String> {
    let candidates = collect_input_files(inputs, jt);
    // Nothing to ask about, so an outage cannot matter.
    if candidates.is_empty() {
        return Ok(candidates);
    }

    let capabilities = match origin {
        Ok(origin) => conversion_service::fetch_capabilities(origin)
            .await
            .map_err(|error| format!("The local conversion service is unavailable: {error}"))?,
        Err(error) => return Err(error.clone()),
    };
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

#[derive(Default)]
struct ReuseSummary {
    already_here: usize,
    reusable: usize,
}

/// How many matches a dropped folder lists before the well says "and N more".
const MAX_INPUT_CHILDREN: usize = 40;

/// One file a run would take, inside a dropped folder.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InputMatch {
    path: String,
    /// Relative to the dropped folder, so the well shows `slides/deck.pdf`.
    name: String,
    job: &'static str,
}

/// One dropped path, and what a run would take from it. Built here, because the
/// host owns the walk, the dotfile rule and the depth cap.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InputNode {
    path: String,
    name: String,
    is_dir: bool,
    /// The job this file feeds. Null for a folder and for a file neither takes.
    job: Option<&'static str>,
    /// Matches inside a dropped folder, capped.
    matches: Vec<InputMatch>,
    /// Matches past the cap.
    truncated: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Scan {
    /// Matching file count per job, so the UI can pick the job that fits.
    convert: usize,
    transcribe: usize,
    /// Transcripts already in the chosen output folder. Convert never reuses.
    already_here_transcribe: usize,
    /// Transcripts in another folder. Copied, not transcribed again.
    reusable_transcribe: usize,
    /// Files skipped because they are already text.
    already_text: usize,
    /// The folder to default the output to: the dropped folder, or a shared one.
    suggested_output: Option<String>,
    /// The selection as the drop well draws it, one node per dropped path.
    nodes: Vec<InputNode>,
}

/// The job a file feeds, read off its extension alone.
fn job_of(path: &Path) -> Option<JobType> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .and_then(|ext| tree::job_for(&ext.to_lowercase()))
}

/// Describe each dropped path for the drop well. One walk per input, and the
/// same walk the run makes, so the well and the run cannot disagree.
fn describe_inputs(inputs: &[String]) -> Vec<InputNode> {
    let takeable = |path: &Path| job_of(path).is_some();
    let name_of = |path: &Path| -> String {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned())
    };

    inputs
        .iter()
        .map(|input| {
            let path = Path::new(input);
            if !path.is_dir() {
                return InputNode {
                    name: name_of(path),
                    path: input.clone(),
                    is_dir: false,
                    job: job_of(path).map(|jt| jt.id()),
                    matches: Vec::new(),
                    truncated: 0,
                };
            }
            let found = collect_files_where(std::slice::from_ref(input), &takeable);
            let truncated = found.len().saturating_sub(MAX_INPUT_CHILDREN);
            let matches = found
                .iter()
                .take(MAX_INPUT_CHILDREN)
                .filter_map(|file| {
                    Some(InputMatch {
                        name: file
                            .strip_prefix(path)
                            .unwrap_or(file)
                            .to_string_lossy()
                            .into_owned(),
                        job: job_of(file)?.id(),
                        path: file.to_string_lossy().into_owned(),
                    })
                })
                .collect();
            InputNode {
                name: name_of(path),
                path: input.clone(),
                is_dir: true,
                job: None,
                matches,
                truncated,
            }
        })
        .collect()
}

/// One job's files for the scan. A host that cannot name an origin reads the
/// same as a service that cannot answer.
async fn scan_files(
    inputs: &[String],
    origin: &Result<String, String>,
    jt: JobType,
) -> Vec<std::path::PathBuf> {
    // The frontend's own probe owns the outage message. The candidate list
    // stands while the service cannot say what it takes.
    plan_backend_conversion_files(inputs, origin, jt)
        .await
        .unwrap_or_else(|_| collect_input_files(inputs, jt))
}

/// Count what each job would process, and how much is already done. `(async)`
/// on a sync fn moves three tree walks and a stat per match off the main thread.
#[tauri::command]
async fn scan_inputs(app: AppHandle, inputs: Vec<String>) -> Result<Scan, String> {
    let cfg = settings::load(&app);
    // A service still starting reads the same as one that cannot answer.
    let origin = backend_host::backend_origin(&app);
    let convert = scan_files(&inputs, &origin, JobType::Convert).await;
    let transcribe = scan_files(&inputs, &origin, JobType::Transcribe).await;
    // `run_pipeline` reuses transcripts alone, so a Convert file never reads as done.
    let transcribe_reuse = split_reusable(&app, &transcribe, &inputs, JobType::Transcribe, &cfg);
    Ok(Scan {
        already_here_transcribe: transcribe_reuse.already_here,
        reusable_transcribe: transcribe_reuse.reusable,
        convert: convert.len(),
        transcribe: transcribe.len(),
        already_text: count_matching(&inputs, ALREADY_TEXT),
        suggested_output: suggested_output_dir(&inputs),
        nodes: describe_inputs(&inputs),
    })
}

/// Where a run writes, and the only answer to that question. The scan judges
/// "already in this folder" against it, so a second answer would report one
/// folder and write to another. `output_dir` is the workspace-less case alone.
fn output_dir_for(cfg: &Settings) -> Option<String> {
    if let (Some(workspace), Some(project)) = (&cfg.workspace_path, &cfg.active_project_path) {
        // The tree's own check, so a hand-edited settings.json cannot escape.
        if let Ok(dir) = tree::resolve(Path::new(workspace), project) {
            // Never the legacy folder: it sits where the tree sees nothing.
            return dir.is_dir().then(|| dir.to_string_lossy().into_owned());
        }
    }
    cfg.output_dir.clone()
}

/// Why a run has nowhere to write. A bound project means no folder picker.
fn no_destination_message(cfg: &Settings) -> String {
    if cfg.workspace_path.is_some() && cfg.active_project_path.is_some() {
        "That project folder is not there any more. Pick another project.".into()
    } else {
        "Choose an output folder first".into()
    }
}

/// Where **this file's** result goes. Beside the source inside the workspace,
/// which keeps the tree's pairing rule true. Outside it, the one destination,
/// where a file from a dropped folder keeps that folder's shape.
fn output_dir_for_source(source: &Path, inputs: &[String], cfg: &Settings) -> Option<String> {
    if let (Some(workspace), Some(parent)) = (&cfg.workspace_path, source.parent()) {
        if parent.starts_with(workspace) && parent.is_dir() {
            return Some(parent.to_string_lossy().into_owned());
        }
    }
    let filing = output_dir_for(cfg)?;
    // Into a project only. Without one the folder defaults to the dropped
    // folder itself, and mirroring would nest it inside itself.
    if cfg.workspace_path.is_none() || cfg.active_project_path.is_none() {
        return Some(filing);
    }
    let dropped = inputs
        .iter()
        .map(Path::new)
        .find(|root| root.is_dir() && source.starts_with(root));
    Some(match dropped {
        Some(root) => mirrored_dir(Path::new(&filing), root, source)
            .to_string_lossy()
            .into_owned(),
        None => filing,
    })
}

/// The folder a file from a dropped folder writes to: the project, the dropped
/// folder's own name, then whatever sat between.
fn mirrored_dir(dir: &Path, root: &Path, source: &Path) -> std::path::PathBuf {
    let Some(folder) = root.file_name() else {
        return dir.to_path_buf();
    };
    let inner = source
        .strip_prefix(root)
        .ok()
        .and_then(|rel| rel.parent())
        .filter(|rel| !rel.as_os_str().is_empty());
    match inner {
        Some(rel) => dir.join(folder).join(rel),
        None => dir.join(folder),
    }
}

/// Split the files needing no conversion into (already in the output folder,
/// copyable from elsewhere). The first does nothing, the second copies.
fn split_reusable(
    app: &AppHandle,
    files: &[std::path::PathBuf],
    inputs: &[String],
    jt: JobType,
    cfg: &Settings,
) -> ReuseSummary {
    if files.is_empty() {
        return ReuseSummary::default();
    }
    let found = history::reusable(app, files, jt.id(), jobs::output_format_for(jt));
    let mut summary = ReuseSummary::default();
    for (source, output) in found {
        // Per source, not per run: a dropped folder keeps its shape.
        if output_dir_for_source(Path::new(&source), inputs, cfg)
            .as_deref()
            .is_some_and(|dir| history::is_in_dir(&output, dir))
        {
            summary.already_here += 1;
        } else {
            summary.reusable += 1;
        }
    }
    summary
}

/// Newest first. `query` filters on file name or folder. Empty means all.
#[tauri::command]
fn list_history(app: AppHandle, query: String, limit: u32) -> Vec<history::Entry> {
    history::list(&app, &query, limit)
}

#[tauri::command]
fn clear_history(app: AppHandle) -> Result<(), String> {
    history::clear(&app)
}

/// Count files whose extension is in `exts`, walking folders as the run does.
fn count_matching(inputs: &[String], exts: &[&str]) -> usize {
    let has_ext = |p: &Path| {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| exts.contains(&e.to_lowercase().as_str()))
    };
    collect_files_where(inputs, &has_ext).len()
}

/// Where results land by default: alongside the input, when the inputs agree.
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
    job_type: String,
) -> Result<RunResult, String> {
    let jt = JobType::from_id(&job_type).ok_or("Unknown job type")?;
    if inputs.is_empty() {
        return Err("Choose at least one file or folder".into());
    }
    // Loaded once: one config decides collection, routing, reuse and output.
    let cfg = settings::load(&app);
    // Derived here, not passed in, so the counts and the writes name one folder.
    // Only a guard that a destination exists: each result goes beside its source.
    let filing_dir = output_dir_for(&cfg).ok_or_else(|| no_destination_message(&cfg))?;
    if !Path::new(&filing_dir).is_dir() {
        return Err(no_destination_message(&cfg));
    }
    let output_dir_of = |source: &Path| -> String {
        output_dir_for_source(source, &inputs, &cfg).unwrap_or_else(|| filing_dir.clone())
    };
    // Read once. In Sidecar mode the origin is this launch's own port.
    let origin = backend_host::backend_origin(&app);
    let mut files = plan_backend_conversion_files(&inputs, &origin, jt).await?;
    // Empty only when the plan sends nothing to the service.
    let backend_origin = origin.unwrap_or_default();
    if files.is_empty() {
        return Err(format!("No {} files in your selection", jt.id()));
    }

    // Three buckets: already here is left alone, elsewhere is copied, rest runs.
    let mut to_copy: Vec<(std::path::PathBuf, String)> = Vec::new();
    let mut skipped = 0;
    // Transcripts alone: a service conversion never stands in for another.
    if cfg.skip_already_done && jt == JobType::Transcribe {
        let found = history::reusable(&app, &files, jt.id(), jobs::output_format_for(jt));
        if !found.is_empty() {
            files.retain(|p| match found.get(p.to_string_lossy().as_ref()) {
                None => true,
                Some(existing) if history::is_in_dir(existing, &output_dir_of(p)) => {
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
    // Say so rather than starting an empty run that reads as a bug.
    if files.is_empty() && to_copy.is_empty() {
        return Err(format!(
            "Already done — all {skipped} file{} already have a result beside them. \
             Turn off “Skip files already done” in Settings to run them again.",
            if skipped == 1 { "" } else { "s" }
        ));
    }

    // Read once for the run, or a half-written override splits it across two
    // recorded origins.
    let mut ledger_origin = String::new();
    if !files.is_empty() {
        // Through the accessor: Sidecar mode mints this token itself.
        backend_host::backend_token(&app)?;
        ledger_origin = backend_host::ledger_origin(&backend_host::deployment(&app)?).to_string();
    }

    // A dropped folder's results nest, and the nest may not exist yet. A
    // failure here surfaces as that job's write error.
    for source in files.iter().chain(to_copy.iter().map(|(source, _)| source)) {
        let _ = std::fs::create_dir_all(output_dir_of(source));
    }

    // Fresh slate: the new generation retires any task still alive.
    let generation = state.new_generation();
    delete_backend_inflight(&app, &state.list());
    state.clear();

    // Copies first, so the free results land before the conversions start.
    let mut copied = 0;
    for (source, existing) in &to_copy {
        let id = state.next_id();
        let job = Job::new(
            id,
            source.to_string_lossy().to_string(),
            output_dir_of(source),
            jt,
        );
        state.insert(job.clone());
        let _ = app.emit("job-updated", job);
        if jobs::reuse_result(&app, id, generation, existing) {
            copied += 1;
        }
    }

    let client_run_id = uuid::Uuid::new_v4().to_string();
    // From the run snapshot: the service folds OCR into the replay key.
    let ocr = submission_options(&cfg, jt);
    let ocr_custom_words = ocr.custom_words_wire();
    for source in &files {
        let id = state.next_id();
        let source_path = source.to_string_lossy().into_owned();
        let output_dir = output_dir_of(source);
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
                backend_url: &ledger_origin,
                client_run_id: &client_run_id,
                idempotency_key: &idempotency_key,
                conversion_profile: settings::ConversionProfile::LocalOnly.id(),
                job_type: jt.id(),
                ocr_language_correction: ocr.language_correction,
                ocr_custom_words: &ocr_custom_words,
                speaker_count: ocr.speaker_count,
            },
        );
        let job = Job::new_backend(
            id,
            source_path,
            output_dir,
            jt,
            jobs::BackendContext::new(
                backend_origin.clone(),
                client_run_id.clone(),
                idempotency_key,
                None,
                settings::ConversionProfile::LocalOnly,
                ocr.clone(),
            ),
        );
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConvertOneOutcome {
    /// "queued" | "copied" | "blocked"
    kind: String,
    /// Set when kind == "blocked": "not_convertible" | "run_in_progress" |
    /// "backend_unavailable" | "backend_not_accepting".
    reason: Option<String>,
    /// The sentence the frontend shows verbatim.
    message: Option<String>,
    /// The result file, for kind "copied".
    path: Option<String>,
}

impl ConvertOneOutcome {
    fn queued() -> Self {
        Self {
            kind: "queued".into(),
            reason: None,
            message: None,
            path: None,
        }
    }

    fn copied(message: impl Into<String>, path: String) -> Self {
        Self {
            kind: "copied".into(),
            reason: None,
            message: Some(message.into()),
            path: Some(path),
        }
    }

    fn blocked(reason: &str, message: impl Into<String>) -> Self {
        Self {
            kind: "blocked".into(),
            reason: Some(reason.into()),
            message: Some(message.into()),
            path: None,
        }
    }
}

/// Whether `convert_one` may still append, given the generation it read before
/// its preflight.
fn queue_is_free(state: &JobManager, since: u64) -> bool {
    state.generation() == since && !state.list().iter().any(Job::is_active)
}

/// What a moved drop left behind: the paths to stage, and a line per path that
/// stayed where it was.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MoveOutcome {
    staged: Vec<String>,
    failed: Vec<String>,
}

/// Move each dropped path, file or folder, into a project. A rename, never a
/// copy. A path that cannot move is staged where it is, so a drop never
/// vanishes, and the history follows each moved file so it is not converted
/// twice.
#[tauri::command(async)]
fn move_into_project(
    app: AppHandle,
    inputs: Vec<String>,
    project_rel: String,
) -> Result<MoveOutcome, String> {
    let cfg = settings::load(&app);
    let workspace = cfg
        .workspace_path
        .clone()
        .ok_or_else(|| "No workspace configured".to_string())?;
    let dir = tree::resolve(Path::new(&workspace), &project_rel)?;
    if !dir.is_dir() {
        return Err("That project folder is not there any more".into());
    }

    let mut staged = Vec::new();
    let mut failed = Vec::new();
    for input in inputs {
        let root = Path::new(&input);
        let Some(name) = root.file_name() else {
            staged.push(input);
            continue;
        };
        // Already in the library: it is filed.
        if root.starts_with(&workspace) {
            staged.push(input);
            continue;
        }
        let landing = dir.join(name);
        if landing.exists() {
            failed.push(format!(
                "{} is already in that project, so it stayed where it was",
                name.to_string_lossy()
            ));
            staged.push(input);
            continue;
        }
        // Keys taken before the move: `MovedFrom` canonicalizes, and a gone
        // path canonicalizes to itself.
        let files = collect_files_where(std::slice::from_ref(&input), &|p| job_of(p).is_some());
        let before: Vec<_> = files
            .iter()
            .map(|f| history::MovedFrom::snapshot(&f.to_string_lossy(), None))
            .collect();
        // ponytail: rename only, so another disk refuses (EXDEV). Copy then
        // delete if moving across disks turns out to matter.
        if let Err(e) = std::fs::rename(root, &landing) {
            failed.push(format!(
                "Could not move {}, so it stayed where it was: {e}",
                name.to_string_lossy()
            ));
            staged.push(input);
            continue;
        }
        for (file, from) in files.iter().zip(&before) {
            let moved = match file.strip_prefix(root) {
                Ok(rel) if !rel.as_os_str().is_empty() => landing.join(rel),
                _ => landing.clone(),
            };
            history::relocate(&app, from, &moved.to_string_lossy(), None);
        }
        staged.push(landing.to_string_lossy().into_owned());
    }
    Ok(MoveOutcome { staged, failed })
}

/// File one library file, and the result beside it, into another project. Both
/// move or neither does, or the source reads as unconverted and offers to
/// convert it again. A collision is refused, not numbered. Answers with the new relative path.
#[tauri::command(async)]
fn move_to_project(app: AppHandle, rel: String, project_rel: String) -> Result<String, String> {
    let state = app.state::<JobManager>();
    // The rule `convert_one` refuses on: a run writes beside the source it
    // recorded, so a move lands the result in the folder just left.
    if !queue_is_free(&state, state.generation()) {
        return Err("A run is going. Wait for it to finish, then move the file.".into());
    }

    let cfg = settings::load(&app);
    let workspace = cfg
        .workspace_path
        .clone()
        .ok_or_else(|| "No workspace configured".to_string())?;
    let root = Path::new(&workspace);

    let source = tree::resolve(root, &rel)?;
    if !source.is_file() {
        return Err("That file is not there any more".into());
    }
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "That file has no name".to_string())?
        .to_string();
    let dir = tree::resolve(root, &project_rel)?;
    if !dir.is_dir() {
        return Err("That project folder is not there any more".into());
    }
    let from_dir = source
        .parent()
        .ok_or_else(|| "That file has no folder".to_string())?;
    if history::same_dir(from_dir, &dir) {
        return Ok(rel);
    }

    let result = tree::paired_result(root, &rel);
    for path in [Some(&source), result.as_ref()].into_iter().flatten() {
        let Some(taken) = path.file_name() else {
            continue;
        };
        if dir.join(taken).exists() {
            return Err(format!(
                "{} is already in that project. Rename one of them first.",
                taken.to_string_lossy()
            ));
        }
    }

    let source_path = source.to_string_lossy().into_owned();
    let result_path = result.as_ref().map(|p| p.to_string_lossy().into_owned());
    // Canonicalized while both files are still where the history says they are.
    let before = history::MovedFrom::snapshot(&source_path, result_path.as_deref());

    let moved_source = dir.join(&name);
    std::fs::rename(&source, &moved_source).map_err(|e| format!("Could not move {name}: {e}"))?;

    let mut moved_result = None;
    if let Some(existing) = &result {
        let landing = existing
            .file_name()
            .map(|taken| dir.join(taken))
            .ok_or_else(|| "That result has no name".to_string())?;
        if let Err(e) = std::fs::rename(existing, &landing) {
            // Put the source back: a split pair offers to convert a done file.
            let _ = std::fs::rename(&moved_source, &source);
            return Err(format!("Could not move the result beside it: {e}"));
        }
        moved_result = Some(landing.to_string_lossy().into_owned());
    }

    history::relocate(
        &app,
        &before,
        &moved_source.to_string_lossy(),
        moved_result.as_deref(),
    );
    Ok(format!("{project_rel}/{name}"))
}

/// Convert one file from the library tree, into the folder it already sits in.
/// Modelled on `retry_job`, never `run_pipeline`: it appends one job instead of
/// clearing the queue. Every refusal is a verdict, because the tree renders it.
#[tauri::command]
async fn convert_one(
    app: AppHandle,
    state: State<'_, JobManager>,
    rel: String,
) -> Result<ConvertOneOutcome, String> {
    let cfg = settings::load(&app);
    let workspace = cfg
        .workspace_path
        .clone()
        .ok_or_else(|| "No workspace configured".to_string())?;
    let source = tree::resolve(Path::new(&workspace), &rel)?;
    if !source.is_file() {
        return Ok(ConvertOneOutcome::blocked(
            "not_convertible",
            "That file is not there any more.",
        ));
    }
    let Some(jt) = job_of(&source) else {
        return Ok(ConvertOneOutcome::blocked(
            "not_convertible",
            "Tool-Kit has no job that takes this kind of file.",
        ));
    };

    // Read the generation first, for the re-check below.
    let generation = state.generation();
    if !queue_is_free(&state, generation) {
        return Ok(ConvertOneOutcome::blocked(
            "run_in_progress",
            "A run is already going. This file is staged in Run, ready for when it finishes.",
        ));
    }

    // Beside the source, which keeps the tree's pairing rule true.
    let output_dir = source
        .parent()
        .ok_or_else(|| "That file has no folder".to_string())?
        .to_string_lossy()
        .into_owned();
    let source_path = source.to_string_lossy().into_owned();
    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file")
        .to_string();
    let inputs = vec![source_path.clone()];

    // The same preflight a run does, mapped to reasons the tree can act on.
    let origin = backend_host::backend_origin(&app);
    let plan = match plan_backend_conversion_files(&inputs, &origin, jt).await {
        Ok(plan) => plan,
        Err(error) if error == BACKEND_NOT_ACCEPTING => {
            return Ok(ConvertOneOutcome::blocked("backend_not_accepting", error))
        }
        Err(error) => return Ok(ConvertOneOutcome::blocked("backend_unavailable", error)),
    };
    if plan.is_empty() {
        return Ok(ConvertOneOutcome::blocked(
            "not_convertible",
            "Nothing here converts this file.",
        ));
    }
    let backend_origin = origin.unwrap_or_default();
    // Through the accessor: Sidecar mode mints this token itself.
    if let Err(error) = backend_host::backend_token(&app) {
        return Ok(ConvertOneOutcome::blocked("backend_unavailable", error));
    }
    let deployment = match backend_host::deployment(&app) {
        Ok(deployment) => deployment,
        Err(error) => return Ok(ConvertOneOutcome::blocked("backend_unavailable", error)),
    };
    let ledger_origin = backend_host::ledger_origin(&deployment).to_string();

    // The preflight carries a round trip, so the refusal at the top is stale.
    if !queue_is_free(&state, generation) {
        return Ok(ConvertOneOutcome::blocked(
            "run_in_progress",
            "The queue moved while this file was being checked. It is staged in Run, ready for when that finishes.",
        ));
    }
    // The tree is blind to a result the user moved, so this gate still runs.
    if cfg.skip_already_done && jt == JobType::Transcribe {
        let found = history::reusable(
            &app,
            std::slice::from_ref(&source),
            jt.id(),
            jobs::output_format_for(jt),
        );
        if let Some(existing) = found.get(&source_path) {
            if history::is_in_dir(existing, &output_dir) {
                return Ok(ConvertOneOutcome::copied(
                    "That file already has a result in this folder.",
                    existing.clone(),
                ));
            }
            let id = state.next_id();
            let job = Job::new(id, source_path, output_dir, jt);
            state.insert(job.clone());
            let _ = app.emit("job-updated", job);
            // A failed copy reports itself on the row, which offers Retry.
            let written = jobs::reuse_result(&app, id, generation, existing)
                .then(|| state.get(id).and_then(|job| job.output_path))
                .flatten();
            return Ok(match written {
                Some(path) => {
                    ConvertOneOutcome::copied("Copied a result from an earlier run.", path)
                }
                None => ConvertOneOutcome::queued(),
            });
        }
    }

    let id = state.next_id();
    let client_run_id = uuid::Uuid::new_v4().to_string();
    let idempotency_key = uuid::Uuid::new_v4().to_string();
    let ocr = submission_options(&cfg, jt);
    history::upsert_in_flight(
        &app,
        &history::NewInFlight {
            source_path: &source_path,
            file_name: &file_name,
            output_dir: &output_dir,
            backend_url: &ledger_origin,
            client_run_id: &client_run_id,
            idempotency_key: &idempotency_key,
            conversion_profile: settings::ConversionProfile::LocalOnly.id(),
            job_type: jt.id(),
            ocr_language_correction: ocr.language_correction,
            ocr_custom_words: &ocr.custom_words_wire(),
            speaker_count: ocr.speaker_count,
        },
    );
    let job = Job::new_backend(
        id,
        source_path,
        output_dir,
        jt,
        jobs::BackendContext::new(
            backend_origin,
            client_run_id,
            idempotency_key,
            None,
            settings::ConversionProfile::LocalOnly,
            ocr,
        ),
    );
    state.insert(job.clone());
    let _ = app.emit("job-updated", job);
    jobs::run_job(app.clone(), id, generation);
    Ok(ConvertOneOutcome::queued())
}

/// Stop the run.
#[tauri::command]
fn stop_run(app: AppHandle, state: State<JobManager>) -> Result<usize, String> {
    state.new_generation();
    let mut stopped = 0;
    let jobs = state.list();
    delete_backend_inflight(&app, &jobs);
    for job in jobs {
        if job.is_active() {
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

/// Put a row back in the queue, for Retry and Retry all alike.
fn requeue(job: &mut Job) {
    job.status = "queued".into();
    job.progress_note = "Queued".into();
    job.error = None;
    job.started_at = None;
}

#[tauri::command]
fn retry_job(app: AppHandle, state: State<JobManager>, id: u64) -> Result<(), String> {
    // Retrying joins the current run, so a later Stop cancels it too.
    let generation = state.generation();
    let updated = state
        .update(id, requeue)
        .ok_or_else(|| "Job not found".to_string())?;
    // Emit now: a permit can take minutes, and a failed row invites a click.
    let _ = app.emit("job-updated", updated);
    jobs::restore_in_flight(&app, id);
    jobs::run_job(app.clone(), id, generation);
    Ok(())
}

/// Re-queue every failed job. An upstream blip can fail a handful of a batch.
#[tauri::command]
fn retry_failed(app: AppHandle, state: State<JobManager>) -> Result<usize, String> {
    let generation = state.generation();
    let mut ids = Vec::new();
    for job in state.list() {
        if job.status == "failed" {
            if let Some(updated) = state.update(job.id, requeue) {
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

/// Quit for real. The close button only hides the window (menu-bar app).
#[tauri::command]
async fn quit_app(app: AppHandle) {
    // Not `RunEvent::Exit`: a wait inside applicationWillTerminate reads as a
    // hang.
    backend_host::stop(&app, backend_host::STOP_BUDGET).await;
    app.exit(0);
}

/// Show something in Finder. `reveal_item_in_dir` opens the level above a
/// folder, so folders are opened directly.
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

/// Read a result file into the viewer. Capped, and UTF-8 only.
pub(crate) const MAX_PREVIEW_BYTES: u64 = 2 * 1024 * 1024;

/// Reading and writing a document. Free of Tauri types, so the size cap, the
/// UTF-8 rule and the mtime check are unit-testable.
mod document_io {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::time::UNIX_EPOCH;

    #[derive(Debug, serde::Serialize)]
    #[serde(rename_all = "camelCase")]
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

    /// A hidden sibling, so the rename stays on one filesystem and stays
    /// atomic.
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

    /// Save, refusing when the file changed since it was read. Temp file then
    /// rename: `fs::write` truncates, so a crash leaves a half-length document.
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

#[tauri::command]
fn read_document(path: String) -> Result<document_io::Document, String> {
    document_io::read(Path::new(&path), MAX_PREVIEW_BYTES)
}

/// Copy's own ceiling: a result too large to edit is still worth copying.
const MAX_COPY_BYTES: u64 = 50 * 1024 * 1024;

#[tauri::command]
fn read_document_text(path: String) -> Result<String, String> {
    Ok(document_io::read(Path::new(&path), MAX_COPY_BYTES)?.text)
}

#[tauri::command]
fn write_document(path: String, text: String, expected_mtime_ms: u64) -> Result<u64, String> {
    document_io::write(Path::new(&path), &text, expected_mtime_ms)
}

fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Quit from the tray, asking first if a run is in flight. Once the window is
/// hidden the tray is the only way out.
fn quit_with_confirm(app: &AppHandle) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

    let active = app
        .state::<JobManager>()
        .list()
        .iter()
        .filter(|j| j.is_active())
        .count();
    if active == 0 {
        app.exit(0);
        return;
    }

    // A menu-bar click does not activate the app, so an unparented alert opens
    // behind whatever is frontmost.
    show_main_window(app);
    let handle = app.clone();
    app.dialog()
        .message(format!(
            "{active} file{} still processing. They resume when you reopen Tool-Kit.",
            if active == 1 { " is" } else { "s are" }
        ))
        .title("Quit Tool-Kit?")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Quit anyway".into(),
            "Keep working".into(),
        ))
        // `show` is non-blocking. `blocking_show` deadlocks the main thread.
        .show(move |quit| {
            if quit {
                handle.exit(0);
            }
        });
}

fn toggle_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let visible = w.is_visible().unwrap_or(false);
        let focused = w.is_focused().unwrap_or(false);
        if visible && focused {
            let _ = w.hide();
        } else {
            show_main_window(app);
        }
    }
}

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
            convert_one,
            move_to_project,
            move_into_project,
            stop_run,
            retry_job,
            reveal_path,
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
            ensure_workspace,
            list_projects,
            create_project,
            list_project_files,
            changed_project_dirs,
            conversion_service::service_request,
            backend_host::app_owns_backend
        ])
        .setup(|app| {
            // Opened once. A database that cannot open degrades to no history.
            app.manage(history::init(app.handle()));
            // Manual only. Sidecar mode has no port yet, and the resume path
            // fails a job on one refused connection.
            if matches!(
                backend_host::deployment(app.handle()),
                Ok(backend_host::Deployment::Manual { .. })
            ) {
                jobs::recover_in_flight(app.handle().clone());
            }
            // Nothing waits on this: the window opens whether or not it starts.
            backend_host::start(app.handle());

            // The window opens hidden and the frontend shows it once placed.
            // Reveal it anyway if it never gets there. `show` is idempotent.
            if let Some(w) = app.get_webview_window("main") {
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    let _ = w.show();
                });
            }

            {
                use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
                use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

                let show_i = MenuItem::with_id(app, "show", "Show Tool-Kit", true, None::<&str>)?;
                let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&show_i, &quit_i])?;

                // A template image, not the colour app icon: macOS tints it.
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
                // Not SUPER|SHIFT+V: macOS "Paste and Match Style", system-wide.
                let toggle = Shortcut::new(Some(Modifiers::ALT | Modifiers::SUPER), Code::KeyV);
                app.handle().plugin(
                    tauri_plugin_global_shortcut::Builder::new()
                        .with_handler(move |app, scut, event| {
                            if scut == &toggle && event.state() == ShortcutState::Pressed {
                                toggle_main_window(app);
                            }
                        })
                        .build(),
                )?;
                if let Err(e) = app.global_shortcut().register(toggle) {
                    eprintln!("[tool-kit] could not register global shortcut: {e}");
                }

                // Spliced, never `set_menu`: a fresh menu drops Edit, View,
                // Window and Help. Index 2 is the native slot.
                let settings_i = MenuItem::with_id(
                    app,
                    "toolkit:settings",
                    "Settings…",
                    true,
                    Some("CmdOrCtrl+Comma"),
                )?;
                let separator = PredefinedMenuItem::separator(app)?;
                if let Some(app_menu) = app
                    .menu()
                    .map(|menu| menu.items())
                    .transpose()?
                    .and_then(|items| items.first().and_then(|item| item.as_submenu()).cloned())
                {
                    app_menu.insert(&settings_i, 2)?;
                    app_menu.insert(&separator, 3)?;
                }
                // Prefixed: the tray's `on_menu_event` is a global listener
                // matching the bare "show" and "quit".
                app.on_menu_event(|app, event| {
                    if event.id.as_ref() == "toolkit:settings" {
                        // The window may be hidden. A wait here stalls AppKit.
                        show_main_window(app);
                        let _ = app.emit("open-settings", ());
                    }
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Not ExitRequested: ⌘Q hides the window and never fires it.
            if let tauri::RunEvent::Exit = event {
                backend_host::stop_on_exit(app);
            }
            // Closing only hides the window, so the Dock icon is the way back.
            if let tauri::RunEvent::Reopen { .. } = event {
                show_main_window(app);
            }
        });
}

#[cfg(test)]
mod scan_tests {
    use super::*;
    use std::fs;

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

    #[test]
    fn a_bound_workspace_sends_a_run_to_its_active_project() {
        let root = tree("outdir", &["Inbox/keep.md", "Acme/keep.md"]);
        let cfg = Settings {
            workspace_path: Some(root.to_string_lossy().into_owned()),
            active_project_path: Some("Acme".into()),
            // Left over from before the library existed. Never read again.
            output_dir: Some("/Users/someone/Desktop".into()),
            ..Settings::default()
        };

        assert_eq!(
            output_dir_for(&cfg),
            Some(root.join("Acme").to_string_lossy().into_owned())
        );
    }

    /// A dropped folder keeps its shape, so two files in one run have two
    /// destinations. The project root leaves the nested one reading unconverted.
    #[test]
    fn a_result_lands_beside_its_source_not_at_the_project_root() {
        let root = tree("outdir-nested", &["Acme/deck.pdf", "Acme/slides/q3.pdf"]);
        let cfg = Settings {
            workspace_path: Some(root.to_string_lossy().into_owned()),
            active_project_path: Some("Acme".into()),
            output_dir: Some("/Users/someone/Desktop".into()),
            ..Settings::default()
        };

        assert_eq!(
            output_dir_for_source(&root.join("Acme/deck.pdf"), &[], &cfg),
            Some(root.join("Acme").to_string_lossy().into_owned())
        );
        assert_eq!(
            output_dir_for_source(&root.join("Acme/slides/q3.pdf"), &[], &cfg),
            Some(root.join("Acme/slides").to_string_lossy().into_owned())
        );
    }

    /// A file outside the library falls back to the one destination there is.
    #[test]
    fn a_source_outside_the_workspace_falls_back_to_the_filing_folder() {
        let root = tree("outdir-outside", &["Acme/keep.md"]);
        let cfg = Settings {
            workspace_path: Some(root.to_string_lossy().into_owned()),
            active_project_path: Some("Acme".into()),
            output_dir: Some("/Users/someone/Desktop".into()),
            ..Settings::default()
        };

        assert_eq!(
            output_dir_for_source(Path::new("/Users/someone/Desktop/loose.pdf"), &[], &cfg),
            Some(root.join("Acme").to_string_lossy().into_owned())
        );
    }

    /// A dropped folder from outside keeps its shape inside the project, and
    /// a loose file beside it still lands at the project root.
    #[test]
    fn a_dropped_folder_from_outside_keeps_its_shape() {
        let root = tree("outdir-mirror", &["Acme/keep.md"]);
        let cfg = Settings {
            workspace_path: Some(root.to_string_lossy().into_owned()),
            active_project_path: Some("Acme".into()),
            ..Settings::default()
        };
        let dropped = tree(
            "outdir-mirror-drop",
            &["slides/q3.pdf", "slides/appendix/charts.pdf"],
        );
        let inputs = [dropped.join("slides").to_string_lossy().into_owned()];
        let acme = root.join("Acme");

        assert_eq!(
            output_dir_for_source(&dropped.join("slides/q3.pdf"), &inputs, &cfg),
            Some(acme.join("slides").to_string_lossy().into_owned())
        );
        assert_eq!(
            output_dir_for_source(&dropped.join("slides/appendix/charts.pdf"), &inputs, &cfg),
            Some(acme.join("slides/appendix").to_string_lossy().into_owned())
        );
        assert_eq!(
            output_dir_for_source(Path::new("/Users/someone/Desktop/loose.pdf"), &inputs, &cfg),
            Some(acme.to_string_lossy().into_owned())
        );
    }

    #[test]
    fn without_a_workspace_the_chosen_folder_still_wins() {
        let cfg = Settings {
            output_dir: Some("/Users/someone/Desktop".into()),
            ..Settings::default()
        };

        assert_eq!(output_dir_for(&cfg), Some("/Users/someone/Desktop".into()));
    }

    #[test]
    fn a_project_that_climbs_out_of_the_workspace_is_refused() {
        let root = tree("outdir-escape", &["Inbox/keep.md"]);
        let cfg = Settings {
            workspace_path: Some(root.to_string_lossy().into_owned()),
            active_project_path: Some("../elsewhere".into()),
            output_dir: Some("/Users/someone/Desktop".into()),
            ..Settings::default()
        };

        // Falls back rather than writing outside the workspace.
        assert_eq!(output_dir_for(&cfg), Some("/Users/someone/Desktop".into()));
    }

    #[test]
    fn a_project_folder_that_is_gone_does_not_send_the_run_nowhere() {
        let root = tree("outdir-missing", &["Inbox/keep.md"]);
        let cfg = Settings {
            workspace_path: Some(root.to_string_lossy().into_owned()),
            active_project_path: Some("Deleted".into()),
            output_dir: None,
            ..Settings::default()
        };

        assert_eq!(output_dir_for(&cfg), None);
    }

    /// A pre-library `outputDir` writes the run where the tree cannot see it.
    #[test]
    fn a_project_folder_that_is_gone_refuses_rather_than_using_the_legacy_folder() {
        let root = tree("outdir-missing-legacy", &["Inbox/keep.md"]);
        let cfg = Settings {
            workspace_path: Some(root.to_string_lossy().into_owned()),
            active_project_path: Some("Deleted".into()),
            output_dir: Some("/Users/someone/Desktop".into()),
            ..Settings::default()
        };

        assert_eq!(output_dir_for(&cfg), None);
        assert_eq!(
            no_destination_message(&cfg),
            "That project folder is not there any more. Pick another project."
        );
    }

    fn names(v: &[std::path::PathBuf]) -> Vec<String> {
        let mut n: Vec<String> = v
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        n.sort();
        n
    }

    /// A file whose folder is also the output folder must stay eligible.
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

    /// The scan's fallback feeds the autodetect, which picks Transcribe only
    /// while its count is the larger one.
    #[test]
    fn convert_takes_service_formats_and_leaves_media_to_transcribe() {
        let root = tree(
            "scan-fallback-candidates",
            &[
                "document.odt",
                "legacy.docx",
                "talk.mp3",
                "movie.mp4",
                "already.md",
            ],
        );
        let inputs = [root.to_string_lossy().into_owned()];
        assert_eq!(
            names(&collect_input_files(&inputs, JobType::Convert)),
            vec!["document.odt", "legacy.docx"]
        );
    }

    /// Nothing leaves the Mac, so a file the service does not advertise is
    /// dropped rather than sent anywhere else.
    #[test]
    fn the_service_takes_only_what_it_advertises() {
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
        let candidates = collect_input_files(&inputs, JobType::Convert);
        assert!(names(&candidates).contains(&"table.csv".into()));
        assert!(!names(&candidates).contains(&"already.md".into()));

        // A service with the Vision engine advertises image/png.
        let supported = HashSet::from([
            "text/csv".to_string(),
            "application/vnd.oasis.opendocument.text".to_string(),
            "image/png".to_string(),
        ]);
        let plan = route_conversion_candidates(candidates, &supported);
        assert_eq!(names(&plan), vec!["document.odt", "image.png", "table.csv"]);
        assert!(require_backend_capacity(plan, false).is_err());

        // No advertised image support, so the image is dropped.
        let unsupported = route_conversion_candidates(
            vec![root.join("legacy.docx"), root.join("image.png")],
            &HashSet::new(),
        );
        assert!(unsupported.is_empty());
        assert!(require_backend_capacity(unsupported, false).is_ok());

        let recordings = tree("transcribe-capabilities", &["interview.m4a", "call.wav"]);
        let candidates = collect_input_files(
            &[recordings.to_string_lossy().into_owned()],
            JobType::Transcribe,
        );
        let supported = HashSet::from(["audio/mp4".to_string()]);
        let plan = route_conversion_candidates(candidates, &supported);
        assert_eq!(names(&plan), vec!["interview.m4a"]);
    }

    /// An outage stops every recording, and only an empty selection skips the ask.
    #[tokio::test]
    async fn an_outage_blocks_every_recording() {
        let root = tree("transcribe-outage", &["call.wav", "notes.md"]);
        let down: Result<String, String> = Err("The service is starting".into());
        let wav = [root.join("call.wav").to_string_lossy().into_owned()];
        assert!(
            plan_backend_conversion_files(&wav, &down, JobType::Transcribe)
                .await
                .is_err()
        );

        let text = [root.join("notes.md").to_string_lossy().into_owned()];
        assert!(
            plan_backend_conversion_files(&text, &down, JobType::Transcribe)
                .await
                .unwrap()
                .is_empty()
        );
    }

    /// The service 422s a count outside 1 to 20 on any file, and a document
    /// has no speakers to count.
    #[test]
    fn a_speaker_count_rides_only_on_a_recording_and_only_in_range() {
        let with = |count| Settings {
            speaker_count: Some(count),
            ..Settings::default()
        };
        assert_eq!(
            submission_options(&with(3), JobType::Transcribe).speaker_count,
            Some(3)
        );
        assert_eq!(
            submission_options(&with(3), JobType::Convert).speaker_count,
            None
        );
        assert_eq!(
            submission_options(&with(0), JobType::Transcribe).speaker_count,
            None
        );
        assert_eq!(
            submission_options(&with(21), JobType::Transcribe).speaker_count,
            None
        );
    }

    /// mime_guess answers audio/m4a, which the contract does not list, so an
    /// m4a upload would be refused 415.
    #[test]
    fn the_m4a_media_type_is_the_one_the_contract_names() {
        assert_eq!(media_type(Path::new("/tmp/interview.m4a")), "audio/mp4");
        assert_eq!(media_type(Path::new("/tmp/interview.M4A")), "audio/mp4");
        assert_eq!(media_type(Path::new("/tmp/call.wav")), "audio/wav");
        assert_eq!(media_type(Path::new("/tmp/clip.mp4")), "video/mp4");
        assert_eq!(media_type(Path::new("/tmp/report.pdf")), "application/pdf");
    }

    #[test]
    fn a_new_run_identifies_every_retained_backend_context_for_cleanup() {
        let copy = Job::new(1, "/tmp/image.png".into(), "/tmp".into(), JobType::Convert);
        let backend = Job::new_backend(
            2,
            "/tmp/report.pdf".into(),
            "/tmp".into(),
            JobType::Convert,
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
            backend_inflight_keys(&[copy, backend]),
            ["22222222-2222-4222-8222-222222222222"]
        );
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
        assert_eq!(collect_input_files(&inputs, JobType::Convert).len(), 1);
        assert_eq!(collect_input_files(&inputs, JobType::Transcribe).len(), 0);
        assert_eq!(count_matching(&inputs, ALREADY_TEXT), 3);
    }
}

#[cfg(test)]
mod convert_one_gate_tests {
    use super::*;
    use jobs::Job;

    fn queued_job(state: &JobManager) -> Job {
        Job::new(
            state.next_id(),
            "/tmp/deck.pdf".into(),
            "/tmp".into(),
            JobType::Convert,
        )
    }

    #[test]
    fn an_idle_queue_at_the_same_generation_is_free() {
        let state = JobManager::default();
        assert!(queue_is_free(&state, state.generation()));
    }

    #[test]
    fn a_job_still_in_flight_closes_the_queue() {
        let state = JobManager::default();
        let generation = state.generation();
        state.insert(queued_job(&state));
        assert!(!queue_is_free(&state, generation));
    }

    /// A run started inside the preflight has bumped the generation with no
    /// rows yet.
    #[test]
    fn a_run_started_during_the_preflight_closes_the_queue() {
        let state = JobManager::default();
        let generation = state.generation();
        state.new_generation();
        assert!(state.list().is_empty());
        assert!(!queue_is_free(&state, generation));
    }

    #[test]
    fn a_finished_job_leaves_the_queue_free() {
        let state = JobManager::default();
        let generation = state.generation();
        let mut job = queued_job(&state);
        job.status = "done".into();
        state.insert(job);
        assert!(queue_is_free(&state, generation));
    }
}

#[cfg(test)]
mod document_io_tests {
    use super::*;
    use std::fs;

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

    /// An edit made outside the app must not be replaced by a stale copy.
    #[test]
    fn write_refuses_when_the_file_changed_on_disk() {
        let path = doc("stale", "original\n");
        let opened = document_io::read(&path, MAX_PREVIEW_BYTES).unwrap();

        let err = document_io::write(&path, "from the pane\n", opened.mtime_ms + 1).unwrap_err();
        assert!(err.contains("changed on disk"), "unexpected error: {err}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "original\n");
    }

    /// A short overwrite must not leave the tail of the old bytes behind.
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
