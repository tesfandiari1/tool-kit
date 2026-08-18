use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use sha2::{Digest, Sha256};
use thiserror::Error;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::{
    fs::OpenOptions,
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{watch, Notify},
};
use uuid::Uuid;

use crate::{
    artifacts::{
        artifact_relative_path, source_relative_path, ArtifactError, ArtifactStore, AttemptPaths,
        PreparedSubmission, PublishedArtifact,
    },
    engines::{
        is_complete_native_inspection, AnyDocDiagnostics, AnyDocEngine, EngineAnalysis,
        EngineFailure, EngineOutcome, PdfInspectorEngine, ANYDOC_ENGINE_NAME, ANYDOC_VERSION,
    },
    faults::{FaultBarrier, FaultPoint},
    persistence::{
        hash_idempotency_key, ArtifactKind as StoredArtifactKind, ConversionState, CreateOutcome,
        EngineRecord, FailedResult, FailureStage, LocalAnalysis, LocalStart, NeedsRemoteResult,
        NewArtifact, NewConversion, NewSource, RepositoryError, SqliteRepository, StoredArtifact,
        StoredConversion, StoredFailure, SuccessfulArtifacts,
    },
    worker_protocol::{Inspection, PdfTypeLabel, PDF_INSPECTOR_VERSION},
};

use super::{
    model::{
        now, source_format_by_media_type, LocalEngineKind, ManifestEngine, ManifestOutput,
        ManifestRoute, ManifestSource,
    },
    ArtifactKind, ArtifactRecord, ArtifactView, ConversionManifest, ConversionProfile, JobStatus,
    JobView, SourceMetadata,
};

const MARKDOWN_MEDIA_TYPE: &str = "text/markdown; charset=utf-8";
const MANIFEST_MEDIA_TYPE: &str = "application/json";
const LOCAL_ROUTE: &str = "local_pdf";
const NATIVE_TEXT_REASON: &str = "native_text_pdf";
const LOCAL_ANYDOC_ROUTE: &str = "local_anydoc";
const STRUCTURED_DOCUMENT_REASON: &str = "structured_document";
const MAX_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ConversionService {
    repository: SqliteRepository,
    artifacts: ArtifactStore,
    pdf_engine: PdfInspectorEngine,
    anydoc_engine: AnyDocEngine,
    max_output_bytes: u64,
    work_notification: Arc<Notify>,
    faults: Arc<FaultBarrier>,
}

