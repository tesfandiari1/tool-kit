use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::fs::File;
use uuid::Uuid;

use crate::persistence::{
    ArtifactKind as StoredArtifactKind, ConversionState, Profile, StoredArtifact, StoredConversion,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversionProfile {
    Standard,
    LocalOnly,
    BestQuality,
}

impl ConversionProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::LocalOnly => "local_only",
            Self::BestQuality => "best_quality",
        }
    }
}

impl FromStr for ConversionProfile {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "standard" => Ok(Self::Standard),
            "local_only" => Ok(Self::LocalOnly),
            "best_quality" => Ok(Self::BestQuality),
            _ => Err(()),
        }
    }
}

impl From<ConversionProfile> for Profile {
    fn from(value: ConversionProfile) -> Self {
        match value {
            ConversionProfile::Standard => Self::Standard,
            ConversionProfile::LocalOnly => Self::LocalOnly,
            ConversionProfile::BestQuality => Self::BestQuality,
        }
    }
}

/// One advertised upload format: the accepted extension, the canonical media
/// type stored with the source, the container signature the upload path
/// requires, and the local engine that converts it. Authoritative format
/// detection happens from content at execution time; this table is the
/// admission boundary.
///
/// A format appears here only with a passing round-trip fixture, and the
/// newest migration's CHECK constraint matches this table exactly. That pairing
/// is enforced by `advertised_media_types_match_the_migration_check`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SourceFormat {
    pub extension: &'static str,
    pub media_type: &'static str,
    pub magic: ContainerMagic,
    pub engine: LocalEngineKind,
    /// The family label AnyDoc reports in its diagnostics; used to check
    /// manifest consistency for non-PDF jobs. Unused for PDF.
    pub format_label: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContainerMagic {
    Pdf,
    Zip,
    Ole,
    /// `{\rtf`, the group that must open every RTF file.
    Rtf,
    /// CSV carries no signature at all, so admission cannot check one. The
    /// extension and declared media type are the whole gate, and the engine
    /// names the format explicitly because detection returns `None`.
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LocalEngineKind {
    Pdf,
    AnyDoc,
}

pub(crate) const SOURCE_FORMATS: &[SourceFormat] = &[
    SourceFormat {
        extension: "pdf",
        media_type: "application/pdf",
        magic: ContainerMagic::Pdf,
        engine: LocalEngineKind::Pdf,
        format_label: "pdf",
    },
    SourceFormat {
        extension: "docx",
        media_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "docx",
    },
    SourceFormat {
        extension: "doc",
        media_type: "application/msword",
        magic: ContainerMagic::Ole,
        engine: LocalEngineKind::AnyDoc,
        format_label: "doc",
    },
    SourceFormat {
        extension: "pptx",
        media_type: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "pptx",
    },
    SourceFormat {
        extension: "ppt",
        media_type: "application/vnd.ms-powerpoint",
        magic: ContainerMagic::Ole,
        engine: LocalEngineKind::AnyDoc,
        format_label: "ppt",
    },
    SourceFormat {
        extension: "xlsx",
        media_type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "excel",
    },
    SourceFormat {
        extension: "xls",
        media_type: "application/vnd.ms-excel",
        magic: ContainerMagic::Ole,
        engine: LocalEngineKind::AnyDoc,
        format_label: "excel",
    },
    SourceFormat {
        extension: "epub",
        media_type: "application/epub+zip",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "epub",
    },
    SourceFormat {
        extension: "odt",
        media_type: "application/vnd.oasis.opendocument.text",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "odt",
    },
    SourceFormat {
        extension: "ods",
        media_type: "application/vnd.oasis.opendocument.spreadsheet",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "ods",
    },
    SourceFormat {
        extension: "odp",
        media_type: "application/vnd.oasis.opendocument.presentation",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "odp",
    },
    SourceFormat {
        extension: "rtf",
        media_type: "application/rtf",
        magic: ContainerMagic::Rtf,
        engine: LocalEngineKind::AnyDoc,
        format_label: "rtf",
    },
    SourceFormat {
        extension: "csv",
        media_type: "text/csv",
        magic: ContainerMagic::None,
        engine: LocalEngineKind::AnyDoc,
        format_label: "csv",
    },
    // Extension variants of the families above. AnyDoc routes each to the same
    // parser, so they carry the same `format_label`; only admission differs.
    SourceFormat {
        extension: "docm",
        media_type: "application/vnd.ms-word.document.macroEnabled.12",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "docx",
    },
    SourceFormat {
        extension: "xlsm",
        media_type: "application/vnd.ms-excel.sheet.macroEnabled.12",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "excel",
    },
    SourceFormat {
        extension: "pptm",
        media_type: "application/vnd.ms-powerpoint.presentation.macroEnabled.12",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "pptx",
    },
    SourceFormat {
        extension: "ppsx",
        media_type: "application/vnd.openxmlformats-officedocument.presentationml.slideshow",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "pptx",
    },
    SourceFormat {
        extension: "ppsm",
        media_type: "application/vnd.ms-powerpoint.slideshow.macroEnabled.12",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "pptx",
    },
    // `pps` and `pot` are the same OLE container as `ppt` and share its media
    // type; the extension only tells PowerPoint how to open the file.
    SourceFormat {
        extension: "pps",
        media_type: "application/vnd.ms-powerpoint",
        magic: ContainerMagic::Ole,
        engine: LocalEngineKind::AnyDoc,
        format_label: "ppt",
    },
    SourceFormat {
        extension: "pot",
        media_type: "application/vnd.ms-powerpoint",
        magic: ContainerMagic::Ole,
        engine: LocalEngineKind::AnyDoc,
        format_label: "ppt",
    },
];

pub(crate) fn source_format_by_extension(extension: &str) -> Option<&'static SourceFormat> {
    SOURCE_FORMATS
        .iter()
        .find(|format| format.extension.eq_ignore_ascii_case(extension))
}

