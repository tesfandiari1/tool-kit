use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{
    fs::OpenOptions,
    io::AsyncWriteExt,
    sync::{watch, Notify},
};
use uuid::Uuid;

use crate::{
    artifacts::{
        artifact_relative_path, source_relative_path, ArtifactError, ArtifactStore, AttemptPaths,
        PreparedSubmission, PublishedArtifact,
    },
    engines::{EngineFailure, EngineOutcome, PdfInspectorEngine},
    persistence::{
        hash_idempotency_key, ArtifactKind as StoredArtifactKind, ConversionState, CreateOutcome,
        DocumentClassification, EngineRecord, FailedResult, FailureStage, LocalAnalysis,
        LocalStart, NeedsRemoteResult, NewArtifact, NewConversion, NewSource, RepositoryError,
        SqliteRepository, StoredArtifact, StoredConversion, StoredFailure, SuccessfulArtifacts,
    },
    worker_protocol::{Inspection, PdfTypeLabel, PDF_INSPECTOR_VERSION},
};

use super::{
    model::{now, ManifestEngine, ManifestOutput, ManifestRoute, ManifestSource},
    ArtifactKind, ArtifactRecord, ArtifactView, ConversionManifest, ConversionProfile, JobStatus,
    JobView, SourceMetadata,
};

const SOURCE_MEDIA_TYPE: &str = "application/pdf";
const MARKDOWN_MEDIA_TYPE: &str = "text/markdown; charset=utf-8";
const MANIFEST_MEDIA_TYPE: &str = "application/json";
const LOCAL_ROUTE: &str = "local_pdf";
const NATIVE_TEXT_REASON: &str = "native_text_pdf";

#[derive(Clone, Debug)]
pub struct ConversionService {
    repository: SqliteRepository,
    artifacts: ArtifactStore,
    engine: PdfInspectorEngine,
    work_notification: Arc<Notify>,
}

impl ConversionService {
    pub fn new(
        repository: SqliteRepository,
        artifacts: ArtifactStore,
        engine: PdfInspectorEngine,
    ) -> Self {
        Self {
            repository,
            artifacts,
            engine,
            work_notification: Arc::new(Notify::new()),
        }
    }

    pub async fn prepare_submission(
        &self,
        job_id: Uuid,
    ) -> Result<PreparedSubmission, ConversionServiceError> {
        Ok(self.artifacts.prepare_submission(job_id).await?)
    }

    pub async fn discard_unaccepted_job(&self, job_id: Uuid) {
        if let Err(error) = self.artifacts.discard_unaccepted_job(job_id).await {
            tracing::warn!(%job_id, %error, "failed to discard an unaccepted submission");
            if let Err(quarantine_error) = self.artifacts.quarantine_preacceptance(job_id).await {
                tracing::error!(
                    %job_id,
                    %quarantine_error,
                    "failed to quarantine an unaccepted submission after cleanup failed"
                );
            }
        }
    }

