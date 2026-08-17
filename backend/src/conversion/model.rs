use std::{path::PathBuf, str::FromStr};

use serde::Serialize;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use uuid::Uuid;

use crate::{artifacts::AttemptPaths, worker_protocol::Inspection};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
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

#[derive(Clone, Debug)]
pub struct JobRecord {
    pub id: Uuid,
    pub active_attempt_id: Uuid,
    pub client_run_id: Uuid,
    pub profile: ConversionProfile,
    pub status: JobStatus,
    pub route: Option<&'static str>,
    pub reason_codes: Vec<&'static str>,
    pub warnings: Vec<String>,
    pub failure: Option<JobFailure>,
    pub source: SourceMetadata,
    pub paths: AttemptPaths,
    pub artifacts: Option<PublishedArtifacts>,
    pub inspection: Option<Inspection>,
    pub created_at: String,
    pub updated_at: String,
    pub origin_request_id: String,
}

impl JobRecord {
    pub fn new(
        id: Uuid,
        active_attempt_id: Uuid,
        client_run_id: Uuid,
        profile: ConversionProfile,
        source: SourceMetadata,
        paths: AttemptPaths,
        origin_request_id: String,
    ) -> Self {
        let now = now();
        Self {
            id,
            active_attempt_id,
            client_run_id,
            profile,
            status: JobStatus::Queued,
            route: None,
            reason_codes: Vec::new(),
            warnings: Vec::new(),
            failure: None,
            source,
            paths,
            artifacts: None,
            inspection: None,
            created_at: now.clone(),
            updated_at: now,
            origin_request_id,
        }
    }

    pub fn touch(&mut self) {
        self.updated_at = now();
    }

    pub fn view(&self) -> JobView {
        JobView {
            id: self.id,
            active_attempt_id: self.active_attempt_id,
            client_run_id: self.client_run_id,
            profile: self.profile,
            status: self.status,
            route: self.route.map(|kind| RouteView {
                kind,
                reason_codes: self.reason_codes.clone(),
            }),
            warnings: self.warnings.clone(),
            failure: self.failure.clone(),
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteView {
    pub kind: &'static str,
    pub reason_codes: Vec<&'static str>,
}

#[derive(Clone, Debug, Serialize)]
pub struct JobFailure {
    pub code: &'static str,
    pub message: &'static str,
}

#[derive(Clone, Debug)]
pub struct SourceMetadata {
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Clone, Debug)]
pub struct PublishedArtifacts {
    pub markdown: ArtifactRecord,
    pub manifest: ArtifactRecord,
}

#[derive(Clone, Debug)]
pub struct ArtifactRecord {
    pub kind: ArtifactKind,
    pub path: PathBuf,
    pub media_type: &'static str,
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

impl ArtifactRecord {
    pub fn view(&self, job_id: Uuid, attempt_id: Uuid) -> ArtifactView {
        ArtifactView {
            kind: self.kind,
            attempt_id,
            media_type: self.media_type,
            byte_length: self.byte_length,
            sha256: self.sha256.clone(),
            href: format!(
                "/api/v1/conversions/{job_id}/artifacts/{}",
                self.kind.as_str()
            ),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactView {
    pub kind: ArtifactKind,
    pub attempt_id: Uuid,
    pub media_type: &'static str,
    pub byte_length: u64,
    pub sha256: String,
    pub href: String,
}

pub fn now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}
