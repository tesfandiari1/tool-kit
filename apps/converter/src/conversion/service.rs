use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
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
    audio_protocol::AUDIO_ENGINE_NAME,
    engines::{
        is_complete_native_inspection, AnyDocDiagnostics, AnyDocEngine, AudioDiagnostics,
        AudioEngine, EngineAnalysis, EngineFailure, EngineOutcome, PdfInspectorEngine,
        VisionDiagnostics, VisionEngine, ANYDOC_ENGINE_NAME, ANYDOC_VERSION,
    },
    faults::{FaultBarrier, FaultPoint},
    persistence::{
        hash_idempotency_key, ConversionState, CreateOutcome, EngineRecord, FailedResult,
        FailureStage, LocalAnalysis, LocalStart, NeedsRemoteResult, NewArtifact, NewConversion,
        NewSource, RepositoryError, SqliteRepository, StoredArtifact, StoredConversion,
        StoredFailure, SuccessfulArtifacts,
    },
    vision_protocol::VISION_ENGINE_NAME,
    worker_protocol::{Inspection, PdfTypeLabel, PDF_INSPECTOR_VERSION},
};

use super::{
    model::{
        now, servable_media_types, source_format_by_media_type, EngineAvailability,
        LocalEngineKind, ManifestEngine, ManifestRoute, ManifestSource,
    },
    policy::{self, LocalResult, PolicyDecision, RouteKind},
    ArtifactKind, ArtifactRecord, ArtifactView, ConversionManifest, ConversionProfile, JobView,
    SourceMetadata,
};

const MARKDOWN_MEDIA_TYPE: &str = "text/markdown; charset=utf-8";
const MANIFEST_MEDIA_TYPE: &str = "application/json";
const MAX_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;

// The wire-visible failure text. Startup recovery reports the same two
// refusals, so it reads them from here rather than keeping its own copy.
pub(crate) const SOURCE_INTEGRITY_CODE: &str = "source_integrity_failed";
pub(crate) const SOURCE_INTEGRITY_MESSAGE: &str =
    "The immutable source failed integrity validation.";
pub(crate) const ARTIFACT_INTEGRITY_CODE: &str = "artifact_integrity_failed";
pub(crate) const ARTIFACT_INTEGRITY_MESSAGE: &str =
    "A published artifact failed integrity validation.";
const PUBLICATION_FAILED_MESSAGE: &str = "The service could not publish conversion artifacts.";

#[derive(Clone, Debug)]
pub struct ConversionService {
    repository: SqliteRepository,
    artifacts: ArtifactStore,
    pdf_engine: PdfInspectorEngine,
    anydoc_engine: AnyDocEngine,
    /// Absent wherever the Vision worker does not run. Image jobs then fail
    /// closed as `worker_unavailable`; nothing else changes.
    vision_engine: Option<VisionEngine>,
    /// Absent wherever the audio worker does not run, exactly like Vision.
    audio_engine: Option<AudioEngine>,
    max_output_bytes: u64,
    /// How long a claimed job may wait for its engine's parser permit before
    /// the runner gives up and lets startup recovery requeue it.
    permit_wait_limit: Duration,
    work_notification: Arc<Notify>,
    faults: Arc<FaultBarrier>,
}

