use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Profile {
    Standard,
    LocalOnly,
    BestQuality,
}

impl Profile {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::LocalOnly => "local_only",
            Self::BestQuality => "best_quality",
        }
    }

    pub(crate) fn from_database(value: &str) -> Option<Self> {
        match value {
            "standard" => Some(Self::Standard),
            "local_only" => Some(Self::LocalOnly),
            "best_quality" => Some(Self::BestQuality),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConversionState {
    Queued,
    ConvertingLocal,
    Finalizing,
    Succeeded,
    Failed,
    NeedsRemote,
}

impl ConversionState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::ConvertingLocal => "converting_local",
            Self::Finalizing => "finalizing",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::NeedsRemote => "needs_remote",
        }
    }

    pub(crate) fn from_database(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "converting_local" => Some(Self::ConvertingLocal),
            "finalizing" => Some(Self::Finalizing),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "needs_remote" => Some(Self::NeedsRemote),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptState {
    Queued,
    ConvertingLocal,
    Finalizing,
    Succeeded,
    Failed,
    NeedsRemote,
    Interrupted,
}

impl AttemptState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::ConvertingLocal => "converting_local",
            Self::Finalizing => "finalizing",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::NeedsRemote => "needs_remote",
            Self::Interrupted => "interrupted",
        }
    }

    pub(crate) fn from_database(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "converting_local" => Some(Self::ConvertingLocal),
            "finalizing" => Some(Self::Finalizing),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "needs_remote" => Some(Self::NeedsRemote),
            "interrupted" => Some(Self::Interrupted),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactKind {
    Markdown,
    Manifest,
}

impl ArtifactKind {
    pub(crate) fn from_database(value: &str) -> Option<Self> {
        match value {
            "markdown" => Some(Self::Markdown),
            "manifest" => Some(Self::Manifest),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentClassification {
    TextBased,
    Scanned,
    ImageBased,
    Mixed,
    /// A structured non-PDF document (Word, PowerPoint, Excel, EPUB) parsed
    /// locally by AnyDoc.
    StructuredDocument,
}

impl DocumentClassification {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::TextBased => "text_based",
            Self::Scanned => "scanned",
            Self::ImageBased => "image_based",
            Self::Mixed => "mixed",
            Self::StructuredDocument => "structured_document",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalStart {
    pub engine: EngineRecord,
    pub route: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocalAnalysis {
    pub classification: DocumentClassification,
    pub inspection: Value,
    pub reason_codes: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NeedsRemoteResult {
    pub analysis: LocalAnalysis,
    pub fallback_reason: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureStage {
    Queued,
    ConvertingLocal,
    Finalizing,
}

impl FailureStage {
    pub(crate) fn conversion_state(self) -> ConversionState {
        match self {
            Self::Queued => ConversionState::Queued,
            Self::ConvertingLocal => ConversionState::ConvertingLocal,
            Self::Finalizing => ConversionState::Finalizing,
        }
    }

    pub(crate) fn attempt_state(self) -> AttemptState {
        match self {
            Self::Queued => AttemptState::Queued,
            Self::ConvertingLocal => AttemptState::ConvertingLocal,
            Self::Finalizing => AttemptState::Finalizing,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FailedResult {
    pub stage: FailureStage,
    pub failure: StoredFailure,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewArtifact {
    pub relative_path: String,
    pub media_type: String,
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SuccessfulArtifacts {
    pub markdown: NewArtifact,
    pub manifest: NewArtifact,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitOperation {
    CreateConversion,
    StartLocal,
    ClaimNextQueued,
    MarkFinalizing,
    FinishNeedsRemote,
    FinishFailed,
    FinishSucceeded,
    MarkArtifactIntegrityFailed,
    InterruptAndRequeue,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RequeueOutcome {
    Requeued(StoredConversion),
    LimitReached(StoredConversion),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewSource {
    pub relative_path: String,
    pub media_type: String,
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewConversion {
    pub id: Uuid,
    pub initial_attempt_id: Uuid,
    pub client_run_id: Uuid,
    pub idempotency_key_sha256: String,
    pub request_fingerprint: String,
    pub profile: Profile,
    pub source: NewSource,
    pub origin_request_id: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CreateOutcome {
    Created(StoredConversion),
    Replay(StoredConversion),
    Conflict,
    Capacity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSource {
    pub relative_path: String,
    pub media_type: String,
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineRecord {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredFailure {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoredAttempt {
    pub id: Uuid,
    pub number: u32,
    pub queue_sequence: i64,
    pub state: AttemptState,
    pub recovery_count: u32,
    pub engine: Option<EngineRecord>,
    pub route: Option<String>,
    pub classification: Option<String>,
    pub inspection: Option<Value>,
    pub reason_codes: Vec<String>,
    pub warnings: Vec<String>,
    pub fallback_reason: Option<String>,
    pub failure: Option<StoredFailure>,
    pub created_at: String,
    pub updated_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredArtifact {
    pub attempt_id: Uuid,
    pub kind: ArtifactKind,
    pub relative_path: String,
    pub media_type: String,
    pub byte_length: u64,
    pub sha256: String,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoredConversion {
    pub id: Uuid,
    pub client_run_id: Uuid,
    pub profile: Profile,
    pub state: ConversionState,
    pub source: StoredSource,
    pub active_attempt: StoredAttempt,
    pub route: Option<String>,
    pub reason_codes: Vec<String>,
    pub warnings: Vec<String>,
    pub failure: Option<StoredFailure>,
    pub artifacts: Vec<StoredArtifact>,
    pub origin_request_id: String,
    pub created_at: String,
    pub updated_at: String,
}