impl ConversionService {
    pub fn new(
        repository: SqliteRepository,
        artifacts: ArtifactStore,
        pdf_engine: PdfInspectorEngine,
        anydoc_engine: AnyDocEngine,
        max_output_bytes: u64,
    ) -> Self {
        Self {
            repository,
            artifacts,
            pdf_engine,
            anydoc_engine,
            max_output_bytes,
            work_notification: Arc::new(Notify::new()),
            faults: Arc::new(FaultBarrier::default()),
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
                media_type: submission.source.media_type.clone(),
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

    pub async fn health_check(&self) -> Result<(), RepositoryError> {
        self.repository.health_check().await
    }

    pub async fn probe_data_root(&self) -> Result<(), ArtifactError> {
        self.artifacts.probe_writable().await
    }

    pub(crate) fn work_notification(&self) -> Arc<Notify> {
        Arc::clone(&self.work_notification)
    }

    /// Test-only. The returned barrier is disarmed and only a test can arm it.
    pub(crate) fn fault_barrier(&self) -> Arc<FaultBarrier> {
        Arc::clone(&self.faults)
    }

    pub(crate) fn recovery_repository(&self) -> &SqliteRepository {
        &self.repository
    }

    pub(crate) fn recovery_artifacts(&self) -> &ArtifactStore {
        &self.artifacts
    }

    pub(crate) async fn validate_recovery_source(
        &self,
        job: &StoredConversion,
    ) -> Result<(), ArtifactReadFailure> {
        validate_source_metadata(job)?;
        self.artifacts
            .validate_source(
                job.id,
                Some(job.source.byte_length),
                Some(&job.source.sha256),
            )
            .await
            .map_err(classify_artifact_error)?;
        Ok(())
    }

    pub(crate) async fn validate_finalizing_publication(
        &self,
        job: &StoredConversion,
    ) -> Result<SuccessfulArtifacts, ArtifactReadFailure> {
        self.validate_published_artifacts(job, None).await
    }

    pub(crate) async fn claim_next_queued(
        &self,
    ) -> Result<Option<StoredConversion>, RepositoryError> {
        self.repository.claim_next_queued(local_start).await
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
        self.faults.hold(FaultPoint::AfterClaim).await;
        if *shutdown.borrow() {
            tracing::info!(%job_id, %attempt_id, "preserving claimed conversion during forced shutdown");
            return Ok(());
        }

        let Some(source_format) = source_format_by_media_type(&job.source.media_type) else {
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
        };
        if job.source.relative_path != portable_relative(source_relative_path(job_id)) {
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

        let permit = match source_format.engine {
            LocalEngineKind::Pdf => self.pdf_engine.acquire().await,
            LocalEngineKind::AnyDoc => self.anydoc_engine.acquire().await,
        };
        let permit = match permit {
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

        let conversion = match source_format.engine {
            LocalEngineKind::Pdf => {
                self.pdf_engine
                    .convert(&paths, source, permit, shutdown)
                    .await
            }
            LocalEngineKind::AnyDoc => {
                self.anydoc_engine
                    .convert(&paths, source, permit, shutdown)
                    .await
            }
        };
        match conversion {
            Ok(EngineOutcome::Converted {
                analysis,
                byte_length,
                sha256,
            }) => {
                self.finalize_success(job, paths, analysis, byte_length, sha256)
                    .await?;
            }
            Ok(EngineOutcome::NeedsRemote {
                analysis,
                reason_code,
            }) => {
                let reason = reason_code.as_str().to_owned();
                let analysis = local_analysis(&analysis, vec![reason.clone()]);
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
            Ok(EngineOutcome::Rejected { rejection }) => {
                self.finish_failure(
                    job_id,
                    attempt_id,
                    FailureStage::ConvertingLocal,
                    rejection.code,
                    rejection.message,
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
        engine_analysis: EngineAnalysis,
        markdown_bytes: u64,
        markdown_sha256: String,
    ) -> Result<(), ConversionExecutionError> {
        let job_id = job.id;
        let attempt_id = job.active_attempt.id;
        let request_id = job.origin_request_id.clone();
        let reason_code =
            match source_format_by_media_type(&job.source.media_type).map(|format| format.engine) {
                Some(LocalEngineKind::AnyDoc) => STRUCTURED_DOCUMENT_REASON,
                _ => NATIVE_TEXT_REASON,
            };
        let analysis = local_analysis(&engine_analysis, vec![reason_code.to_owned()]);
        let finalizing = self
            .repository
            .mark_finalizing(job_id, attempt_id, analysis)
            .await?;
        self.faults.hold(FaultPoint::AfterFinalizing).await;

        let Some(engine_record) = finalizing.active_attempt.engine.clone() else {
            self.finish_failure(
                job_id,
                attempt_id,
                FailureStage::Finalizing,
                EngineFailure::Protocol.code(),
                "The service could not publish conversion artifacts.",
                true,
            )
            .await?;
            return Ok(());
        };
        let completed_at = now();
        let manifest = ConversionManifest {
            schema_version: 1,
            job_id,
            attempt_id,
            client_run_id: finalizing.client_run_id,
            profile: finalizing.profile.into(),
            source: ManifestSource {
                media_type: finalizing.source.media_type.clone(),
                byte_length: finalizing.source.byte_length,
                sha256: finalizing.source.sha256.clone(),
            },
            engine: ManifestEngine {
                name: engine_record.name,
                version: engine_record.version,
                features: Vec::new(),
            },
            route: ManifestRoute {
                kind: match source_format_by_media_type(&finalizing.source.media_type)
                    .map(|format| format.engine)
                {
                    Some(LocalEngineKind::AnyDoc) => LOCAL_ANYDOC_ROUTE.to_owned(),
                    _ => LOCAL_ROUTE.to_owned(),
                },
                reason_codes: vec![reason_code.to_owned()],
            },
            document: engine_analysis.diagnostics,
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
        let encoded = match serde_json::to_vec(&manifest) {
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
        if encoded.len() as u64 > MAX_MANIFEST_BYTES {
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
        self.faults.hold(FaultPoint::AfterPublish).await;

        let artifacts = match self.validate_finalizing_publication(&finalizing).await {
            Ok(artifacts) => artifacts,
            Err(ArtifactReadFailure::Integrity) => {
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
            Err(ArtifactReadFailure::Transient(error)) => return Err(error.into()),
        };
        self.faults.hold(FaultPoint::BeforeSuccessCommit).await;
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

    pub(crate) async fn validate_stored_artifacts(
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
        self.validate_published_artifacts(job, Some((markdown, manifest)))
            .await?;
        Ok(())
    }

    async fn validate_published_artifacts(
        &self,
        job: &StoredConversion,
        expected: Option<(&StoredArtifact, &StoredArtifact)>,
    ) -> Result<SuccessfulArtifacts, ArtifactReadFailure> {
        validate_source_metadata(job)?;
        self.artifacts
            .validate_published_contents(job.id, job.active_attempt.id)
            .await
            .map_err(classify_artifact_error)?;

        let markdown_expected = expected.map(|(markdown, _)| markdown);
        let manifest_expected = expected.map(|(_, manifest)| manifest);
        let manifest = self
            .artifacts
            .open_bounded_validated_artifact(
                job.id,
                job.active_attempt.id,
                PublishedArtifact::Manifest,
                MAX_MANIFEST_BYTES,
                manifest_expected.map(|artifact| artifact.byte_length),
                manifest_expected.map(|artifact| artifact.sha256.as_str()),
            )
            .await
            .map_err(classify_artifact_error)?;
        let mut encoded = Vec::with_capacity(manifest.byte_length as usize);
        manifest
            .file
            .take(MAX_MANIFEST_BYTES + 1)
            .read_to_end(&mut encoded)
            .await
            .map_err(|source| {
                ArtifactReadFailure::Transient(ArtifactError::InspectPath {
                    path: self
                        .artifacts
                        .attempt_paths(job.id, job.active_attempt.id)
                        .manifest(),
                    source,
                })
            })?;
        if !manifest_bytes_match(&encoded, manifest.byte_length, &manifest.sha256) {
            return Err(ArtifactReadFailure::Integrity);
        }
        let raw: serde_json::Value =
            serde_json::from_slice(&encoded).map_err(|_| ArtifactReadFailure::Integrity)?;
        validate_manifest_shape(&raw)?;
        let decoded: ConversionManifest =
            serde_json::from_value(raw).map_err(|_| ArtifactReadFailure::Integrity)?;
        let markdown_limit =
            markdown_read_limit(markdown_expected, &decoded.output, self.max_output_bytes)?;
        let markdown = self
            .artifacts
            .open_bounded_validated_artifact(
                job.id,
                job.active_attempt.id,
                PublishedArtifact::Markdown,
                markdown_limit,
                Some(decoded.output.byte_length),
                Some(&decoded.output.sha256),
            )
            .await
            .map_err(classify_artifact_error)?;
        validate_manifest(job, &decoded, markdown.byte_length, &markdown.sha256)?;

        Ok(SuccessfulArtifacts {
            markdown: NewArtifact {
                relative_path: portable_relative(artifact_relative_path(
                    job.id,
                    job.active_attempt.id,
                    PublishedArtifact::Markdown,
                )),
                media_type: MARKDOWN_MEDIA_TYPE.to_owned(),
                byte_length: markdown.byte_length,
                sha256: markdown.sha256,
            },
            manifest: NewArtifact {
                relative_path: portable_relative(artifact_relative_path(
                    job.id,
                    job.active_attempt.id,
                    PublishedArtifact::Manifest,
                )),
                media_type: MANIFEST_MEDIA_TYPE.to_owned(),
                byte_length: manifest.byte_length,
                sha256: manifest.sha256,
            },
        })
    }

    async fn open_validated_artifact(
        &self,
        job: &StoredConversion,
        markdown: &StoredArtifact,
        manifest: &StoredArtifact,
        kind: ArtifactKind,
    ) -> Result<ArtifactRecord, ArtifactReadFailure> {
        self.validate_artifact_pair(job, markdown, manifest).await?;
        let (selected, selected_kind) = match kind {
            ArtifactKind::Markdown => (markdown, PublishedArtifact::Markdown),
            ArtifactKind::Manifest => (manifest, PublishedArtifact::Manifest),
        };
        let opened = self
            .artifacts
            .open_bounded_validated_artifact(
                job.id,
                job.active_attempt.id,
                selected_kind,
                selected.byte_length,
                Some(selected.byte_length),
                Some(&selected.sha256),
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

fn local_start(media_type: &str) -> Option<LocalStart> {
    let format = source_format_by_media_type(media_type)?;
    let start = match format.engine {
        LocalEngineKind::Pdf => LocalStart {
            engine: EngineRecord {
                name: "pdf-inspector".to_owned(),
                version: PDF_INSPECTOR_VERSION.to_owned(),
            },
            route: LOCAL_ROUTE.to_owned(),
        },
        LocalEngineKind::AnyDoc => LocalStart {
            engine: EngineRecord {
                name: ANYDOC_ENGINE_NAME.to_owned(),
                version: ANYDOC_VERSION.to_owned(),
            },
            route: LOCAL_ANYDOC_ROUTE.to_owned(),
        },
    };
    Some(start)
}

fn local_analysis(analysis: &EngineAnalysis, reason_codes: Vec<String>) -> LocalAnalysis {
    LocalAnalysis {
        classification: analysis.classification,
        inspection: analysis.diagnostics.clone(),
        reason_codes,
        warnings: Vec::new(),
    }
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

fn validate_source_metadata(job: &StoredConversion) -> Result<(), ArtifactReadFailure> {
    if job.source.relative_path != portable_relative(source_relative_path(job.id))
        || source_format_by_media_type(&job.source.media_type).is_none()
    {
        return Err(ArtifactReadFailure::Integrity);
    }
    Ok(())
}

fn validate_manifest(
    job: &StoredConversion,
    manifest: &ConversionManifest,
    markdown_byte_length: u64,
    markdown_sha256: &str,
) -> Result<(), ArtifactReadFailure> {
    let attempt = &job.active_attempt;
    let Some(engine) = attempt.engine.as_ref() else {
        return Err(ArtifactReadFailure::Integrity);
    };
    let Some(route) = attempt.route.as_ref() else {
        return Err(ArtifactReadFailure::Integrity);
    };
    let Some(inspection) = attempt.inspection.as_ref() else {
        return Err(ArtifactReadFailure::Integrity);
    };
    let Some(classification) = attempt.classification.as_deref() else {
        return Err(ArtifactReadFailure::Integrity);
    };
    let Some(started_at) = attempt.started_at.as_deref() else {
        return Err(ArtifactReadFailure::Integrity);
    };
    // The document detail is engine-specific JSON, checked against the shape
    // of the engine that produced it.
    let manifest_inspection = manifest.document.clone();
    let manifest_classification: &str = match engine.name.as_str() {
        "pdf-inspector" => {
            let document_inspection: Inspection = serde_json::from_value(manifest.document.clone())
                .map_err(|_| ArtifactReadFailure::Integrity)?;
            if !is_complete_native_inspection(&document_inspection) {
                return Err(ArtifactReadFailure::Integrity);
            }
            match document_inspection.pdf_type {
                PdfTypeLabel::TextBased => "text_based",
                PdfTypeLabel::Scanned => "scanned",
                PdfTypeLabel::ImageBased => "image_based",
                PdfTypeLabel::Mixed => "mixed",
            }
        }
        ANYDOC_ENGINE_NAME => {
            let diagnostics: AnyDocDiagnostics = serde_json::from_value(manifest.document.clone())
                .map_err(|_| ArtifactReadFailure::Integrity)?;
            let expected = source_format_by_media_type(&job.source.media_type)
                .filter(|format| format.engine == LocalEngineKind::AnyDoc)
                .map(|format| format.format_label);
            if Some(diagnostics.format.as_str()) != expected {
                return Err(ArtifactReadFailure::Integrity);
            }
            "structured_document"
        }
        _ => return Err(ArtifactReadFailure::Integrity),
    };
    OffsetDateTime::parse(&manifest.started_at, &Rfc3339)
        .map_err(|_| ArtifactReadFailure::Integrity)?;
    OffsetDateTime::parse(&manifest.completed_at, &Rfc3339)
        .map_err(|_| ArtifactReadFailure::Integrity)?;
    match job.state {
        ConversionState::Finalizing if attempt.finished_at.is_none() => {}
        ConversionState::Succeeded => {
            OffsetDateTime::parse(
                attempt
                    .finished_at
                    .as_deref()
                    .ok_or(ArtifactReadFailure::Integrity)?,
                &Rfc3339,
            )
            .map_err(|_| ArtifactReadFailure::Integrity)?;
        }
        _ => return Err(ArtifactReadFailure::Integrity),
    }

    if manifest.schema_version != 1
        || manifest.job_id != job.id
        || manifest.attempt_id != attempt.id
        || manifest.client_run_id != job.client_run_id
        || manifest.profile != job.profile.into()
        || manifest.source.media_type != job.source.media_type
        || manifest.source.byte_length != job.source.byte_length
        || manifest.source.sha256 != job.source.sha256
        || manifest.engine.name != engine.name
        || manifest.engine.version != engine.version
        || !manifest.engine.features.is_empty()
        || manifest.route.kind != *route
        || job.route.as_deref() != Some(route.as_str())
        || manifest.route.reason_codes != attempt.reason_codes
        || manifest.route.reason_codes != job.reason_codes
        || manifest.warnings != attempt.warnings
        || manifest.warnings != job.warnings
        || manifest_inspection != *inspection
        || classification != manifest_classification
        || manifest.output.media_type != MARKDOWN_MEDIA_TYPE
        || markdown_byte_length == 0
        || manifest.output.byte_length != markdown_byte_length
        || manifest.output.sha256 != markdown_sha256
        || manifest.started_at != started_at
    {
        return Err(ArtifactReadFailure::Integrity);
    }
    Ok(())
}

fn validate_manifest_shape(value: &serde_json::Value) -> Result<(), ArtifactReadFailure> {
    require_exact_object_keys(
        value,
        &[
            "schemaVersion",
            "jobId",
            "attemptId",
            "clientRunId",
            "profile",
            "source",
            "engine",
            "route",
            "document",
            "warnings",
            "output",
            "startedAt",
            "completedAt",
        ],
    )?;
    require_exact_object_keys(&value["source"], &["mediaType", "byteLength", "sha256"])?;
    require_exact_object_keys(&value["engine"], &["name", "version", "features"])?;
    require_exact_object_keys(&value["route"], &["kind", "reasonCodes"])?;
    // The document object carries the producing engine's own diagnostics
    // shape; an unknown engine's manifest is not verifiable.
    let engine_name = value["engine"]["name"]
        .as_str()
        .ok_or(ArtifactReadFailure::Integrity)?;
    match engine_name {
        "pdf-inspector" => {
            require_exact_object_keys(
                &value["document"],
                &[
                    "pdfType",
                    "confidence",
                    "pageCount",
                    "pagesNeedingOcr",
                    "ocrReasonsByPage",
                    "hasEncodingIssues",
                    "isComplex",
                    "pagesWithTables",
                    "pagesWithColumns",
                    "processingTimeMs",
                ],
            )?;
            let reasons = value["document"]["ocrReasonsByPage"]
                .as_array()
                .ok_or(ArtifactReadFailure::Integrity)?;
            for page in reasons {
                require_exact_object_keys(page, &["page", "reasons"])?;
            }
        }
        ANYDOC_ENGINE_NAME => {
            require_exact_object_keys(&value["document"], &["format", "processingTimeMs"])?;
        }
        _ => return Err(ArtifactReadFailure::Integrity),
    }
    require_exact_object_keys(&value["output"], &["mediaType", "byteLength", "sha256"])?;
    Ok(())
}

fn markdown_read_limit(
    stored: Option<&StoredArtifact>,
    output: &ManifestOutput,
    configured_maximum: u64,
) -> Result<u64, ArtifactReadFailure> {
    if output.media_type != MARKDOWN_MEDIA_TYPE || output.byte_length == 0 {
        return Err(ArtifactReadFailure::Integrity);
    }
    match stored {
        Some(stored)
            if stored.byte_length == output.byte_length && stored.sha256 == output.sha256 =>
        {
            Ok(stored.byte_length)
        }
        Some(_) => Err(ArtifactReadFailure::Integrity),
        None if output.byte_length <= configured_maximum => Ok(configured_maximum),
        None => Err(ArtifactReadFailure::Integrity),
    }
}

fn manifest_bytes_match(bytes: &[u8], byte_length: u64, sha256: &str) -> bool {
    bytes.len() as u64 == byte_length && hex::encode(Sha256::digest(bytes)) == sha256
}

fn require_exact_object_keys(
    value: &serde_json::Value,
    expected: &[&str],
) -> Result<(), ArtifactReadFailure> {
    let object = value.as_object().ok_or(ArtifactReadFailure::Integrity)?;
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(ArtifactReadFailure::Integrity);
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) enum ArtifactReadFailure {
    Integrity,
    Transient(ArtifactError),
}

pub(crate) fn classify_artifact_error(error: ArtifactError) -> ArtifactReadFailure {
    match error {
        ArtifactError::InspectPath { ref source, .. } if is_definitive_inspect_error(source) => {
            ArtifactReadFailure::Integrity
        }
        ArtifactError::ValidatePublication { ref source, .. }
            if is_definitive_inspect_error(source) =>
        {
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
        | ArtifactError::ByteLengthLimitExceeded { .. }
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

    use serde_json::json;
    use sha2::{Digest, Sha256};
    use uuid::Uuid;

    use super::{
        classify_artifact_error, manifest_bytes_match, markdown_read_limit,
        validate_manifest_shape, ArtifactError, ArtifactReadFailure, ManifestOutput,
    };
    use crate::persistence::{ArtifactKind as StoredArtifactKind, StoredArtifact};

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

    #[test]
    fn publication_directory_races_use_the_same_error_boundary() {
        let missing = ArtifactError::ValidatePublication {
            path: PathBuf::from("artifacts"),
            source: std::io::Error::from(ErrorKind::NotFound),
        };
        assert!(matches!(
            classify_artifact_error(missing),
            ArtifactReadFailure::Integrity
        ));

        let denied = ArtifactError::ValidatePublication {
            path: PathBuf::from("artifacts"),
            source: std::io::Error::from(ErrorKind::PermissionDenied),
        };
        assert!(matches!(
            classify_artifact_error(denied),
            ArtifactReadFailure::Transient(ArtifactError::ValidatePublication { source, .. })
                if source.kind() == ErrorKind::PermissionDenied
        ));
    }

    #[test]
    fn manifest_shape_rejects_unknown_nested_and_page_fields() {
        let valid = json!({
            "schemaVersion": 1,
            "jobId": null,
            "attemptId": null,
            "clientRunId": null,
            "profile": null,
            "source": {"mediaType": null, "byteLength": null, "sha256": null},
            "engine": {"name": "pdf-inspector", "version": null, "features": null},
            "route": {"kind": null, "reasonCodes": null},
            "document": {
                "pdfType": null,
                "confidence": null,
                "pageCount": null,
                "pagesNeedingOcr": null,
                "ocrReasonsByPage": [{"page": null, "reasons": null}],
                "hasEncodingIssues": null,
                "isComplex": null,
                "pagesWithTables": null,
                "pagesWithColumns": null,
                "processingTimeMs": null
            },
            "warnings": null,
            "output": {"mediaType": null, "byteLength": null, "sha256": null},
            "startedAt": null,
            "completedAt": null
        });
        assert!(validate_manifest_shape(&valid).is_ok());

        for path in ["top", "source", "document", "page"] {
            let mut mutated = valid.clone();
            match path {
                "top" => mutated["unexpected"] = json!(true),
                "source" => mutated["source"]["unexpected"] = json!(true),
                "document" => mutated["document"]["unexpected"] = json!(true),
                "page" => {
                    mutated["document"]["ocrReasonsByPage"][0]["unexpected"] = json!(true);
                }
                _ => unreachable!(),
            }
            assert!(matches!(
                validate_manifest_shape(&mutated),
                Err(ArtifactReadFailure::Integrity)
            ));
        }
    }

    #[test]
    fn markdown_limits_preserve_valid_history_and_bound_new_publications() {
        let stored = StoredArtifact {
            attempt_id: Uuid::nil(),
            kind: StoredArtifactKind::Markdown,
            relative_path: "result.md".to_owned(),
            media_type: super::MARKDOWN_MEDIA_TYPE.to_owned(),
            byte_length: 10,
            sha256: "a".repeat(64),
            created_at: "created".to_owned(),
        };
        let output = ManifestOutput {
            media_type: super::MARKDOWN_MEDIA_TYPE.to_owned(),
            byte_length: 10,
            sha256: "a".repeat(64),
        };

        assert_eq!(markdown_read_limit(Some(&stored), &output, 5).unwrap(), 10);
        assert!(matches!(
            markdown_read_limit(None, &output, 5),
            Err(ArtifactReadFailure::Integrity)
        ));
        assert_eq!(markdown_read_limit(None, &output, 10).unwrap(), 10);

        let mismatched = ManifestOutput {
            sha256: "b".repeat(64),
            ..output
        };
        assert!(matches!(
            markdown_read_limit(Some(&stored), &mismatched, 10),
            Err(ArtifactReadFailure::Integrity)
        ));
    }

    #[test]
    fn parsed_manifest_bytes_must_match_the_validated_handle_hash() {
        let bytes = br#"{"schemaVersion":1}"#;
        let sha256 = hex::encode(Sha256::digest(bytes));
        assert!(manifest_bytes_match(bytes, bytes.len() as u64, &sha256));
        assert!(!manifest_bytes_match(
            br#"{"schemaVersion":2}"#,
            bytes.len() as u64,
            &sha256
        ));
    }
}