impl ConversionService {
    // Four engines and their two shared limits. Grouping them into a record
    // would be a second name for the field list right below.
    #[expect(clippy::too_many_arguments)]
    pub fn new(
        repository: SqliteRepository,
        artifacts: ArtifactStore,
        pdf_engine: PdfInspectorEngine,
        anydoc_engine: AnyDocEngine,
        vision_engine: Option<VisionEngine>,
        audio_engine: Option<AudioEngine>,
        max_output_bytes: u64,
        permit_wait_limit: Duration,
    ) -> Self {
        Self {
            repository,
            artifacts,
            pdf_engine,
            anydoc_engine,
            vision_engine,
            audio_engine,
            max_output_bytes,
            permit_wait_limit,
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
            submission.language_correction,
            &submission.custom_words,
            submission.speaker_count,
        );
        let input = NewConversion {
            id: job_id,
            initial_attempt_id: attempt_id,
            client_run_id: submission.client_run_id,
            idempotency_key_sha256: hash_idempotency_key(&submission.idempotency_key),
            request_fingerprint: fingerprint,
            profile: submission.profile,
            source: NewSource {
                relative_path: portable_relative(source_relative_path(job_id)),
                media_type: submission.source.media_type.clone(),
                byte_length: published_source.byte_length,
                sha256: published_source.sha256,
            },
            origin_request_id: submission.origin_request_id,
            ocr_language_correction: submission.language_correction,
            ocr_custom_words: submission.custom_words,
            speaker_count: submission.speaker_count,
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

    /// The macOS product version the Vision worker handshook with, or `None`
    /// where that engine is not running here.
    pub fn vision_version(&self) -> Option<&str> {
        self.vision_engine.as_ref().map(VisionEngine::version)
    }

    /// The macOS product version plus the pinned FluidAudio version the audio
    /// worker handshook with, or `None` where that engine is not running here.
    pub fn audio_version(&self) -> Option<&str> {
        self.audio_engine.as_ref().map(AudioEngine::version)
    }

    /// Which optional engines came up here. What capabilities advertises and
    /// what admission accepts both turn on it.
    pub(crate) fn engine_availability(&self) -> EngineAvailability {
        EngineAvailability {
            vision: self.vision_engine.is_some(),
            audio: self.audio_engine.is_some(),
        }
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
        let vision_version = self.vision_version();
        let audio_version = self.audio_version();
        let servable = servable_media_types(self.engine_availability());
        self.repository
            .claim_next_queued(&servable, |media_type| {
                local_start(media_type, vision_version, audio_version)
            })
            .await
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
            return Ok(Some(if job.state.is_terminal() {
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

        // Not corruption: the bytes are intact, no engine claims the type.
        let Some(source_format) = source_format_by_media_type(&job.source.media_type) else {
            self.finish_failure(
                job_id,
                attempt_id,
                FailureStage::ConvertingLocal,
                "unsupported_source_media_type",
                "No local engine handles this source media type.",
                true,
            )
            .await?;
            return Ok(());
        };
        if job.source.relative_path != portable_relative(source_relative_path(job_id)) {
            self.fail_source_integrity(job_id, attempt_id).await?;
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
                    self.fail_source_integrity(job_id, attempt_id).await?;
                    return Ok(());
                }
                ArtifactReadFailure::Transient(error) => {
                    tracing::error!(%job_id, %attempt_id, %error, "claimed source could not be validated; recovery is required");
                    return Err(error.into());
                }
            },
        };

        // A timed-out AnyDoc parse detaches still holding its permit, so this
        // wait is not always short. Unbounded and cancellation-blind, it froze
        // the single runner for every engine and outlasted graceful shutdown.
        let acquire = async {
            match source_format.engine {
                LocalEngineKind::Pdf => self.pdf_engine.acquire().await,
                LocalEngineKind::AnyDoc => self.anydoc_engine.acquire().await,
                LocalEngineKind::Vision => match self.vision_engine.as_ref() {
                    Some(engine) => engine.acquire().await,
                    None => Err(EngineFailure::Unavailable),
                },
                LocalEngineKind::Audio => match self.audio_engine.as_ref() {
                    Some(engine) => engine.acquire().await,
                    None => Err(EngineFailure::Unavailable),
                },
            }
        };
        let mut cancellation = shutdown.clone();
        let permit = tokio::select! {
            permit = acquire => permit,
            () = async {
                let _ = cancellation.wait_for(|stop| *stop).await;
            } => {
                tracing::info!(%job_id, %attempt_id, "shutdown interrupted an engine permit wait; preserving claim");
                return Ok(());
            }
            () = tokio::time::sleep(self.permit_wait_limit) => {
                return Err(ConversionExecutionError::EnginePermitStalled { job_id, attempt_id });
            }
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

        let scan_cancellation = shutdown.clone();
        let conversion = match source_format.engine {
            LocalEngineKind::Pdf => {
                self.pdf_engine
                    .convert(&paths, source, permit, shutdown)
                    .await
            }
            LocalEngineKind::AnyDoc => {
                self.anydoc_engine
                    .convert(&paths, source, permit, shutdown, source_format.format_label)
                    .await
            }
            // Unreachable: the permit above came from the same engine, so an
            // absent one already failed the job. Spelled out because `expect`
            // is denied and a wrong guess here would spend a real conversion.
            LocalEngineKind::Vision => match self.vision_engine.as_ref() {
                Some(engine) => {
                    engine
                        .convert(
                            &paths,
                            source,
                            permit,
                            shutdown,
                            job.ocr_language_correction,
                            &job.ocr_custom_words,
                        )
                        .await
                }
                None => Err(EngineFailure::Unavailable),
            },
            LocalEngineKind::Audio => match self.audio_engine.as_ref() {
                Some(engine) => {
                    engine
                        .convert(
                            &paths,
                            source,
                            permit,
                            shutdown,
                            &job.source.media_type,
                            job.speaker_count,
                        )
                        .await
                }
                None => Err(EngineFailure::Unavailable),
            },
        };
        // A PDF with no native text on any page is a scan, and Vision reads
        // scans locally. Anything short of a conversion keeps the inspector's
        // needs_remote and its reason.
        let scan_engine = match (&conversion, self.vision_engine.as_ref()) {
            (Ok(EngineOutcome::NeedsRemote { analysis, .. }), Some(engine))
                if source_format.engine == LocalEngineKind::Pdf =>
            {
                scanned_page_count(analysis).map(|pages| engine.with_page_budget(pages))
            }
            _ => None,
        };
        let mut relabel = None;
        let conversion = match scan_engine {
            Some(engine) => {
                // Boxed: inline, this future pushed `execute_claimed` past a
                // test thread's stack in debug builds.
                let scan = Box::pin(self.read_scan(&job, &paths, &engine, scan_cancellation));
                match scan.await {
                    Ok(converted @ EngineOutcome::Converted { .. }) => {
                        relabel = Some(LocalStart {
                            engine: EngineRecord {
                                name: VISION_ENGINE_NAME.to_owned(),
                                version: engine.version().to_owned(),
                            },
                            route: RouteKind::LocalVision.as_str().to_owned(),
                        });
                        Ok(converted)
                    }
                    Err(EngineFailure::Interrupted) => Err(EngineFailure::Interrupted),
                    _ => conversion,
                }
            }
            None => conversion,
        };

        // Everything below this line is the routing policy's call, not the
        // engine's. The engine reports what it measured; `policy::decide` says
        // whether that is publishable under this profile.
        let route = if relabel.is_some() {
            RouteKind::LocalVision
        } else {
            route_for(source_format.engine)
        };
        let (analysis, local_result, published_bytes) = match conversion {
            Ok(EngineOutcome::Converted {
                analysis,
                byte_length,
                sha256,
            }) => {
                let signals = analysis.quality;
                (
                    analysis,
                    LocalResult::Converted(signals),
                    Some((byte_length, sha256)),
                )
            }
            Ok(EngineOutcome::NeedsRemote {
                analysis,
                reason_code,
            }) => (analysis, LocalResult::GaveUp(reason_code), None),
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
                return Ok(());
            }
            Err(EngineFailure::Interrupted) => {
                tracing::info!(
                    %job_id,
                    %attempt_id,
                    %request_id,
                    "conversion interrupted for shutdown; preserving recoverable state"
                );
                return Ok(());
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
                return Ok(());
            }
        };

        let decision = policy::decide(job.profile, route, local_result);
        match (&decision, published_bytes) {
            (PolicyDecision::Publish { .. }, Some(markdown)) => {
                self.finalize_success(job, paths, analysis, decision, markdown, relabel)
                    .await?;
            }
            // The policy refused to publish output the engine did produce, so
            // the staged Markdown is discarded with the attempt.
            (PolicyDecision::NeedsRemote { reason_code }, _) => {
                let reason = reason_code.as_str().to_owned();
                let analysis = local_analysis(&analysis, &decision);
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
            // A `Publish` decision with no bytes cannot happen: only the
            // `Converted` arm produces one, and it always carries them. Kept
            // as a closed failure rather than a panic, and logged, because an
            // impossible state that reaches production silently is worse than
            // the branch. Deleting it would mean threading byte counts through
            // `policy::decide`, which is pure on purpose.
            (PolicyDecision::Publish { .. }, None) => {
                tracing::error!(
                    %job_id,
                    %attempt_id,
                    %request_id,
                    "publish decision carried no staged bytes"
                );
                self.finish_failure(
                    job_id,
                    attempt_id,
                    FailureStage::ConvertingLocal,
                    EngineFailure::Protocol.code(),
                    EngineFailure::Protocol.message(),
                    true,
                )
                .await?;
            }
        }
        Ok(())
    }

    /// OCRs a scanned PDF with Vision. Any failure before the worker runs is
    /// `Unavailable`, which keeps the inspector's needs_remote.
    async fn read_scan(
        &self,
        job: &StoredConversion,
        paths: &AttemptPaths,
        engine: &VisionEngine,
        cancellation: watch::Receiver<bool>,
    ) -> Result<EngineOutcome, EngineFailure> {
        let source = self
            .artifacts
            .open_validated_source(
                job.id,
                Some(job.source.byte_length),
                Some(&job.source.sha256),
            )
            .await
            .map_err(|_| EngineFailure::Unavailable)?;
        let permit = engine.acquire().await?;
        engine
            .convert(
                paths,
                source,
                permit,
                cancellation,
                job.ocr_language_correction,
                &job.ocr_custom_words,
            )
            .await
    }

    async fn finalize_success(
        &self,
        job: StoredConversion,
        paths: AttemptPaths,
        engine_analysis: EngineAnalysis,
        decision: PolicyDecision,
        // The staged Markdown's size and digest, as the engine measured them.
        markdown: (u64, String),
        // The engine that wrote the Markdown, when it is not the one claimed.
        relabel: Option<LocalStart>,
    ) -> Result<(), ConversionExecutionError> {
        let (markdown_bytes, markdown_sha256) = markdown;
        let job_id = job.id;
        let attempt_id = job.active_attempt.id;
        let request_id = job.origin_request_id.clone();
        let analysis = local_analysis(&engine_analysis, &decision);
        let finalizing = self
            .repository
            .mark_finalizing(job_id, attempt_id, analysis, relabel.as_ref())
            .await?;
        self.faults.hold(FaultPoint::AfterFinalizing).await;

        // The stored row names the engine and route, relabeled or not.
        let (Some(engine_record), Some(route)) = (
            finalizing.active_attempt.engine.clone(),
            finalizing.active_attempt.route.clone(),
        ) else {
            self.finish_failure(
                job_id,
                attempt_id,
                FailureStage::Finalizing,
                EngineFailure::Protocol.code(),
                PUBLICATION_FAILED_MESSAGE,
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
            profile: finalizing.profile,
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
                kind: route,
                reason_codes: decision.reason_strings(),
            },
            document: engine_analysis.diagnostics,
            warnings: decision.warning_strings(),
            output: ManifestSource {
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
            Ok(encoded) if encoded.len() as u64 <= MAX_MANIFEST_BYTES => encoded,
            _ => {
                self.finish_failure(
                    job_id,
                    attempt_id,
                    FailureStage::Finalizing,
                    "manifest_encoding_failed",
                    PUBLICATION_FAILED_MESSAGE,
                    true,
                )
                .await?;
                return Ok(());
            }
        };
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
                PUBLICATION_FAILED_MESSAGE,
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
                    PUBLICATION_FAILED_MESSAGE,
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
                    ARTIFACT_INTEGRITY_CODE,
                    ARTIFACT_INTEGRITY_MESSAGE,
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

    async fn fail_source_integrity(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<(), ConversionExecutionError> {
        self.finish_failure(
            job_id,
            attempt_id,
            FailureStage::ConvertingLocal,
            SOURCE_INTEGRITY_CODE,
            SOURCE_INTEGRITY_MESSAGE,
            true,
        )
        .await
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
        // `deny_unknown_fields` on the manifest structs is the shape check.
        // The `document` object is the producing engine's own type, so
        // `validate_manifest` decodes it against that engine's struct.
        let decoded: ConversionManifest =
            serde_json::from_slice(&encoded).map_err(|_| ArtifactReadFailure::Integrity)?;
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
            code: ARTIFACT_INTEGRITY_CODE.to_owned(),
            message: ARTIFACT_INTEGRITY_MESSAGE.to_owned(),
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
    /// Read by the Vision engine and by nothing else.
    pub language_correction: bool,
    /// Newline separated, as the worker's env var wants it.
    pub custom_words: String,
    /// Read by the Audio engine and by nothing else. `None` lets the diarizer
    /// guess how many speakers the recording holds.
    pub speaker_count: Option<u32>,
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
    #[error(
        "conversion {job_id} attempt {attempt_id} waited past the engine permit limit; recovery is required"
    )]
    EnginePermitStalled { job_id: Uuid, attempt_id: Uuid },
    #[error(transparent)]
    Artifacts(#[from] ArtifactError),
    #[error(transparent)]
    Persistence(#[from] RepositoryError),
}

/// The request identity a replay is matched on, so every field the job is run
/// with belongs in it. The OCR options and the speaker count are persisted and
/// read by the engine much later, so leaving them out lets a resumed run replay
/// onto a job converted with the settings the user has since turned off.
pub(crate) fn submission_fingerprint(
    client_run_id: Uuid,
    profile: ConversionProfile,
    source_sha256: &str,
    language_correction: bool,
    custom_words: &str,
    speaker_count: Option<u32>,
) -> String {
    let mut digest = Sha256::new();
    // Bumped with the fields below, so an old fingerprint cannot collide.
    digest.update(b"tool-kit-conversion-m2\0");
    digest.update(client_run_id.as_bytes());
    digest.update(b"\0");
    digest.update(profile.as_str().as_bytes());
    digest.update(b"\0");
    digest.update(source_sha256.as_bytes());
    digest.update(b"\0");
    digest.update(if language_correction { b"1" } else { b"0" });
    digest.update(b"\0");
    digest.update(custom_words.as_bytes());
    digest.update(b"\0");
    // Empty for "guess", which is a different job from any pinned count.
    digest.update(
        speaker_count
            .map(|count| count.to_string())
            .unwrap_or_default()
            .as_bytes(),
    );
    hex::encode(digest.finalize())
}

/// The two worker versions are the ones their handshakes reported, or `None`
/// where that worker does not run. `None` here is not a verdict on the job:
/// the claim's SELECT already skipped the media types this boot cannot serve,
/// so a job that reaches this and answers `None` names a type no build knows.
fn local_start(
    media_type: &str,
    vision_version: Option<&str>,
    audio_version: Option<&str>,
) -> Option<LocalStart> {
    let format = source_format_by_media_type(media_type)?;
    let engine = match format.engine {
        LocalEngineKind::Pdf => EngineRecord {
            name: "pdf-inspector".to_owned(),
            version: PDF_INSPECTOR_VERSION.to_owned(),
        },
        LocalEngineKind::AnyDoc => EngineRecord {
            name: ANYDOC_ENGINE_NAME.to_owned(),
            version: ANYDOC_VERSION.to_owned(),
        },
        LocalEngineKind::Vision => EngineRecord {
            name: VISION_ENGINE_NAME.to_owned(),
            version: vision_version?.to_owned(),
        },
        LocalEngineKind::Audio => EngineRecord {
            name: AUDIO_ENGINE_NAME.to_owned(),
            version: audio_version?.to_owned(),
        },
    };
    Some(LocalStart {
        engine,
        route: route_for(format.engine).as_str().to_owned(),
    })
}

/// The page count of a PDF whose every page needs OCR, which is the only kind
/// Vision takes over. A mixed PDF stays remote.
fn scanned_page_count(analysis: &EngineAnalysis) -> Option<u32> {
    let inspection: Inspection = serde_json::from_value(analysis.diagnostics.clone()).ok()?;
    (u32::try_from(inspection.pages_needing_ocr.len()) == Ok(inspection.page_count))
        .then_some(inspection.page_count)
}

/// The one place the engine-to-route rule lives. Both the manifest and the
/// stored attempt row read it, so they cannot label the same job differently.
fn route_for(engine: LocalEngineKind) -> RouteKind {
    match engine {
        LocalEngineKind::Pdf => RouteKind::LocalPdf,
        LocalEngineKind::AnyDoc => RouteKind::LocalAnyDoc,
        LocalEngineKind::Vision => RouteKind::LocalVision,
        LocalEngineKind::Audio => RouteKind::LocalAudio,
    }
}

fn local_analysis(analysis: &EngineAnalysis, decision: &PolicyDecision) -> LocalAnalysis {
    LocalAnalysis {
        classification: analysis.classification,
        inspection: analysis.diagnostics.clone(),
        reason_codes: decision.reason_strings(),
        warnings: decision.warning_strings(),
    }
}

fn artifact_pair(job: &StoredConversion) -> Option<(&StoredArtifact, &StoredArtifact)> {
    if job.artifacts.len() != 2 {
        return None;
    }
    let markdown = job
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == ArtifactKind::Markdown)?;
    let manifest = job
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == ArtifactKind::Manifest)?;
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
        VISION_ENGINE_NAME => {
            serde_json::from_value::<VisionDiagnostics>(manifest.document.clone())
                .map_err(|_| ArtifactReadFailure::Integrity)?;
            "image_based"
        }
        AUDIO_ENGINE_NAME => {
            serde_json::from_value::<AudioDiagnostics>(manifest.document.clone())
                .map_err(|_| ArtifactReadFailure::Integrity)?;
            "audio"
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
        || manifest.profile != job.profile
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
        || manifest.document != *inspection
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

fn markdown_read_limit(
    stored: Option<&StoredArtifact>,
    output: &ManifestSource,
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
        classify_artifact_error, manifest_bytes_match, markdown_read_limit, submission_fingerprint,
        ArtifactError, ArtifactReadFailure, ConversionManifest, ConversionProfile, ManifestSource,
    };
    use crate::persistence::{ArtifactKind, StoredArtifact};
    use crate::worker_protocol::Inspection;

    /// A replay is matched on the fingerprint alone, so a per-run setting
    /// outside it means a resumed run silently replays onto a job converted
    /// with the setting the user has since changed.
    #[test]
    fn the_per_run_engine_settings_are_part_of_the_request_identity() {
        let run = Uuid::new_v4();
        let sha = "a".repeat(64);
        let fingerprint = |correction, words, speakers| {
            submission_fingerprint(
                run,
                ConversionProfile::Standard,
                &sha,
                correction,
                words,
                speakers,
            )
        };
        let baseline = fingerprint(true, "Acme", None);
        assert_ne!(baseline, fingerprint(false, "Acme", None));
        assert_ne!(baseline, fingerprint(true, "", None));
        assert_ne!(baseline, fingerprint(true, "Acme", Some(2)));
        assert_ne!(
            fingerprint(true, "Acme", Some(2)),
            fingerprint(true, "Acme", Some(3))
        );
        assert_eq!(baseline, fingerprint(true, "Acme", None));
    }

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

    /// The manifest shape is `deny_unknown_fields` on the structs it decodes
    /// into. `document` is untyped there, so its shape is the producing
    /// engine's own type, checked where the manifest is validated.
    #[test]
    fn manifest_shape_rejects_unknown_nested_and_document_fields() {
        let document = json!({
            "pdfType": "text_based",
            "confidence": 1.0,
            "pageCount": 1,
            "pagesNeedingOcr": [],
            "ocrReasonsByPage": [{"page": 1, "reasons": []}],
            "hasEncodingIssues": false,
            "isComplex": false,
            "pagesWithTables": [],
            "pagesWithColumns": [],
            "processingTimeMs": 1
        });
        let valid = json!({
            "schemaVersion": 1,
            "jobId": Uuid::nil(),
            "attemptId": Uuid::nil(),
            "clientRunId": Uuid::nil(),
            "profile": "standard",
            "source": {"mediaType": "application/pdf", "byteLength": 1, "sha256": "a".repeat(64)},
            "engine": {"name": "pdf-inspector", "version": "1.25.2", "features": []},
            "route": {"kind": "local_pdf", "reasonCodes": []},
            "document": document,
            "warnings": [],
            "output": {"mediaType": "text/markdown", "byteLength": 1, "sha256": "b".repeat(64)},
            "startedAt": "1970-01-01T00:00:00Z",
            "completedAt": "1970-01-01T00:00:00Z"
        });
        assert!(serde_json::from_value::<ConversionManifest>(valid.clone()).is_ok());
        assert!(serde_json::from_value::<Inspection>(document.clone()).is_ok());

        for path in ["top", "source", "engine", "route", "output"] {
            let mut mutated = valid.clone();
            match path {
                "top" => mutated["unexpected"] = json!(true),
                other => mutated[other]["unexpected"] = json!(true),
            }
            assert!(
                serde_json::from_value::<ConversionManifest>(mutated).is_err(),
                "an unknown {path} field must not decode"
            );
        }

        for path in ["document", "page"] {
            let mut mutated = document.clone();
            match path {
                "document" => mutated["unexpected"] = json!(true),
                _ => mutated["ocrReasonsByPage"][0]["unexpected"] = json!(true),
            }
            assert!(
                serde_json::from_value::<Inspection>(mutated).is_err(),
                "an unknown {path} field must not decode"
            );
        }
    }

    #[test]
    fn markdown_limits_preserve_valid_history_and_bound_new_publications() {
        let stored = StoredArtifact {
            attempt_id: Uuid::nil(),
            kind: ArtifactKind::Markdown,
            relative_path: "result.md".to_owned(),
            media_type: super::MARKDOWN_MEDIA_TYPE.to_owned(),
            byte_length: 10,
            sha256: "a".repeat(64),
            created_at: "created".to_owned(),
        };
        let output = ManifestSource {
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

        let mismatched = ManifestSource {
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
