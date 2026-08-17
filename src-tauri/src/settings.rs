//! Lightweight app settings persisted as JSON in the app config directory.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Selected inputs: individual files and/or folders to scan.
    #[serde(default)]
    pub inputs: Vec<String>,
    /// Folder where finished outputs are written.
    pub output_dir: Option<String>,
    /// Last selected job: "convert" | "transcribe" | "summarize".
    pub job_type: String,
    /// Datalab output format: "markdown" | "html" | "json".
    pub datalab_format: String,
    /// Optional Datalab pipeline id (pl_...). Blank uses the /convert endpoint.
    pub datalab_pipeline_id: Option<String>,
    /// Claude model used for the Summarize job.
    pub summarize_model: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            inputs: Vec::new(),
            output_dir: None,
            job_type: "convert".into(),
            datalab_format: "markdown".into(),
            datalab_pipeline_id: None,
            summarize_model: "claude-sonnet-4-6".into(),
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

pub fn save(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let path = settings_path(app)?;
    let bytes = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}
