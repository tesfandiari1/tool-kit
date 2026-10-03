use std::str::FromStr;

use serde::{
    de::{value::Error as ValueError, IntoDeserializer},
    Deserialize, Serialize,
};
use serde_json::Value;
use uuid::Uuid;

/// The serde strings are the HTTP contract and the sqlx ones the database
/// column. They are the same three words, which is why one enum carries both.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
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
}

impl FromStr for Profile {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::deserialize(IntoDeserializer::<ValueError>::into_deserializer(value)).map_err(|_| ())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
pub enum ConversionState {
    Queued,
    ConvertingLocal,
    Finalizing,
    Succeeded,
    Failed,
    NeedsRemote,
}

impl ConversionState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::NeedsRemote)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, sqlx::Type)]
#[sqlx(rename_all = "snake_case")]
pub enum AttemptState {
    Queued,
    ConvertingLocal,
    Finalizing,
    Succeeded,
    Failed,
    NeedsRemote,
    Interrupted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
pub enum ArtifactKind {
    Markdown,
    Manifest,
}

impl ArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Manifest => "manifest",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(rename_all = "snake_case")]
pub enum DocumentClassification {
    TextBased,
    Scanned,
    ImageBased,
    Mixed,
    /// A structured non-PDF document (Word, PowerPoint, Excel, EPUB) parsed
    /// locally by AnyDoc.
    StructuredDocument,
    /// A recording transcribed locally by the Audio engine.
    Audio,
}

impl DocumentClassification {
    /// Parses a stored classification. The one vocabulary is the derive above:
    /// startup recovery validates stored classifications, and a copy of it in
    /// another module once rejected `structured_document` and refused to boot
    /// after any AnyDoc job succeeded.
    pub(crate) fn from_stored(value: &str) -> Option<Self> {
        Self::deserialize(IntoDeserializer::<ValueError>::into_deserializer(value)).ok()
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
    QuarantineUnrecoverable,
    InterruptAndRequeue,
}

/// One row startup recovery must look at. Decoding is per row on purpose: a
/// listing that fails wholesale puts every job behind the worst row in it.
#[derive(Clone, Debug, PartialEq)]
pub enum RecoveryCandidate {
    Loaded(Box<StoredConversion>),
    /// The row exists and cannot be read as a conversion.
    Undecodable {
        id: String,
        reason: String,
    },
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
    /// The OCR settings the request carried. Only the Vision engine reads
    /// them, and they are stored because the runner claims work from this row
    /// long after the request is gone.
    pub ocr_language_correction: bool,
    pub ocr_custom_words: String,
    /// The speaker count the request pinned, `None` to let the diarizer guess.
    /// Stored for the same reason the OCR settings are.
    pub speaker_count: Option<u32>,
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
    pub ocr_language_correction: bool,
    pub ocr_custom_words: String,
    pub speaker_count: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `as_str` is a hand copy of the `snake_case` rule serde and sqlx share.
    #[test]
    fn as_str_matches_the_derived_vocabulary() {
        for profile in [Profile::Standard, Profile::LocalOnly, Profile::BestQuality] {
            assert_eq!(serde_json::to_value(profile).unwrap(), profile.as_str());
            assert_eq!(profile.as_str().parse::<Profile>(), Ok(profile));
        }
        for kind in [ArtifactKind::Markdown, ArtifactKind::Manifest] {
            assert_eq!(serde_json::to_value(kind).unwrap(), kind.as_str());
        }
        assert_eq!("scanned".parse::<Profile>(), Err(()));
        assert_eq!(
            DocumentClassification::from_stored("structured_document"),
            Some(DocumentClassification::StructuredDocument)
        );
        assert_eq!(DocumentClassification::from_stored("Scanned"), None);
    }
}