pub(crate) fn source_format_by_media_type(media_type: &str) -> Option<&'static SourceFormat> {
    SOURCE_FORMATS
        .iter()
        .find(|format| format.media_type == media_type)
}

/// The media types the service advertises and accepts, in table order.
/// Deduplicated: `ppt`, `pps`, and `pot` are one media type on three
/// extensions.
pub(crate) fn advertised_media_types() -> Vec<&'static str> {
    let mut seen = Vec::new();
    for format in SOURCE_FORMATS {
        if !seen.contains(&format.media_type) {
            seen.push(format.media_type);
        }
    }
    seen
}

impl From<Profile> for ConversionProfile {
    fn from(value: Profile) -> Self {
        match value {
            Profile::Standard => Self::Standard,
            Profile::LocalOnly => Self::LocalOnly,
            Profile::BestQuality => Self::BestQuality,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    ConvertingLocal,
    Finalizing,
    Succeeded,
    Failed,
    NeedsRemote,
}

impl JobStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::NeedsRemote)
    }
}

impl From<ConversionState> for JobStatus {
    fn from(value: ConversionState) -> Self {
        match value {
            ConversionState::Queued => Self::Queued,
            ConversionState::ConvertingLocal => Self::ConvertingLocal,
            ConversionState::Finalizing => Self::Finalizing,
            ConversionState::Succeeded => Self::Succeeded,
            ConversionState::Failed => Self::Failed,
            ConversionState::NeedsRemote => Self::NeedsRemote,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub id: Uuid,
    pub active_attempt_id: Uuid,
    pub client_run_id: Uuid,
    pub profile: ConversionProfile,
    pub status: JobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<RouteView>,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<JobFailure>,
    pub created_at: String,
    pub updated_at: String,
}

impl JobView {
    pub fn from_stored(job: &StoredConversion) -> Self {
        Self {
            id: job.id,
            active_attempt_id: job.active_attempt.id,
            client_run_id: job.client_run_id,
            profile: job.profile.into(),
            status: job.state.into(),
            route: job.route.as_ref().map(|kind| RouteView {
                kind: kind.clone(),
                reason_codes: job.reason_codes.clone(),
            }),
            warnings: job.warnings.clone(),
            failure: job.failure.as_ref().map(|failure| JobFailure {
                code: failure.code.clone(),
                message: failure.message.clone(),
            }),
            created_at: job.created_at.clone(),
            updated_at: job.updated_at.clone(),
        }
    }

    pub fn artifact_integrity_failed(job: &StoredConversion) -> Self {
        let mut view = Self::from_stored(job);
        view.status = JobStatus::Failed;
        view.failure = Some(JobFailure {
            code: "artifact_integrity_failed".to_owned(),
            message: "A published artifact failed integrity validation.".to_owned(),
        });
        view.updated_at = now();
        view
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteView {
    pub kind: String,
    pub reason_codes: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct JobFailure {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceMetadata {
    pub media_type: String,
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
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

impl From<StoredArtifactKind> for ArtifactKind {
    fn from(value: StoredArtifactKind) -> Self {
        match value {
            StoredArtifactKind::Markdown => Self::Markdown,
            StoredArtifactKind::Manifest => Self::Manifest,
        }
    }
}

#[derive(Debug)]
pub struct ArtifactRecord {
    pub file: File,
    pub media_type: String,
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactView {
    pub kind: ArtifactKind,
    pub attempt_id: Uuid,
    pub media_type: String,
    pub byte_length: u64,
    pub sha256: String,
    pub href: String,
}

impl ArtifactView {
    pub fn from_stored(job_id: Uuid, artifact: &StoredArtifact) -> Self {
        let kind = ArtifactKind::from(artifact.kind);
        Self {
            kind,
            attempt_id: artifact.attempt_id,
            media_type: artifact.media_type.clone(),
            byte_length: artifact.byte_length,
            sha256: artifact.sha256.clone(),
            href: format!("/api/v1/conversions/{job_id}/artifacts/{}", kind.as_str()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConversionManifest {
    pub(crate) schema_version: u32,
    pub(crate) job_id: Uuid,
    pub(crate) attempt_id: Uuid,
    pub(crate) client_run_id: Uuid,
    pub(crate) profile: ConversionProfile,
    pub(crate) source: ManifestSource,
    pub(crate) engine: ManifestEngine,
    pub(crate) route: ManifestRoute,
    pub(crate) document: Value,
    pub(crate) warnings: Vec<String>,
    pub(crate) output: ManifestOutput,
    pub(crate) started_at: String,
    pub(crate) completed_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManifestSource {
    pub(crate) media_type: String,
    pub(crate) byte_length: u64,
    pub(crate) sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ManifestEngine {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) features: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManifestRoute {
    pub(crate) kind: String,
    pub(crate) reason_codes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManifestOutput {
    pub(crate) media_type: String,
    pub(crate) byte_length: u64,
    pub(crate) sha256: String,
}

pub fn now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The admission table and the database CHECK are two copies of one list.
    /// If they drift, an upload the API accepts fails at INSERT with a
    /// constraint error instead of a clean 415, so pin them together.
    #[test]
    fn advertised_media_types_match_the_migration_check() {
        let sql = include_str!("../../migrations/0003_anydoc_full_format_set.sql");
        let check = sql
            .split_once("source_media_type     TEXT NOT NULL")
            .expect("the conversions CHECK must exist")
            .1
            .split_once("))")
            .expect("the CHECK must close")
            .0;
        for media_type in advertised_media_types() {
            assert!(
                check.contains(&format!("'{media_type}'")),
                "{media_type} is advertised but the migration CHECK rejects it"
            );
        }
        let quoted = check.matches('\'').count() / 2;
        assert_eq!(
            quoted,
            advertised_media_types().len(),
            "the migration CHECK lists media types the admission table does not"
        );
    }

    #[test]
    fn every_extension_is_unique_and_lowercase() {
        let mut seen = Vec::new();
        for format in SOURCE_FORMATS {
            assert_eq!(format.extension, format.extension.to_ascii_lowercase());
            assert!(
                !seen.contains(&format.extension),
                "duplicate extension {}",
                format.extension
            );
            seen.push(format.extension);
        }
    }
}
