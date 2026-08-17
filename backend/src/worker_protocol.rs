use serde::{Deserialize, Serialize};

pub const WORKER_PROTOCOL_VERSION: u32 = 1;
pub const PDF_INSPECTOR_VERSION: &str = "1.15.0";
pub const WORKER_REPORT_FILE: &str = "worker-report.json";
pub const MARKDOWN_FILE: &str = "result.md";
pub const WORKER_MAX_OUTPUT_BYTES_ENV: &str = "TOOLKIT_PDF_WORKER_MAX_OUTPUT_BYTES";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerReport {
    pub protocol_version: u32,
    pub engine: EngineIdentity,
    pub outcome: WorkerOutcome,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EngineIdentity {
    pub name: String,
    pub version: String,
    pub features: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkerOutcome {
    Converted {
        inspection: Inspection,
        artifact: WorkerArtifact,
    },
    NeedsRemote {
        inspection: Inspection,
        reason_code: FallbackReason,
    },
    Rejected {
        code: RejectionCode,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inspection {
    pub pdf_type: PdfTypeLabel,
    pub confidence: f32,
    pub page_count: u32,
    pub pages_needing_ocr: Vec<u32>,
    pub ocr_reasons_by_page: Vec<PageReasons>,
    pub has_encoding_issues: bool,
    pub is_complex: bool,
    pub pages_with_tables: Vec<u32>,
    pub pages_with_columns: Vec<u32>,
    pub processing_time_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageReasons {
    pub page: u32,
    pub reasons: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PdfTypeLabel {
    TextBased,
    Scanned,
    ImageBased,
    Mixed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerArtifact {
    pub relative_path: String,
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackReason {
    ScannedPdf,
    ImageBasedPdf,
    MixedPdf,
    GarbledText,
    OcrRequired,
    LocalQualityFailed,
    OutputTooLarge,
}

impl FallbackReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ScannedPdf => "scanned_pdf",
            Self::ImageBasedPdf => "image_based_pdf",
            Self::MixedPdf => "mixed_pdf",
            Self::GarbledText => "garbled_text",
            Self::OcrRequired => "ocr_required",
            Self::LocalQualityFailed => "local_quality_failed",
            Self::OutputTooLarge => "output_too_large",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionCode {
    EncryptedPdf,
    InvalidPdf,
    InvalidPdfStructure,
}

impl RejectionCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EncryptedPdf => "encrypted_pdf",
            Self::InvalidPdf => "invalid_pdf",
            Self::InvalidPdfStructure => "invalid_pdf_structure",
        }
    }
}
