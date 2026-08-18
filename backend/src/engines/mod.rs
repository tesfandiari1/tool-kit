mod outcome;
mod pdf_inspector;

pub use outcome::{EngineAnalysis, EngineFailure, EngineOutcome};
pub use pdf_inspector::{EngineStartupError, PdfInspectorEngine};

pub(crate) use pdf_inspector::is_complete_native_inspection;
