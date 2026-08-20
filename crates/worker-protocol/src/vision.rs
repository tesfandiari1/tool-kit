//! The Swift Vision worker's report contract.
//!
//! Deliberately separate from [`crate::pdf`]: the two workers share conventions
//! only (source bytes on stdin, the staging directory as argv[1], a report file
//! written into it, a `--version` handshake, exit 0 or 70). A Vision report
//! carries no PDF inspection and its rejection codes are its own, so one shared
//! type would describe neither.
//!
//! The Swift side is `workers/vision/main.swift`. Keep the two in sync.

use serde::{Deserialize, Serialize};

pub const VISION_WORKER_PROTOCOL_VERSION: u32 = 1;
pub const VISION_ENGINE_NAME: &str = "apple-vision";
pub const VISION_WORKER_REPORT_FILE: &str = "worker-report.json";
pub const VISION_MARKDOWN_FILE: &str = "result.md";

/// The handshake line is this prefix plus the running macOS product version.
/// Vision ships with the OS, so that version is the engine's only version
/// number and cannot be pinned at compile time the way pdf-inspector's is.
pub const VISION_WORKER_IDENTITY_PREFIX: &str = "tool-kit-vision-worker protocol=1 apple-vision=";

/// The source-binding names are the shared worker ones: identical meaning,
/// identical parent-side values. See [`crate::pdf`], which declares them for
/// the PDF worker. They are repeated rather than imported so this module stays
/// free of that one, which is the whole point of a per-engine protocol.
pub const VISION_WORKER_EXPECTED_SOURCE_BYTES_ENV: &str = "TOOLKIT_WORKER_EXPECTED_SOURCE_BYTES";
pub const VISION_WORKER_EXPECTED_SOURCE_SHA256_ENV: &str = "TOOLKIT_WORKER_EXPECTED_SOURCE_SHA256";
pub const VISION_WORKER_MAX_OUTPUT_BYTES_ENV: &str = "TOOLKIT_WORKER_MAX_OUTPUT_BYTES";

/// `"1"` or `"0"`. Absent means on.
pub const VISION_WORKER_LANGUAGE_CORRECTION_ENV: &str = "TOOLKIT_VISION_WORKER_LANGUAGE_CORRECTION";
/// Newline separated. Absent or empty means none.
pub const VISION_WORKER_CUSTOM_WORDS_ENV: &str = "TOOLKIT_VISION_WORKER_CUSTOM_WORDS";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisionReport {
    pub protocol_version: u32,
    pub engine: VisionEngineIdentity,
    pub outcome: VisionOutcome,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct VisionEngineIdentity {
    pub name: String,
    /// The macOS product version the worker ran on.
    pub version: String,
    pub features: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VisionOutcome {
    Converted { artifact: VisionArtifact },
    Rejected { code: VisionRejectionCode },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisionArtifact {
    pub relative_path: String,
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VisionRejectionCode {
    /// The bytes carry no frame any installed decoder can read.
    InvalidImage,
    /// The source holds more than one frame. The worker reads one, so a
    /// multi-page TIFF or an animation would publish a fraction of itself as
    /// the whole document.
    MultiFrameImage,
    /// The image decoded and Vision recognized no text on it.
    NoTextFound,
    /// The Markdown exceeded the run's output ceiling, so nothing was written.
    OutputTooLarge,
}
