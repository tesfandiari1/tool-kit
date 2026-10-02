mod model;
mod policy;
mod service;

pub use crate::persistence::{
    ArtifactKind, ConversionState as JobStatus, Profile as ConversionProfile,
};
pub(crate) use model::ConversionManifest;
pub(crate) use model::{
    servable_media_types, source_format_by_extension, ContainerMagic, EngineAvailability,
    LocalEngineKind,
};
pub use model::{ArtifactRecord, ArtifactView, JobView, SourceMetadata};
pub(crate) use service::{
    classify_artifact_error, ArtifactReadFailure, ConversionExecutionError,
    ARTIFACT_INTEGRITY_CODE, ARTIFACT_INTEGRITY_MESSAGE, SOURCE_INTEGRITY_CODE,
    SOURCE_INTEGRITY_MESSAGE,
};
pub use service::{ArtifactLookup, ConversionService, Submission, SubmissionDecision};
