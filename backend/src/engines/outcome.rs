//! Engine-neutral outcome vocabulary shared by every local engine.
//!
//! Engines return [`EngineOutcome`]; the service drives finalization,
//! persistence, and the manifest from it without knowing which engine ran.
//! Engine-specific detail rides inside [`EngineAnalysis::diagnostics`] as
//! opaque JSON, so a second engine does not require a shared document model.

use serde_json::Value;

use crate::persistence::DocumentClassification;
use crate::worker_protocol::FallbackReason;

/// What one local engine learned about one document, in engine-neutral terms.
#[derive(Clone, Debug)]
pub struct EngineAnalysis {
    /// Routing and provenance classification stored on the attempt.
    pub classification: DocumentClassification,
    /// Measurements the routing policy reads. Engine-neutral on purpose: the
    /// policy must not learn what a PDF page is.
    pub quality: QualitySignals,
    /// Engine-specific detail persisted as attempt diagnostics and embedded in
    /// the manifest. Opaque to the service.
    pub diagnostics: Value,
}

/// What an engine measured about the output it produced.
///
/// An engine that cannot measure something reports `None`, and the policy reads
/// that as "no claim made", never as "measured and fine". What it does with a
/// missing claim is the policy's decision, recorded in `policy::decide`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QualitySignals {
    /// Share of pages the engine counted as carrying extractable text,
    /// `0.0..=1.0`. `None` when the engine cannot measure it.
    ///
    /// **This is not a completeness measure, and must never be used as one.**
    /// Measured against the real worker, it is wrong in both directions:
    ///
    /// - A ten-page report whose cover page holds one title line, with no
    ///   images anywhere, reports `0.9`. Nothing is missing from the Markdown.
    /// - A page carrying both text and a full-page scan counts as a text page,
    ///   so a document can report `1.0` with a whole scan's content absent.
    ///
    /// What it does support is the honest statement that some pages produced
    /// no text, which is why the only thing the policy does with it is warn.
    pub native_text_ratio: Option<f32>,
    /// Structural complexity the engine detected in the layout.
    pub has_tables: bool,
    pub has_columns: bool,
}

impl QualitySignals {
    /// For an engine that reports no measurements at all.
    pub fn unmeasured() -> Self {
        Self {
            native_text_ratio: None,
            has_tables: false,
            has_columns: false,
        }
    }

    /// True when the engine measured the document and some pages carried no
    /// extractable text. Says nothing about whether content was lost. Floating
    /// point comes straight from the engine, so compare with a tolerance
    /// rather than against exactly 1.0.
    pub fn has_pages_without_text(self) -> bool {
        self.native_text_ratio
            .is_some_and(|ratio| ratio < 1.0 - f32::EPSILON)
    }
}

#[derive(Clone, Debug)]
pub enum EngineOutcome {
    Converted {
        analysis: EngineAnalysis,
        byte_length: u64,
        sha256: String,
    },
    NeedsRemote {
        analysis: EngineAnalysis,
        reason_code: FallbackReason,
    },
    Rejected {
        rejection: EngineRejection,
    },
}

/// An engine-owned rejection: the stable code and client-safe message travel
/// together, so a second engine adds codes without touching the service.
#[derive(Clone, Debug)]
pub struct EngineRejection {
    pub code: &'static str,
    pub message: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineFailure {
    Unavailable,
    Timeout,
    Interrupted,
    Crashed,
    Protocol,
}

impl EngineFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unavailable => "worker_unavailable",
            Self::Timeout => "worker_timeout",
            Self::Interrupted => "worker_interrupted",
            Self::Crashed => "worker_crash",
            Self::Protocol => "worker_protocol_error",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::Unavailable => "The conversion worker is unavailable.",
            Self::Timeout => "The conversion worker exceeded its time limit.",
            Self::Interrupted => "The conversion worker was interrupted.",
            Self::Crashed => "The conversion worker stopped unexpectedly.",
            Self::Protocol => "The conversion worker returned an invalid result.",
        }
    }
}