    pub async fn submit(
        &self,
        submission: Submission,
    ) -> Result<SubmissionDecision, ConversionServiceError> {
        let job_id = submission.prepared.job_id;
        let published_source = match self.artifacts.publish_source(&submission.prepared).await {
            Ok(source) => source,
            Err(error @ ArtifactError::SourceCommittedButNotSynced { .. }) => {
                self.quarantine_uncertain_preacceptance(job_id).await;
                return Err(ConversionServiceError::Artifacts(error));
            }
            Err(error) => {
                self.discard_unaccepted_job(job_id).await;
                return Err(ConversionServiceError::Artifacts(error));
            }
        };
        if published_source.byte_length != submission.source.byte_length
            || published_source.sha256 != submission.source.sha256
        {
            self.discard_unaccepted_job(job_id).await;
            return Err(ConversionServiceError::SourceChangedDuringAcceptance);
        }

        let attempt_id = submission.attempt_id;
        if let Err(error) = self
            .artifacts
            .create_retry_attempt(job_id, attempt_id)
            .await
        {
            self.discard_unaccepted_job(job_id).await;
            return Err(ConversionServiceError::Artifacts(error));
        }

        let fingerprint = submission_fingerprint(
            submission.client_run_id,
            submission.profile,
            &published_source.sha256,
        );
        let input = NewConversion {
            id: job_id,
            initial_attempt_id: attempt_id,
            client_run_id: submission.client_run_id,
            idempotency_key_sha256: hash_idempotency_key(&submission.idempotency_key),
            request_fingerprint: fingerprint,
            profile: submission.profile.into(),
            source: NewSource {
                relative_path: portable_relative(source_relative_path(job_id)),
                media_type: SOURCE_MEDIA_TYPE.to_owned(),
                byte_length: published_source.byte_length,
                sha256: published_source.sha256,
            },
            origin_request_id: submission.origin_request_id,
        };

        let decision = match self.repository.create_or_replay(input).await {
            Ok(decision) => decision,
            Err(error @ RepositoryError::CommitOutcomeUncertain { .. }) => {
                return Err(ConversionServiceError::Persistence(error));
            }
            Err(error) => {
                self.discard_unaccepted_job(job_id).await;
                return Err(ConversionServiceError::Persistence(error));
            }
        };

        match decision {
            CreateOutcome::Created(job) => {
                let accepted = JobView::from_stored(&job);
                self.work_notification.notify_one();
                Ok(SubmissionDecision::Created(accepted))
            }
            CreateOutcome::Replay(job) => {
                self.discard_unaccepted_job(job_id).await;
                Ok(SubmissionDecision::Replay(self.public_view(job).await?))
            }
            CreateOutcome::Conflict => {
                self.discard_unaccepted_job(job_id).await;
                Ok(SubmissionDecision::Conflict)
            }
            CreateOutcome::Capacity => {
                self.discard_unaccepted_job(job_id).await;
                Ok(SubmissionDecision::Capacity)
            }
        }
    }

    pub async fn get(&self, job_id: Uuid) -> Result<Option<JobView>, ConversionServiceError> {
        let Some(job) = self.repository.get(job_id).await? else {
            return Ok(None);
        };
        Ok(Some(self.public_view(job).await?))
    }

    pub async fn accepting_jobs(&self) -> bool {
        match self.repository.accepting_jobs().await {
            Ok(accepting) => accepting,
            Err(error) => {
                tracing::error!(%error, "failed to read durable conversion capacity");
                false
            }
        }
    }

    pub(crate) fn work_notification(&self) -> Arc<Notify> {
        Arc::clone(&self.work_notification)
    }

    pub(crate) async fn claim_next_queued(
        &self,
    ) -> Result<Option<StoredConversion>, RepositoryError> {
        self.repository.claim_next_queued(local_start()).await
    }

    pub async fn artifact_views(
        &self,
        job_id: Uuid,
    ) -> Result<Option<Vec<ArtifactView>>, ConversionServiceError> {
        let Some(job) = self.repository.get(job_id).await? else {
            return Ok(None);
        };
        if job.state != ConversionState::Succeeded {
            return Ok(Some(Vec::new()));
        }
        let Some((markdown, manifest)) = artifact_pair(&job) else {
            self.record_artifact_integrity_failure(&job).await;
            return Ok(Some(Vec::new()));
        };
        match self.validate_artifact_pair(&job, markdown, manifest).await {
            Ok(()) => {}
            Err(ArtifactReadFailure::Integrity) => {
                self.record_artifact_integrity_failure(&job).await;
                return Ok(Some(Vec::new()));
            }
            Err(ArtifactReadFailure::Transient(error)) => {
                return Err(ConversionServiceError::Artifacts(error));
            }
        }
        Ok(Some(vec![
            ArtifactView::from_stored(job.id, markdown),
            ArtifactView::from_stored(job.id, manifest),
        ]))
    }

