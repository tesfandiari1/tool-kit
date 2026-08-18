//! Lightweight app settings persisted as JSON in the app config directory.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// `#[serde(default)]` on the struct is load-bearing: without it, a settings.json
/// written before a new field existed fails to parse, and `load()` silently falls
/// back to defaults — wiping the user's output folder and job choice.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Selected inputs: individual files and/or folders to scan.
    pub inputs: Vec<String>,
    /// Folder where finished outputs are written.
    pub output_dir: Option<String>,
    /// Last selected job: "convert" | "transcribe".
    pub job_type: String,
    /// Datalab output format: "markdown" | "html" | "json".
    pub datalab_format: String,
    /// Optional Datalab pipeline id (pl_...). Blank uses the /convert endpoint.
    pub datalab_pipeline_id: Option<String>,
    /// High-accuracy Convert profile: re-OCR every page and run an LLM pass.
    /// Much better on scanned, table-heavy documents; slower and costs more
    /// credits, and unnecessary for clean digital PDFs.
    pub datalab_high_accuracy: bool,
    /// Leave a file alone when the history says it already has a result on
    /// disk. On by default: paying twice for the same conversion is the thing
    /// the history layer exists to prevent.
    pub skip_already_done: bool,
    /// Last SplitPane layout, percentages keyed by pane id (`start` / `end`).
    pub split_layout: Option<BTreeMap<String, f64>>,
    /// Inner window size while the document pane is open.
    pub expanded_width: Option<u32>,
    pub expanded_height: Option<u32>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            inputs: Vec::new(),
            output_dir: None,
            job_type: "convert".into(),
            datalab_format: "markdown".into(),
            datalab_pipeline_id: None,
            datalab_high_accuracy: true,
            skip_already_done: true,
            split_layout: None,
            expanded_width: None,
            expanded_height: None,
        }
    }
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("settings.json"))
}

pub fn load(app: &AppHandle) -> Settings {
    settings_path(app)
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok())
        .unwrap_or_default()
}

/// Written to a temp file and renamed into place. `fs::write` truncates first,
/// so a reader landing mid-write — or a crash — would see a half-file, and
/// `load()` answers a parse failure with `unwrap_or_default()`: the user's
/// output folder and job silently reset. Rename is atomic, so a reader sees
/// either the old file or the new one.
pub fn save(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let path = settings_path(app)?;
    let bytes = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}
