mod model;
mod service;

pub(crate) use model::ConversionManifest;
pub(crate) use model::{advertised_media_types, source_format_by_extension, ContainerMagic};
pub use model::{
    ArtifactKind, ArtifactRecord, ArtifactView, ConversionProfile, JobStatus, JobView,
    SourceMetadata,
};
pub(crate) use service::{classify_artifact_error, ArtifactReadFailure, ConversionExecutionError};
pub use service::{ArtifactLookup, ConversionService, Submission, SubmissionDecision};