    pub async fn artifact(
        &self,
        job_id: Uuid,
        kind: ArtifactKind,
    ) -> Result<Option<ArtifactLookup>, ConversionServiceError> {
        let Some(job) = self.repository.get(job_id).await? else {
            return Ok(None);
        };
        if job.state != ConversionState::Succeeded {
            return Ok(Some(if JobStatus::from(job.state).is_terminal() {
                ArtifactLookup::NotFound
            } else {
                ArtifactLookup::NotReady
            }));
        }
        let Some((markdown, manifest)) = artifact_pair(&job) else {
            self.record_artifact_integrity_failure(&job).await;
            return Ok(Some(ArtifactLookup::NotFound));
        };
        match self
            .open_validated_artifact(&job, markdown, manifest, kind)
            .await
        {
            Ok(artifact) => Ok(Some(ArtifactLookup::Ready(artifact))),
            Err(ArtifactReadFailure::Integrity) => {
                self.record_artifact_integrity_failure(&job).await;
                Ok(Some(ArtifactLookup::NotFound))
            }
            Err(ArtifactReadFailure::Transient(error)) => {
                Err(ConversionServiceError::Artifacts(error))
            }
        }
    }

    pub(crate) async fn execute_claimed(
        &self,
        job: StoredConversion,
        shutdown: watch::Receiver<bool>,
    ) -> Result<(), ConversionExecutionError> {
        let job_id = job.id;
        let attempt_id = job.active_attempt.id;
        let request_id = job.origin_request_id.clone();
        if job.state != ConversionState::ConvertingLocal {
            return Err(ConversionExecutionError::InvalidClaim {
                job_id,
                attempt_id,
                state: job.state,
            });
        }
        if *shutdown.borrow() {
            tracing::info!(%job_id, %attempt_id, "preserving claimed conversion during forced shutdown");
            return Ok(());
        }

        if job.source.relative_path != portable_relative(source_relative_path(job_id))
            || job.source.media_type != SOURCE_MEDIA_TYPE
        {
            self.finish_failure(
                job_id,
                attempt_id,
                FailureStage::ConvertingLocal,
                "source_integrity_failed",
                "The immutable source failed integrity validation.",
                true,
            )
            .await?;
            return Ok(());
        }
        let source = match self
            .artifacts
            .open_validated_source(
                job_id,
                Some(job.source.byte_length),
                Some(&job.source.sha256),
            )
            .await
        {
            Ok(source) => source,
            Err(error) => match classify_artifact_error(error) {
                ArtifactReadFailure::Integrity => {
                    tracing::warn!(%job_id, %attempt_id, "claimed source failed integrity validation");
                    self.finish_failure(
                        job_id,
                        attempt_id,
                        FailureStage::ConvertingLocal,
                        "source_integrity_failed",
                        "The immutable source failed integrity validation.",
                        true,
                    )
                    .await?;
                    return Ok(());
                }
                ArtifactReadFailure::Transient(error) => {
                    tracing::error!(%job_id, %attempt_id, %error, "claimed source could not be validated; recovery is required");
                    return Err(error.into());
                }
            },
        };

        let permit = match self.engine.acquire().await {
            Ok(permit) => permit,
            Err(failure) => {
                self.finish_failure(
                    job_id,
                    attempt_id,
                    FailureStage::ConvertingLocal,
                    failure.code(),
                    failure.message(),
                    true,
                )
                .await?;
                return Ok(());
            }
        };

        let paths = match self.artifacts.prepare_artifacts(job_id, attempt_id).await {
            Ok(paths) => paths,
            Err(error) => {
                tracing::warn!(%job_id, %attempt_id, %error, "failed to prepare conversion artifacts");
                self.finish_failure(
                    job_id,
                    attempt_id,
                    FailureStage::ConvertingLocal,
                    EngineFailure::Protocol.code(),
                    "The service could not prepare conversion artifacts.",
                    true,
                )
                .await?;
                return Ok(());
            }
        };

        match self.engine.convert(&paths, source, permit, shutdown).await {
            Ok(EngineOutcome::Converted {
                inspection,
                byte_length,
                sha256,
            }) => {
                self.finalize_success(job, paths, inspection, byte_length, sha256)
                    .await?;
            }
            Ok(EngineOutcome::NeedsRemote {
                inspection,
                reason_code,
            }) => {
                let reason = reason_code.as_str().to_owned();
                let analysis = match local_analysis(&inspection, vec![reason.clone()]) {
                    Ok(analysis) => analysis,
                    Err(_) => {
                        self.finish_failure(
                            job_id,
                            attempt_id,
                            FailureStage::ConvertingLocal,
                            "inspection_encoding_failed",
                            "The service could not persist document inspection metadata.",
                            true,
                        )
                        .await?;
                        return Ok(());
                    }
                };
                self.repository
                    .finish_needs_remote(
                        job_id,
                        attempt_id,
                        NeedsRemoteResult {
                            analysis,
                            fallback_reason: reason.clone(),
                        },
                    )
                    .await?;
                self.discard_attempt(job_id, attempt_id).await;
                tracing::info!(
                    %job_id,
                    %attempt_id,
                    %request_id,
                    status = "needs_remote",
                    reason_code = %reason,
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
                self.finish_failure(
                    job_id,
                    attempt_id,
                    FailureStage::ConvertingLocal,
                    code.as_str(),
                    message,
                    true,
                )
                .await?;
            }
            Err(EngineFailure::Interrupted) => {
                tracing::info!(
                    %job_id,
                    %attempt_id,
                    %request_id,
                    "conversion interrupted for shutdown; preserving recoverable state"
                );
            }
            Err(failure) => {
                self.finish_failure(
                    job_id,
                    attempt_id,
                    FailureStage::ConvertingLocal,
                    failure.code(),
                    failure.message(),
                    true,
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn finalize_success(
        &self,
        job: StoredConversion,
        paths: AttemptPaths,
        inspection: Inspection,
        markdown_bytes: u64,
        markdown_sha256: String,
    ) -> Result<(), ConversionExecutionError> {
        let job_id = job.id;
        let attempt_id = job.active_attempt.id;
        let request_id = job.origin_request_id.clone();
        let analysis = match local_analysis(&inspection, vec![NATIVE_TEXT_REASON.to_owned()]) {
            Ok(analysis) => analysis,
            Err(_) => {
                self.finish_failure(
                    job_id,
                    attempt_id,
                    FailureStage::ConvertingLocal,
                    "inspection_encoding_failed",
                    "The service could not persist document inspection metadata.",
                    true,
                )
                .await?;
                return Ok(());
            }
        };
        let finalizing = self
            .repository
            .mark_finalizing(job_id, attempt_id, analysis)
            .await?;

        let completed_at = now();
        let manifest = ConversionManifest {
            schema_version: 1,
            job_id,
            attempt_id,
            client_run_id: finalizing.client_run_id,
            profile: finalizing.profile.into(),
            source: ManifestSource {
                media_type: SOURCE_MEDIA_TYPE.to_owned(),
                byte_length: finalizing.source.byte_length,
                sha256: finalizing.source.sha256.clone(),
            },
            engine: ManifestEngine {
                name: "pdf-inspector".to_owned(),
                version: PDF_INSPECTOR_VERSION.to_owned(),
                features: Vec::new(),
            },
            route: ManifestRoute {
                kind: LOCAL_ROUTE.to_owned(),
                reason_codes: vec![NATIVE_TEXT_REASON.to_owned()],
            },
            document: inspection,
            warnings: Vec::new(),
            output: ManifestOutput {
                media_type: MARKDOWN_MEDIA_TYPE.to_owned(),
                byte_length: markdown_bytes,
                sha256: markdown_sha256.clone(),
            },
            started_at: finalizing
                .active_attempt
                .started_at
                .clone()
                .unwrap_or_else(|| finalizing.created_at.clone()),
            completed_at,
        };
        let encoded = match serde_json::to_vec_pretty(&manifest) {
            Ok(encoded) => encoded,
            Err(_) => {
                self.finish_failure(
                    job_id,
                    attempt_id,
                    FailureStage::Finalizing,
                    "manifest_encoding_failed",
                    "The service could not publish conversion artifacts.",
                    true,
                )
                .await?;
                return Ok(());
            }
        };
        let manifest_sha256 = hex::encode(Sha256::digest(&encoded));
        if write_and_sync_new(&paths.staged_manifest(), &encoded)
            .await
            .is_err()
            || sync_directory(paths.publication_staging.clone())
                .await
                .is_err()
        {
            self.finish_failure(
                job_id,
                attempt_id,
                FailureStage::Finalizing,
                "artifact_publication_failed",
                "The service could not publish conversion artifacts.",
                true,
            )
            .await?;
            return Ok(());
        }

        match self.artifacts.publish_artifacts(job_id, attempt_id).await {
            Ok(()) => {}
            Err(error @ ArtifactError::PublicationCommittedButNotSynced { .. }) => {
                tracing::error!(
                    %job_id,
                    %attempt_id,
                    %error,
                    "artifact publication commit is not known to be durable; preserving finalizing state"
                );
                return Err(error.into());
            }
            Err(error) => {
                tracing::warn!(%job_id, %attempt_id, %error, "failed to publish conversion artifacts");
                self.finish_failure(
                    job_id,
                    attempt_id,
                    FailureStage::Finalizing,
                    "artifact_publication_failed",
                    "The service could not publish conversion artifacts.",
                    true,
                )
                .await?;
                return Ok(());
            }
        }

        let markdown_validation = self
            .artifacts
            .validate_artifact(
                job_id,
                attempt_id,
                PublishedArtifact::Markdown,
                Some(markdown_bytes),
                Some(&markdown_sha256),
            )
            .await;
        let manifest_validation = self
            .artifacts
            .validate_artifact(
                job_id,
                attempt_id,
                PublishedArtifact::Manifest,
                Some(encoded.len() as u64),
                Some(&manifest_sha256),
            )
            .await;
        for validation in [markdown_validation, manifest_validation] {
            if let Err(error) = validation {
                match classify_artifact_error(error) {
                    ArtifactReadFailure::Integrity => {
                        self.finish_failure(
                            job_id,
                            attempt_id,
                            FailureStage::Finalizing,
                            "artifact_integrity_failed",
                            "A published artifact failed integrity validation.",
                            false,
                        )
                        .await?;
                        return Ok(());
                    }
                    ArtifactReadFailure::Transient(error) => return Err(error.into()),
                }
            }
        }

        let artifacts = SuccessfulArtifacts {
            markdown: NewArtifact {
                relative_path: portable_relative(artifact_relative_path(
                    job_id,
                    attempt_id,
                    PublishedArtifact::Markdown,
                )),
                media_type: MARKDOWN_MEDIA_TYPE.to_owned(),
                byte_length: markdown_bytes,
                sha256: markdown_sha256,
            },
            manifest: NewArtifact {
                relative_path: portable_relative(artifact_relative_path(
                    job_id,
                    attempt_id,
                    PublishedArtifact::Manifest,
                )),
                media_type: MANIFEST_MEDIA_TYPE.to_owned(),
                byte_length: encoded.len() as u64,
                sha256: manifest_sha256,
            },
        };
        self.repository
            .finish_succeeded(job_id, attempt_id, artifacts)
            .await?;
        tracing::info!(
            %job_id,
            %attempt_id,
            %request_id,
            status = "succeeded",
            "conversion attempt completed"
        );
        Ok(())
    }

    async fn finish_failure(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
        stage: FailureStage,
        code: &str,
        message: &str,
        discard_attempt: bool,
    ) -> Result<(), ConversionExecutionError> {
        let result = FailedResult {
            stage,
            failure: StoredFailure {
                code: code.to_owned(),
                message: message.to_owned(),
            },
        };
        self.repository
            .finish_failed(job_id, attempt_id, result)
            .await?;
        if discard_attempt {
            self.discard_attempt(job_id, attempt_id).await;
        }
        tracing::warn!(%job_id, %attempt_id, failure_code = %code, "conversion attempt failed");
        Ok(())
    }

    async fn discard_attempt(&self, job_id: Uuid, attempt_id: Uuid) {
        if let Err(error) = self.artifacts.discard_attempt(job_id, attempt_id).await {
            tracing::warn!(%job_id, %attempt_id, %error, "failed to discard terminal attempt staging");
        }
    }

    async fn public_view(&self, job: StoredConversion) -> Result<JobView, ConversionServiceError> {
        if job.state == ConversionState::Succeeded {
            match self.validate_stored_artifacts(&job).await {
                Ok(()) => {}
                Err(ArtifactReadFailure::Integrity) => {
                    return Ok(self.record_artifact_integrity_failure(&job).await);
                }
                Err(ArtifactReadFailure::Transient(error)) => {
                    return Err(ConversionServiceError::Artifacts(error));
                }
            }
        }
        Ok(JobView::from_stored(&job))
    }

    async fn validate_stored_artifacts(
        &self,
        job: &StoredConversion,
    ) -> Result<(), ArtifactReadFailure> {
        let (markdown, manifest) = artifact_pair(job).ok_or(ArtifactReadFailure::Integrity)?;
        self.validate_artifact_pair(job, markdown, manifest).await
    }

    async fn validate_artifact_pair(
        &self,
        job: &StoredConversion,
        markdown: &StoredArtifact,
        manifest: &StoredArtifact,
    ) -> Result<(), ArtifactReadFailure> {
        validate_artifact_metadata(job, markdown, manifest)?;
        self.artifacts
            .validate_artifact(
                job.id,
                job.active_attempt.id,
                PublishedArtifact::Markdown,
                Some(markdown.byte_length),
                Some(&markdown.sha256),
            )
            .await
            .map_err(classify_artifact_error)?;
        self.artifacts
            .validate_artifact(
                job.id,
                job.active_attempt.id,
                PublishedArtifact::Manifest,
                Some(manifest.byte_length),
                Some(&manifest.sha256),
            )
            .await
            .map_err(classify_artifact_error)?;
        Ok(())
    }

    async fn open_validated_artifact(
        &self,
        job: &StoredConversion,
        markdown: &StoredArtifact,
        manifest: &StoredArtifact,
        kind: ArtifactKind,
    ) -> Result<ArtifactRecord, ArtifactReadFailure> {
        validate_artifact_metadata(job, markdown, manifest)?;
        let (selected, other, selected_kind, other_kind) = match kind {
            ArtifactKind::Markdown => (
                markdown,
                manifest,
                PublishedArtifact::Markdown,
                PublishedArtifact::Manifest,
            ),
            ArtifactKind::Manifest => (
                manifest,
                markdown,
                PublishedArtifact::Manifest,
                PublishedArtifact::Markdown,
            ),
        };
        let opened = self
            .artifacts
            .open_validated_artifact(
                job.id,
                job.active_attempt.id,
                selected_kind,
                Some(selected.byte_length),
                Some(&selected.sha256),
            )
            .await
            .map_err(classify_artifact_error)?;
        self.artifacts
            .validate_artifact(
                job.id,
                job.active_attempt.id,
                other_kind,
                Some(other.byte_length),
                Some(&other.sha256),
            )
            .await
            .map_err(classify_artifact_error)?;
        Ok(ArtifactRecord {
            file: opened.file,
            media_type: selected.media_type.clone(),
            byte_length: opened.byte_length,
            sha256: opened.sha256,
        })
    }

    async fn record_artifact_integrity_failure(&self, job: &StoredConversion) -> JobView {
        let failure = StoredFailure {
            code: "artifact_integrity_failed".to_owned(),
            message: "A published artifact failed integrity validation.".to_owned(),
        };
        match self
            .repository
            .mark_artifact_integrity_failed(job.id, job.active_attempt.id, failure)
            .await
        {
            Ok(failed) => JobView::from_stored(&failed),
            Err(error) => {
                tracing::error!(
                    job_id = %job.id,
                    attempt_id = %job.active_attempt.id,
                    %error,
                    "failed to persist artifact integrity failure; failing closed publicly"
                );
                JobView::artifact_integrity_failed(job)
            }
        }
    }

    async fn quarantine_uncertain_preacceptance(&self, job_id: Uuid) {
        match self.artifacts.quarantine_preacceptance(job_id).await {
            Ok(path) => tracing::warn!(
                %job_id,
                quarantine = ?path,
                "quarantined a source whose publication durability is uncertain"
            ),
            Err(error) => tracing::error!(
                %job_id,
                %error,
                "failed to quarantine a source whose publication durability is uncertain"
            ),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Submission {
    pub prepared: PreparedSubmission,
    pub attempt_id: Uuid,
    pub client_run_id: Uuid,
    pub profile: ConversionProfile,
    pub source: SourceMetadata,
    pub idempotency_key: String,
    pub origin_request_id: String,
}

#[derive(Debug)]
pub enum SubmissionDecision {
    Created(JobView),
    Replay(JobView),
    Conflict,
    Capacity,
}

#[derive(Debug)]
pub enum ArtifactLookup {
    Ready(ArtifactRecord),
    NotReady,
    NotFound,
}

#[derive(Debug, Error)]
pub enum ConversionServiceError {
    #[error("artifact storage operation failed")]
    Artifacts(#[from] ArtifactError),
    #[error("persistence operation failed")]
    Persistence(#[from] RepositoryError),
    #[error("the immutable source changed while the submission was being accepted")]
    SourceChangedDuringAcceptance,
}

#[derive(Debug, Error)]
pub(crate) enum ConversionExecutionError {
    #[error(
        "conversion {job_id} attempt {attempt_id} was claimed in invalid state {state:?}; recovery is required"
    )]
    InvalidClaim {
        job_id: Uuid,
        attempt_id: Uuid,
        state: ConversionState,
    },
    #[error(transparent)]
    Artifacts(#[from] ArtifactError),
    #[error(transparent)]
    Persistence(#[from] RepositoryError),
}

pub(crate) fn submission_fingerprint(
    client_run_id: Uuid,
    profile: ConversionProfile,
    source_sha256: &str,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"tool-kit-conversion-m1\0");
    digest.update(client_run_id.as_bytes());
    digest.update(b"\0");
    digest.update(profile.as_str().as_bytes());
    digest.update(b"\0");
    digest.update(source_sha256.as_bytes());
    hex::encode(digest.finalize())
}

fn local_start() -> LocalStart {
    LocalStart {
        engine: EngineRecord {
            name: "pdf-inspector".to_owned(),
            version: PDF_INSPECTOR_VERSION.to_owned(),
        },
        route: LOCAL_ROUTE.to_owned(),
    }
}

fn local_analysis(
    inspection: &Inspection,
    reason_codes: Vec<String>,
) -> Result<LocalAnalysis, serde_json::Error> {
    let classification = match inspection.pdf_type {
        PdfTypeLabel::TextBased => DocumentClassification::TextBased,
        PdfTypeLabel::Scanned => DocumentClassification::Scanned,
        PdfTypeLabel::ImageBased => DocumentClassification::ImageBased,
        PdfTypeLabel::Mixed => DocumentClassification::Mixed,
    };
    Ok(LocalAnalysis {
        classification,
        inspection: serde_json::to_value(inspection)?,
        reason_codes,
        warnings: Vec::new(),
    })
}

fn artifact_pair(job: &StoredConversion) -> Option<(&StoredArtifact, &StoredArtifact)> {
    if job.artifacts.len() != 2 {
        return None;
    }
    let markdown = job
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == StoredArtifactKind::Markdown)?;
    let manifest = job
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == StoredArtifactKind::Manifest)?;
    Some((markdown, manifest))
}

fn validate_artifact_metadata(
    job: &StoredConversion,
    markdown: &StoredArtifact,
    manifest: &StoredArtifact,
) -> Result<(), ArtifactReadFailure> {
    let attempt_id = job.active_attempt.id;
    if markdown.attempt_id != attempt_id
        || manifest.attempt_id != attempt_id
        || markdown.relative_path
            != portable_relative(artifact_relative_path(
                job.id,
                attempt_id,
                PublishedArtifact::Markdown,
            ))
        || manifest.relative_path
            != portable_relative(artifact_relative_path(
                job.id,
                attempt_id,
                PublishedArtifact::Manifest,
            ))
        || markdown.media_type != MARKDOWN_MEDIA_TYPE
        || manifest.media_type != MANIFEST_MEDIA_TYPE
    {
        return Err(ArtifactReadFailure::Integrity);
    }
    Ok(())
}

#[derive(Debug)]
enum ArtifactReadFailure {
    Integrity,
    Transient(ArtifactError),
}

fn classify_artifact_error(error: ArtifactError) -> ArtifactReadFailure {
    match error {
        ArtifactError::InspectPath { ref source, .. } if is_definitive_inspect_error(source) => {
            ArtifactReadFailure::Integrity
        }
        ArtifactError::UnexpectedPublicationContents
        | ArtifactError::InvalidRelativePath(_)
        | ArtifactError::PathLayoutMismatch
        | ArtifactError::SymlinkPath(_)
        | ArtifactError::NotDirectory(_)
        | ArtifactError::NotRegularFile(_)
        | ArtifactError::UnsafeOwnedPath(_)
        | ArtifactError::ByteLengthMismatch { .. }
        | ArtifactError::HashMismatch { .. }
        | ArtifactError::FileChangedDuringValidation(_) => ArtifactReadFailure::Integrity,
        error => ArtifactReadFailure::Transient(error),
    }
}

fn is_definitive_inspect_error(source: &std::io::Error) -> bool {
    if source.kind() == std::io::ErrorKind::NotFound {
        return true;
    }
    #[cfg(unix)]
    if source
        .raw_os_error()
        .is_some_and(|code| code == libc::ELOOP || code == libc::ENOTDIR)
    {
        return true;
    }
    false
}

fn portable_relative(path: PathBuf) -> String {
    path.to_string_lossy().replace('\\', "/")
}

async fn write_and_sync_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
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

async fn sync_directory(path: PathBuf) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || std::fs::File::open(path)?.sync_all())
        .await
        .map_err(std::io::Error::other)?
}

#[cfg(test)]
mod tests {
    use std::{io::ErrorKind, path::PathBuf};

    use super::{classify_artifact_error, ArtifactError, ArtifactReadFailure};

    #[test]
    fn artifact_read_errors_distinguish_integrity_from_transient_io() {
        let missing = ArtifactError::InspectPath {
            path: PathBuf::from("missing"),
            source: std::io::Error::from(ErrorKind::NotFound),
        };
        assert!(matches!(
            classify_artifact_error(missing),
            ArtifactReadFailure::Integrity
        ));

        let mismatch = ArtifactError::HashMismatch {
            path: PathBuf::from("result.md"),
            expected: "0".repeat(64),
            actual: "1".repeat(64),
        };
        assert!(matches!(
            classify_artifact_error(mismatch),
            ArtifactReadFailure::Integrity
        ));

        #[cfg(unix)]
        for raw_error in [libc::ELOOP, libc::ENOTDIR] {
            let replaced_path = ArtifactError::InspectPath {
                path: PathBuf::from("replaced-path"),
                source: std::io::Error::from_raw_os_error(raw_error),
            };
            assert!(matches!(
                classify_artifact_error(replaced_path),
                ArtifactReadFailure::Integrity
            ));
        }

        #[cfg(unix)]
        for raw_error in [libc::EIO, libc::EMFILE] {
            let transient_io = ArtifactError::InspectPath {
                path: PathBuf::from("transient-io"),
                source: std::io::Error::from_raw_os_error(raw_error),
            };
            assert!(matches!(
                classify_artifact_error(transient_io),
                ArtifactReadFailure::Transient(ArtifactError::InspectPath { source, .. })
                    if source.raw_os_error() == Some(raw_error)
            ));
        }

        let denied = ArtifactError::InspectPath {
            path: PathBuf::from("temporarily-unreadable"),
            source: std::io::Error::from(ErrorKind::PermissionDenied),
        };
        assert!(matches!(
            classify_artifact_error(denied),
            ArtifactReadFailure::Transient(ArtifactError::InspectPath { source, .. })
                if source.kind() == ErrorKind::PermissionDenied
        ));
    }
}
