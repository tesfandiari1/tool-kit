//! Lightweight app settings persisted as JSON in the app config directory.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversionRoute {
    /// Datalab, over the network, one request per file.
    Direct,
    /// The conversion service, which ships inside the app.
    ///
    /// The default, because the service is in the bundle and a fresh install
    /// should convert without an API key and without sending a document
    /// anywhere. Direct stays a choice for the formats the local engines hand
    /// back, and Rev.ai transcription is untouched either way.
    ///
    /// Nobody arrives here by surprise. A settings.json that predates the
    /// route names none, so `#[serde(default)]` would move it, but that same
    /// file has no `workspace_path` either and the onboarding gate reads that
    /// as first run and asks. The gate writes the route explicitly, so this
    /// default only decides the frame before someone answers.
    #[default]
    Backend,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversionProfile {
    #[default]
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
    /// Reversible M6 route selection. Routing remains per-file when the
    /// backend path is wired; unsupported formats continue to use Datalab.
    pub conversion_route: ConversionRoute,
    /// Backend routing profile. `best_quality` is intentionally unavailable
    /// until the backend implements it instead of returning 409.
    pub conversion_profile: ConversionProfile,
    /// Whether local OCR corrects the words it recognizes. Read only by the
    /// backend's image engine, so it changes nothing on the direct route.
    pub language_correction: bool,
    /// Words local OCR should prefer when it is unsure. Sent to the backend
    /// one per line.
    pub custom_words: Vec<String>,
    /// Leave a file alone when the history says it already has a result on
    /// disk. On by default: paying twice for the same conversion is the thing
    /// the history layer exists to prevent.
    pub skip_already_done: bool,
    /// Last SplitPane layout, percentages keyed by pane id (`start` / `end`).
    pub split_layout: Option<BTreeMap<String, f64>>,
    /// Inner window size while the document pane is open.
    pub expanded_width: Option<u32>,
    pub expanded_height: Option<u32>,
    /// Webview page-zoom factor, not a font size. `#[serde(default)]` on the
    /// struct fills a missing field from the `Default` impl below, so an
    /// existing settings.json loads at 1.0 rather than at 0.0.
    pub zoom: f64,
    /// First-launch workspace setup has run. Until it has, the app shows the
    /// workspace picker instead of the library.
    pub onboarding_complete: bool,
    /// The one workspace folder this install is bound to.
    pub workspace_path: Option<String>,
    /// Stable id from `.toolkit/workspace.json`, so a renamed folder is still
    /// the same workspace.
    pub workspace_id: Option<String>,
    /// The project new documents land in.
    pub active_project_id: Option<String>,
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
            conversion_route: ConversionRoute::Backend,
            conversion_profile: ConversionProfile::Standard,
            language_correction: true,
            custom_words: Vec::new(),
            skip_already_done: true,
            split_layout: None,
            expanded_width: None,
            expanded_height: None,
            zoom: 1.0,
            onboarding_complete: false,
            workspace_path: None,
            workspace_id: None,
            active_project_id: None,
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

#[cfg(test)]
mod tests {
    use super::{ConversionProfile, ConversionRoute, Settings};

    #[test]
    fn defaults_convert_through_the_service_in_the_bundle() {
        let settings = Settings::default();

        assert_eq!(settings.conversion_route, ConversionRoute::Backend);
        assert_eq!(settings.conversion_profile, ConversionProfile::Standard);
    }

    /// `backend_url` is gone: `backend_host::Deployment` carries the Manual
    /// origin. Every install that ever used the Settings field has the key in
    /// its file, and `load()` answers a parse failure by resetting the user's
    /// output folder and job, so the stale key has to be ignored rather than
    /// rejected.
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

        assert_eq!(value["conversionRoute"], "backend");
        assert!(value.get("backendUrl").is_none());
        assert_eq!(value["conversionProfile"], "standard");
        assert_eq!(value["languageCorrection"], true);
        assert_eq!(value["customWords"], serde_json::json!([]));
        assert!(value.get("backendToken").is_none());
    }

    #[test]
    fn settings_from_before_m6_keep_existing_values_and_gain_backend_defaults() {
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
        assert_eq!(settings.datalab_format, "html");
        assert_eq!(settings.datalab_pipeline_id.as_deref(), Some("pl_existing"));
        assert!(!settings.datalab_high_accuracy);
        assert!(!settings.skip_already_done);
        // The route this file never named now defaults to the bundled service.
        // Safe because the same file has no workspace_path, so onboarding runs
        // and asks before a single conversion goes anywhere.
        assert_eq!(settings.conversion_route, ConversionRoute::Backend);
        assert_eq!(settings.conversion_profile, ConversionProfile::Standard);
        // A file written before `zoom` existed must load at 100%, not at 0.0.
        assert_eq!(settings.zoom, 1.0);
        // Same rule for the workspace fields: an existing install has not
        // onboarded and owns no workspace yet.
        assert!(!settings.onboarding_complete);
        assert_eq!(settings.workspace_path, None);
        assert_eq!(settings.workspace_id, None);
        assert_eq!(settings.active_project_id, None);
    }

    #[test]
    fn settings_from_before_the_ocr_options_keep_the_backend_route_and_gain_defaults() {
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
        assert_eq!(settings.conversion_route, ConversionRoute::Backend);
        assert_eq!(settings.conversion_profile, ConversionProfile::LocalOnly);
        assert_eq!(settings.zoom, 1.1);
        // Absent fields arrive as on and empty, matching the backend's own
        // defaults for the two parts.
        assert!(settings.language_correction);
        assert!(settings.custom_words.is_empty());
    }
}
