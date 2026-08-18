mod anydoc;
mod child;
mod outcome;
mod pdf_inspector;

pub use anydoc::AnyDocEngine;
pub use outcome::{EngineAnalysis, EngineFailure, EngineOutcome, EngineRejection};
pub use pdf_inspector::{EngineStartupError, PdfInspectorEngine};

pub(crate) use anydoc::{AnyDocDiagnostics, ANYDOC_ENGINE_NAME, ANYDOC_VERSION};
pub(crate) use pdf_inspector::is_complete_native_inspection;
