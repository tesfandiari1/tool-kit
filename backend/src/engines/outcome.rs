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
    /// Engine-specific detail persisted as attempt diagnostics and embedded in
    /// the manifest. Opaque to the service.
    pub diagnostics: Value,
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
