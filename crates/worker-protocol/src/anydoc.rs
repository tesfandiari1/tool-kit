//! The AnyDoc report contract.
//!
//! AnyDoc runs inside the PDF worker binary as `--anydoc <label> [--mark]
//! <dir>`, with the same stdin and source-binding environment as a PDF. The
//! worker writes into a private directory, not the staging one, because the
//! converter picks which Markdown to publish. Written by
//! `apps/converter/src/bin/tool-kit-pdf-worker.rs`, read by
//! `apps/converter/src/engines/anydoc.rs`.

use serde::{Deserialize, Serialize};

pub const ANYDOC_REPORT_FILE: &str = "worker-report.json";
/// The plain Markdown, beside the report. Absent when it is blank.
pub const ANYDOC_MARKDOWN_FILE: &str = "result.md";
/// The Markdown with a `tkimg{n}tk` token where each undescribed picture sits.
pub const ANYDOC_MARKED_FILE: &str = "marked.md";
/// The marked pictures, one file per distinct picture and nothing else, so the
/// Vision worker can describe the whole directory.
pub const ANYDOC_PICTURES_DIRECTORY: &str = "pictures";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AnyDocReport {
    Converted {
        processing_time_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        marked: Option<MarkedPictures>,
    },
    Rejected {
        code: AnyDocRejection,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MarkedPictures {
    /// Each token's number and the picture file that sits there.
    pub placements: Vec<(u32, String)>,
    /// The number of tokens the marked Markdown holds.
    pub rendered: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnyDocRejection {
    UnsupportedDocument,
    EncryptedDocument,
    InvalidDocument,
    DocumentExceedsLimits,
}

impl AnyDocRejection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedDocument => "unsupported_document",
            Self::EncryptedDocument => "encrypted_document",
            Self::InvalidDocument => "invalid_document",
            Self::DocumentExceedsLimits => "document_exceeds_limits",
        }
    }
}
