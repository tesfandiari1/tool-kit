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
