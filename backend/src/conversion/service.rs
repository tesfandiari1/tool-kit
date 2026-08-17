use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::{fs::OpenOptions, io::AsyncWriteExt};
use uuid::Uuid;

use crate::{
    artifacts::{ArtifactError, ArtifactStore, AttemptPaths},
    engines::{EngineFailure, EngineOutcome, PdfInspectorEngine},
    worker_protocol::{Inspection, PDF_INSPECTOR_VERSION},
};

use super::{
    model::{now, JobFailure, JobRecord},
    ArtifactKind, ArtifactRecord, ConversionProfile, IdempotencyDecision, JobRegistry, JobStatus,
    JobView, PublishedArtifacts, SourceMetadata,
};

#[derive(Clone, Debug)]
pub struct ConversionService {
    registry: JobRegistry,
    artifacts: ArtifactStore,
    engine: PdfInspectorEngine,
}

impl ConversionService {
    pub fn new(
        registry: JobRegistry,
        artifacts: ArtifactStore,
        engine: PdfInspectorEngine,
    ) -> Self {
        Self {
            registry,
            artifacts,
            engine,
        }
    }

    pub async fn begin_attempt(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<AttemptPaths, ArtifactError> {
        self.artifacts.create_attempt(job_id, attempt_id).await
    }

    pub async fn discard_attempt(&self, paths: &AttemptPaths) {
        self.artifacts.discard(paths).await;
    }

    pub async fn submit(&self, submission: Submission) -> IdempotencyDecision {
        let job_id = submission.job_id;
        let paths = submission.paths.clone();
        let job = JobRecord::new(
            submission.job_id,
            submission.attempt_id,
            submission.client_run_id,
            submission.profile,
            submission.source,
            submission.paths,
            submission.origin_request_id,
        );
        let decision = self
            .registry
            .register(submission.idempotency_key, submission.fingerprint, job)
            .await;
        if matches!(decision, IdempotencyDecision::New(_)) {
            let service = self.clone();
            tokio::spawn(async move {
                service.process(job_id).await;
            });
        } else {
            self.artifacts.discard(&paths).await;
        }
        decision
    }

    pub async fn get(&self, job_id: Uuid) -> Option<JobView> {
        self.registry.get(job_id).await.map(|job| job.view())
    }

    pub async fn accepting_jobs(&self) -> bool {
        self.registry.accepting_jobs().await
    }

    pub async fn artifact_views(&self, job_id: Uuid) -> Option<Vec<super::ArtifactView>> {
        let job = self.registry.get(job_id).await?;
        let Some(artifacts) = job.artifacts else {
            return Some(Vec::new());
        };
        Some(vec![
            artifacts.markdown.view(job.id, job.active_attempt_id),
            artifacts.manifest.view(job.id, job.active_attempt_id),
        ])
    }

    pub async fn artifact(&self, job_id: Uuid, kind: ArtifactKind) -> Option<ArtifactLookup> {
        let job = self.registry.get(job_id).await?;
        let Some(artifacts) = job.artifacts else {
            return Some(if job.status.is_terminal() {
                ArtifactLookup::NotFound
            } else {
                ArtifactLookup::NotReady
            });
        };
        let artifact = match kind {
            ArtifactKind::Markdown => artifacts.markdown,
            ArtifactKind::Manifest => artifacts.manifest,
        };
        Some(ArtifactLookup::Ready(artifact))
    }

    async fn process(&self, job_id: Uuid) {
        let Some(job) = self.registry.get(job_id).await else {
            return;
        };
        let attempt_id = job.active_attempt_id;
        let request_id = job.origin_request_id.clone();
        if self
            .artifacts
            .prepare_publication(&job.paths)
            .await
            .is_err()
        {
            self.fail_job(
                job_id,
                EngineFailure::Protocol.code(),
                "The service could not prepare conversion artifacts.",
            )
            .await;
            self.artifacts.discard(&job.paths).await;
            return;
        }

        let permit = match self.engine.acquire().await {
            Ok(permit) => permit,
            Err(failure) => {
                self.fail_job(job_id, failure.code(), failure.message())
                    .await;
                self.artifacts.discard(&job.paths).await;
                return;
            }
        };
        self.registry
            .update(job_id, |job| {
                job.status = JobStatus::ConvertingLocal;
                job.route = Some("local_pdf");
            })
            .await;

        match self.engine.convert(&job.paths, permit).await {
            Ok(EngineOutcome::Converted {
                inspection,
                byte_length,
                sha256,
            }) => {
                self.finalize_success(job, inspection, byte_length, sha256, request_id)
                    .await;
            }
            Ok(EngineOutcome::NeedsRemote {
                inspection,
                reason_code,
            }) => {
                self.registry
                    .update(job_id, |job| {
                        job.status = JobStatus::NeedsRemote;
                        job.reason_codes = vec![reason_code.as_str()];
                        job.inspection = Some(inspection);
                    })
                    .await;
                self.artifacts.discard(&job.paths).await;
                tracing::info!(
                    %job_id,
                    %attempt_id,
                    %request_id,
                    status = "needs_remote",
                    reason_code = reason_code.as_str(),
                    "conversion attempt completed"
                );
            }
            Ok(EngineOutcome::Rejected { code }) => {
                let message = match code {
                    crate::worker_protocol::RejectionCode::EncryptedPdf => {
                        "Encrypted PDFs are not accepted."
                    }
                    crate::worker_protocol::RejectionCode::InvalidPdf => {
                        "The uploaded file is not a valid PDF."
                    }
                    crate::worker_protocol::RejectionCode::InvalidPdfStructure => {
                        "The PDF structure is invalid."
                    }
                };
                self.fail_job(job_id, code.as_str(), message).await;
                self.artifacts.discard(&job.paths).await;
                tracing::warn!(
                    %job_id,
                    %attempt_id,
                    %request_id,
                    failure_code = code.as_str(),
                    "conversion attempt rejected"
                );
            }
            Err(failure) => {
                self.fail_job(job_id, failure.code(), failure.message())
                    .await;
                self.artifacts.discard(&job.paths).await;
                tracing::warn!(
                    %job_id,
                    %attempt_id,
                    %request_id,
                    failure_code = failure.code(),
                    "conversion attempt failed"
                );
            }
        }
    }

    async fn finalize_success(
        &self,
        job: JobRecord,
        inspection: Inspection,
        markdown_bytes: u64,
        markdown_sha256: String,
        request_id: String,
    ) {
        let job_id = job.id;
        let attempt_id = job.active_attempt_id;
        self.registry
            .update(job_id, |job| job.status = JobStatus::Finalizing)
            .await;
        let completed_at = now();
        let manifest = ConversionManifest {
            schema_version: 1,
            job_id,
            attempt_id,
            client_run_id: job.client_run_id,
            profile: job.profile,
            source: ManifestSource {
                media_type: "application/pdf",
                byte_length: job.source.byte_length,
                sha256: &job.source.sha256,
            },
            engine: ManifestEngine {
                name: "pdf-inspector",
                version: PDF_INSPECTOR_VERSION,
                features: Vec::new(),
            },
            route: ManifestRoute {
                kind: "local_pdf",
                reason_codes: vec!["native_text_pdf"],
            },
            document: &inspection,
            warnings: Vec::new(),
            output: ManifestOutput {
                media_type: "text/markdown; charset=utf-8",
                byte_length: markdown_bytes,
                sha256: &markdown_sha256,
            },
            started_at: &job.created_at,
            completed_at: &completed_at,
        };
        let encoded = match serde_json::to_vec_pretty(&manifest) {
            Ok(encoded) => encoded,
            Err(_) => {
                self.fail_and_discard(job_id, &job.paths, "manifest_encoding_failed")
                    .await;
                return;
            }
        };
        let manifest_sha256 = hex::encode(Sha256::digest(&encoded));
        if write_and_sync_new(&job.paths.staged_manifest(), &encoded)
            .await
            .is_err()
            || sync_directory(job.paths.publication_staging.clone())
                .await
                .is_err()
            || self.artifacts.publish(&job.paths).await.is_err()
        {
            self.fail_and_discard(job_id, &job.paths, "artifact_publication_failed")
                .await;
            return;
        }

        let artifacts = PublishedArtifacts {
            markdown: ArtifactRecord {
                kind: ArtifactKind::Markdown,
                path: job.paths.markdown(),
                media_type: "text/markdown; charset=utf-8",
                byte_length: markdown_bytes,
                sha256: markdown_sha256,
            },
            manifest: ArtifactRecord {
                kind: ArtifactKind::Manifest,
                path: job.paths.manifest(),
                media_type: "application/json",
                byte_length: encoded.len() as u64,
                sha256: manifest_sha256,
            },
        };
        self.registry
            .update(job_id, |job| {
                job.status = JobStatus::Succeeded;
                job.reason_codes = vec!["native_text_pdf"];
                job.inspection = Some(inspection);
                job.artifacts = Some(artifacts);
            })
            .await;
        tracing::info!(
            %job_id,
            %attempt_id,
            %request_id,
            status = "succeeded",
            "conversion attempt completed"
        );
    }

    async fn fail_and_discard(&self, job_id: Uuid, paths: &AttemptPaths, code: &'static str) {
        self.fail_job(
            job_id,
            code,
            "The service could not publish conversion artifacts.",
        )
        .await;
        self.artifacts.discard(paths).await;
    }

    async fn fail_job(&self, job_id: Uuid, code: &'static str, message: &'static str) {
        self.registry
            .update(job_id, |job| {
                job.status = JobStatus::Failed;
                job.failure = Some(JobFailure { code, message });
            })
            .await;
    }
}

#[derive(Clone, Debug)]
pub struct Submission {
    pub job_id: Uuid,
    pub attempt_id: Uuid,
    pub client_run_id: Uuid,
    pub profile: ConversionProfile,
    pub source: SourceMetadata,
    pub paths: AttemptPaths,
    pub idempotency_key: String,
    pub fingerprint: String,
    pub origin_request_id: String,
}

#[derive(Clone, Debug)]
pub enum ArtifactLookup {
    Ready(ArtifactRecord),
    NotReady,
    NotFound,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConversionManifest<'a> {
    schema_version: u32,
    job_id: Uuid,
    attempt_id: Uuid,
    client_run_id: Uuid,
    profile: ConversionProfile,
    source: ManifestSource<'a>,
    engine: ManifestEngine,
    route: ManifestRoute,
    document: &'a Inspection,
    warnings: Vec<String>,
    output: ManifestOutput<'a>,
    started_at: &'a str,
    completed_at: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestSource<'a> {
    media_type: &'static str,
    byte_length: u64,
    sha256: &'a str,
}

#[derive(Debug, Serialize)]
struct ManifestEngine {
    name: &'static str,
    version: &'static str,
    features: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestRoute {
    kind: &'static str,
    reason_codes: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestOutput<'a> {
    media_type: &'static str,
    byte_length: u64,
    sha256: &'a str,
}

async fn write_and_sync_new(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options.open(path).await?;
    file.write_all(bytes).await?;
    file.sync_all().await
}

async fn sync_directory(path: std::path::PathBuf) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || std::fs::File::open(path)?.sync_all())
        .await
        .map_err(std::io::Error::other)?
}
