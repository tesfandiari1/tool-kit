//! Lightweight app settings persisted as JSON in the app config directory.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// The profile a submission names. Every new one is `LocalOnly`. `Standard`
/// stays for ledger rows an older build wrote: the service folds the profile
/// into its replay key, so a recovered row resubmits what it recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversionProfile {
    Standard,
    LocalOnly,
}

impl ConversionProfile {
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::LocalOnly => "local_only",
        }
    }

    pub(crate) fn from_id(value: &str) -> Option<Self> {
        match value {
            "standard" => Some(Self::Standard),
            "local_only" => Some(Self::LocalOnly),
            _ => None,
        }
    }
}

/// `#[serde(default)]` on the struct is load-bearing: without it an older
/// settings.json fails to parse, and `load()` falls back to defaults, wiping the
/// user's output folder and job.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Selected inputs: individual files and/or folders to scan.
    pub inputs: Vec<String>,
    /// Folder where finished outputs are written.
    pub output_dir: Option<String>,
    /// Last selected job: "convert" | "transcribe".
    pub job_type: String,
    /// Whether local OCR corrects what it recognizes. Read only by the image
    /// engine.
    pub language_correction: bool,
    /// Words local OCR should prefer, sent to the backend one per line.
    pub custom_words: Vec<String>,
    /// How many people speak in the recordings. Blank lets the diarizer guess.
    pub speaker_count: Option<u32>,
    /// Leave a file alone when the history says it already has a result.
    pub skip_already_done: bool,
    /// Last SplitPane layout, percentages keyed by pane id (`start` / `end`).
    pub split_layout: Option<BTreeMap<String, f64>>,
    /// Inner window size while the document pane is open.
    pub expanded_width: Option<u32>,
    pub expanded_height: Option<u32>,
    /// Webview page-zoom factor, not a font size. `#[serde(default)]` fills a
    /// missing field, so an old file loads at 1.0 and not 0.0.
    pub zoom: f64,
    /// The one workspace folder this install is bound to.
    pub workspace_path: Option<String>,
    /// Library tree rows the user left open, workspace-relative so a rename in
    /// Finder strands none. A `Vec`, never an `Option<Vec>`: the frontend
    /// spreads this over its defaults, where an explicit `null` would win.
    pub expanded_paths: Vec<String>,
    /// The project a drop is filed into, workspace-relative. This, not
    /// `output_dir`, is where a run writes whenever a workspace is bound.
    pub active_project_path: Option<String>,
    /// Move a drop into the active project instead of leaving it where it is.
    /// Never a copy: the result is the only new file.
    pub move_dropped_files: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            inputs: Vec::new(),
            output_dir: None,
            job_type: "convert".into(),
            language_correction: true,
            custom_words: Vec::new(),
            speaker_count: None,
            skip_already_done: true,
            split_layout: None,
            expanded_width: None,
            expanded_height: None,
            zoom: 1.0,
            workspace_path: None,
            expanded_paths: Vec::new(),
            active_project_path: None,
            move_dropped_files: false,
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

