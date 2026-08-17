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
    anthropic: bool,
}

#[tauri::command]
fn secret_status() -> SecretStatus {
    SecretStatus {
        datalab: secrets::has_key("datalab"),
        revai: secrets::has_key("revai"),
        anthropic: secrets::has_key("anthropic"),
    }
}

#[tauri::command]
fn set_secret(provider: String, value: String) -> Result<(), String> {
    match provider.as_str() {
        "datalab" | "revai" | "anthropic" => secrets::set_key(&provider, value.trim()),
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

#[tauri::command]
fn clear_jobs(state: State<JobManager>) {
    state.clear();
}

/// Expand the selected inputs (files and/or folders) into the concrete list of
/// files this job will process. Folders are scanned (non-recursive) for files
/// the job accepts; individual files are included if they match.
fn collect_input_files(inputs: &[String], jt: JobType) -> Vec<std::path::PathBuf> {
    let accepts = |p: &Path| -> bool {
        p.extension()
            .and_then(|e| e.to_str())
            .map(|e| jt.accepts(&e.to_lowercase()))
            .unwrap_or(false)
    };
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for input in inputs {
        let path = Path::new(input);
        if path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_file() && accepts(&p) {
                        files.push(p);
                    }
                }
            }
        } else if path.is_file() && accepts(path) {
            files.push(path.to_path_buf());
        }
    }
    files.sort();
    files.dedup();
    files
}

/// Count the files the given job would process across the selected inputs.
#[tauri::command]
fn scan_inputs(inputs: Vec<String>, job_type: String) -> Result<usize, String> {
    let jt = JobType::from_id(&job_type).ok_or("Unknown job type")?;
    Ok(collect_input_files(&inputs, jt).len())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RunResult {
    count: usize,
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

    let files = collect_input_files(&inputs, jt);
    if files.is_empty() {
        return Err(format!("No {} files in your selection", jt.label().to_lowercase()));
    }

    // Fresh slate per run.
    state.clear();
    for source in &files {
        let id = state.next_id();
        let job = Job::new(id, source.to_string_lossy().to_string(), output_dir.clone(), jt);
        state.insert(job.clone());
        let _ = app.emit("job-updated", job);
        jobs::run_job(app.clone(), id);
    }
    Ok(RunResult { count: files.len() })
}

#[tauri::command]
fn retry_job(app: AppHandle, state: State<JobManager>, id: u64) -> Result<(), String> {
    state
        .update(id, |j| {
            j.status = "queued".into();
            j.progress_note = "Queued".into();
            j.error = None;
        })
        .ok_or_else(|| "Job not found".to_string())?;
    jobs::run_job(app.clone(), id);
    Ok(())
}

#[tauri::command]
fn reveal_path(app: AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().reveal_item_in_dir(path).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_path(app: AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_path(path, None::<&str>).map_err(|e| e.to_string())
}

#[cfg(desktop)]
fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
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
            clear_jobs,
            scan_inputs,
            run_pipeline,
            retry_job,
            reveal_path,
            open_path
        ])
        .setup(|app| {
            #[cfg(desktop)]
            {
                use tauri::menu::{Menu, MenuItem};
                use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

                let show_i = MenuItem::with_id(app, "show", "Show Tool-Kit", true, None::<&str>)?;
                let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&show_i, &quit_i])?;

                let mut tray = TrayIconBuilder::new()
                    .menu(&menu)
                    .show_menu_on_left_click(false)
                    .on_menu_event(|app, event| match event.id.as_ref() {
                        "show" => show_main_window(app),
                        "quit" => app.exit(0),
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
                if let Some(icon) = app.default_window_icon() {
                    tray = tray.icon(icon.clone());
                }
                tray.build(app)?;

                use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
                let toggle = Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyV);
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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
