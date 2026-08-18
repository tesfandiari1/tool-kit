mod history;
mod jobs;
mod providers;
mod secrets;
mod settings;

#[cfg(test)]
mod live_smoke;

use serde::Serialize;
use std::path::Path;
use tauri::{AppHandle, Emitter, Manager, State};

use jobs::{Job, JobManager, JobType};
use settings::Settings;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SecretStatus {
    datalab: bool,
    revai: bool,
}

#[tauri::command]
fn secret_status() -> SecretStatus {
    SecretStatus {
        datalab: secrets::has_key("datalab"),
        revai: secrets::has_key("revai"),
    }
}

#[tauri::command]
fn set_secret(provider: String, value: String) -> Result<(), String> {
    match provider.as_str() {
        "datalab" | "revai" => secrets::set_key(&provider, value.trim()),
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
const ALREADY_TEXT: &[&str] = &["txt", "md", "markdown", "text", "rst", "org", "csv", "tsv", "json"];

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
#[tauri::command(async)]
fn scan_inputs(app: AppHandle, inputs: Vec<String>) -> Result<Scan, String> {
    let cfg = settings::load(&app);
    let convert = collect_input_files(&inputs, JobType::Convert);
    let transcribe = collect_input_files(&inputs, JobType::Transcribe);
    let (here_c, reuse_c) = split_reusable(&app, &convert, JobType::Convert, &cfg);
    let (here_t, reuse_t) = split_reusable(&app, &transcribe, JobType::Transcribe, &cfg);
    Ok(Scan {
        already_here_convert: here_c,
        already_here_transcribe: here_t,
        reusable_convert: reuse_c,
        reusable_transcribe: reuse_t,
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
) -> (usize, usize) {
    if files.is_empty() {
        return (0, 0);
    }
    let found = history::reusable(app, files, jt.id(), &jobs::output_format_for(jt, cfg));
    // With no output folder chosen yet nothing can be "already here", so every
    // reusable result counts as a copy — which is what will happen once the
    // user picks one.
    let Some(dir) = cfg.output_dir.as_deref() else {
        return (0, found.len());
    };
    let here = found.values().filter(|out| history::is_in_dir(out, dir)).count();
    (here, found.len() - here)
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
            if p.file_name().and_then(|s| s.to_str()).is_some_and(|s| s.starts_with('.')) {
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

#[tauri::command]
fn run_pipeline(
    app: AppHandle,
    state: State<JobManager>,
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
    let provider = jt.provider();
    if !secrets::has_key(provider.key_name()) {
        return Err(format!("Add your {} API key in Settings", provider.label()));
    }

    let mut files = collect_input_files(&inputs, jt);
    if files.is_empty() {
        return Err(format!("No {} files in your selection", jt.label().to_lowercase()));
    }

    // Loaded once and reused: the same config decides what gets skipped here
    // and what the run itself produces, so the two can't disagree.
    let cfg = settings::load(&app);

    // Three buckets. A file whose result is already in the output folder is
    // left alone; one whose result exists elsewhere is copied, because the
    // source is unchanged and the format matches, so paying the provider again
    // would buy bytes we already have; everything else runs.
    let mut to_copy: Vec<(std::path::PathBuf, String)> = Vec::new();
    let mut skipped = 0;
    if cfg.skip_already_done {
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

    // Fresh slate per run. Bumping the generation retires any task still alive
    // from a previous run so it can't keep spending credits or writing files.
    let generation = state.new_generation();
    // Set before any copy lands: `finish` reads this config to decide what
    // format to file the new history row under.
    state.set_run_config(cfg);
    state.clear();

    // Copies first, so the free results are on screen before the paid ones
    // start crawling.
    let mut copied = 0;
    for (source, existing) in &to_copy {
        let id = state.next_id();
        let job = Job::new(id, source.to_string_lossy().to_string(), output_dir.clone(), jt);
        state.insert(job.clone());
        let _ = app.emit("job-updated", job);
        if jobs::reuse_result(&app, id, existing) {
            copied += 1;
        }
    }

    for source in &files {
        let id = state.next_id();
        let job = Job::new(id, source.to_string_lossy().to_string(), output_dir.clone(), jt);
        state.insert(job.clone());
        let _ = app.emit("job-updated", job);
        jobs::run_job(app.clone(), id, generation);
    }
    Ok(RunResult { count: files.len(), skipped, copied })
}

/// Stop the current run: retire in-flight tasks and mark anything unfinished
/// as stopped. Work already submitted upstream still costs what it cost, but
/// nothing further is started and no more results are written.
#[tauri::command]
fn stop_run(app: AppHandle, state: State<JobManager>) -> Result<usize, String> {
    state.new_generation();
    let mut stopped = 0;
    for job in state.list() {
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
        jobs::run_job(app.clone(), *id, generation);
    }
    Ok(ids.len())
}

/// Quit for real. The close button only hides the window (this is a menu-bar
/// app), so "Stop and quit" needs an explicit way out.
#[tauri::command]
fn quit_app(app: AppHandle) {
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
            "{active} file{} still processing. The provider has already been billed for them, \
             and quitting now writes nothing to disk.",
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
            quit_app,
            retry_failed,
            list_history,
            clear_history
        ])
        .setup(|app| {
            // Opened once and held for the process. A database that can't be
            // opened degrades to "no history" rather than failing startup.
            app.manage(history::init(app.handle()));

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
                let tray_icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray@2x.png"));

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

                use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
                // Not SUPER|SHIFT+V: that is macOS "Paste and Match Style",
                // which this would hijack system-wide in every app.
                let toggle = Shortcut::new(Some(Modifiers::ALT | Modifiers::SUPER), Code::KeyV);
                let toggle_for_handler = toggle;
                app.handle().plugin(
                    tauri_plugin_global_shortcut::Builder::new()
                        .with_handler(move |app, scut, event| {
                            if scut == &toggle_for_handler && event.state() == ShortcutState::Pressed {
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
        assert_eq!(names(&found), vec!["report.pdf"], "input excluded by its own folder");
    }

    #[test]
    fn folders_are_walked_recursively() {
        let root = tree("recursive", &["a.pdf", "sub/b.pdf", "sub/deep/c.pdf", "sub/notes.txt"]);
        let found = collect_input_files(&[root.to_string_lossy().to_string()], JobType::Convert);
        assert_eq!(names(&found), vec!["a.pdf", "b.pdf", "c.pdf"]);
    }

    #[test]
    fn dot_directories_are_skipped_when_walking() {
        let root = tree("hidden", &["keep.pdf", ".git/objects/junk.pdf", ".DS_Store"]);
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
        assert_eq!(names(&collect_input_files(&inputs, JobType::Convert)), vec!["doc.pdf", "sheet.xlsx"]);
        assert_eq!(names(&collect_input_files(&inputs, JobType::Transcribe)), vec!["clip.mov", "talk.mp3"]);
    }

    #[test]
    fn extension_match_is_case_insensitive() {
        let root = tree("case", &["SCAN.PDF", "Audio.MP3"]);
        let inputs = vec![root.to_string_lossy().to_string()];
        assert_eq!(names(&collect_input_files(&inputs, JobType::Convert)), vec!["SCAN.PDF"]);
        assert_eq!(names(&collect_input_files(&inputs, JobType::Transcribe)), vec!["Audio.MP3"]);
    }

    #[test]
    fn suggested_output_is_the_shared_parent_and_none_when_mixed() {
        let root = tree("suggest", &["one/a.pdf", "one/b.pdf", "two/c.pdf"]);
        let one = root.join("one");
        let same = vec![
            one.join("a.pdf").to_string_lossy().to_string(),
            one.join("b.pdf").to_string_lossy().to_string(),
        ];
        assert_eq!(suggested_output_dir(&same), Some(one.to_string_lossy().to_string()));

        // A dropped folder is its own answer.
        let folder = vec![one.to_string_lossy().to_string()];
        assert_eq!(suggested_output_dir(&folder), Some(one.to_string_lossy().to_string()));

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