/// Written to a temp file and renamed in. `fs::write` truncates first, so a
/// crash leaves a half-file, and `load()` answers that by resetting the user's
/// output folder and job.
pub fn save(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let path = settings_path(app)?;
    let bytes = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::Settings;

    /// `backend_url` is gone: `backend_host::Deployment` carries the Manual
    /// origin. Ignored, not rejected, because `load()` resets on a parse
    /// failure and every install that used it still has the key.
    #[test]
    fn a_settings_file_naming_the_removed_backend_url_still_loads() {
        let settings: Settings = serde_json::from_str(
            r#"{
                "outputDir": "/tmp/output",
                "jobType": "transcribe",
                "backendUrl": "http://127.0.0.1:8080"
            }"#,
        )
        .expect("settings written before the field was removed should deserialize");

        assert_eq!(settings.output_dir.as_deref(), Some("/tmp/output"));
        assert_eq!(settings.job_type, "transcribe");
    }

    #[test]
    fn backend_settings_serialize_for_the_webview_without_a_token() {
        let value = serde_json::to_value(Settings::default()).expect("settings should serialize");

        assert!(value.get("backendUrl").is_none());
        assert_eq!(value["languageCorrection"], true);
        assert_eq!(value["customWords"], serde_json::json!([]));
        assert_eq!(value["speakerCount"], serde_json::Value::Null);
        assert!(value.get("backendToken").is_none());
    }

    #[test]
    fn settings_from_before_m6_keep_existing_values_and_ignore_the_remote_keys() {
        let settings: Settings = serde_json::from_str(
            r#"{
                "inputs": ["/tmp/source.pdf"],
                "outputDir": "/tmp/output",
                "jobType": "convert",
                "datalabFormat": "html",
                "datalabPipelineId": "pl_existing",
                "datalabHighAccuracy": false,
                "skipAlreadyDone": false
            }"#,
        )
        .expect("pre-M6 settings should deserialize");

        assert_eq!(settings.inputs, ["/tmp/source.pdf"]);
        assert_eq!(settings.output_dir.as_deref(), Some("/tmp/output"));
        // The remote provider's keys are ignored, not rejected.
        assert!(!settings.skip_already_done);
        // A file written before `zoom` existed must load at 100%, not at 0.0.
        assert_eq!(settings.zoom, 1.0);
        // Same rule for the workspace: an existing install owns none yet.
        assert_eq!(settings.workspace_path, None);
    }

    /// Every settings.json on disk predates this field, and `load()` resets on
    /// a parse failure, so an absent key must arrive as an empty list.
    #[test]
    fn settings_from_before_the_library_tree_gain_an_empty_expansion_list() {
        let settings: Settings = serde_json::from_str(
            r#"{
                "outputDir": "/tmp/output",
                "jobType": "convert",
                "workspacePath": "/tmp/workspace"
            }"#,
        )
        .expect("settings written before the tree should deserialize");

        assert!(settings.expanded_paths.is_empty());
        assert_eq!(settings.output_dir.as_deref(), Some("/tmp/output"));
        assert_eq!(settings.workspace_path.as_deref(), Some("/tmp/workspace"));
    }

    /// A field nothing reads, still on disk in every older settings.json.
    /// Ignored rather than rejected, because `load()` resets on a parse failure.
    /// The proof that the struct has no `deny_unknown_fields`.
    #[test]
    fn a_settings_file_still_naming_the_active_project_loads() {
        let settings: Settings = serde_json::from_str(
            r#"{
                "outputDir": "/tmp/output",
                "activeProjectId": "01J0000000000000000000",
                "workspacePath": "/tmp/workspace"
            }"#,
        )
        .expect("a retired key should be ignored, not rejected");

        assert_eq!(settings.output_dir.as_deref(), Some("/tmp/output"));
        assert_eq!(settings.workspace_path.as_deref(), Some("/tmp/workspace"));
    }

    /// A `Vec`, so the wire carries `[]` and never a `null` that would win.
    #[test]
    fn the_expansion_list_serializes_as_an_array() {
        let value = serde_json::to_value(Settings::default()).expect("settings should serialize");

        assert_eq!(value["expandedPaths"], serde_json::json!([]));
    }

    #[test]
    fn settings_from_before_the_ocr_options_gain_defaults() {
        let settings: Settings = serde_json::from_str(
            r#"{
                "inputs": ["/tmp/scan.png"],
                "outputDir": "/tmp/output",
                "jobType": "convert",
                "conversionRoute": "backend",
                "conversionProfile": "local_only",
                "zoom": 1.1
            }"#,
        )
        .expect("settings written before the OCR options should deserialize");

        assert_eq!(settings.inputs, ["/tmp/scan.png"]);
        assert_eq!(settings.zoom, 1.1);
        // Absent fields arrive as on and empty, the backend's own defaults.
        assert!(settings.language_correction);
        assert!(settings.custom_words.is_empty());
    }

    /// Blank means the diarizer guesses, which is what an older file must load
    /// as rather than pinning a count nobody chose.
    #[test]
    fn settings_from_before_the_speaker_count_leave_it_unset() {
        let settings: Settings = serde_json::from_str(
            r#"{
                "outputDir": "/tmp/output",
                "jobType": "transcribe"
            }"#,
        )
        .expect("settings written before the speaker count should deserialize");

        assert_eq!(settings.speaker_count, None);
        assert_eq!(Settings::default().speaker_count, None);
    }

    #[test]
    fn a_chosen_speaker_count_round_trips() {
        let settings: Settings =
            serde_json::from_str(r#"{"speakerCount": 2}"#).expect("a speaker count should parse");

        assert_eq!(settings.speaker_count, Some(2));
    }
}
