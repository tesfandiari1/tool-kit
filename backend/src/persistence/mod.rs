mod model;
mod sqlite;

pub use model::{
    ArtifactKind, AttemptState, CommitOperation, ConversionState, CreateOutcome,
    DocumentClassification, EngineRecord, FailedResult, FailureStage, LocalAnalysis, LocalStart,
    NeedsRemoteResult, NewArtifact, NewConversion, NewSource, Profile, RecoveryCandidate,
    RequeueOutcome, StoredArtifact, StoredAttempt, StoredConversion, StoredFailure, StoredSource,
    SuccessfulArtifacts,
};
pub use sqlite::{hash_idempotency_key, RepositoryError, SqliteRepository, DATABASE_FILENAME};
