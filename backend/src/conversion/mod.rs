mod model;
mod service;

pub(crate) use model::ConversionManifest;
pub use model::{
    ArtifactKind, ArtifactRecord, ArtifactView, ConversionProfile, JobStatus, JobView,
    SourceMetadata,
};
pub(crate) use service::{classify_artifact_error, ArtifactReadFailure, ConversionExecutionError};
pub use service::{ArtifactLookup, ConversionService, Submission, SubmissionDecision};
