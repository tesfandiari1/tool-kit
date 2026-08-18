use thiserror::Error;
use uuid::Uuid;

use crate::{
    artifacts::{ArtifactError, PublicationState},
    conversion::{ArtifactReadFailure, ConversionService},
    persistence::{
        AttemptState, ConversionState, FailedResult, FailureStage, RepositoryError, RequeueOutcome,
        StoredConversion, StoredFailure,
    },
};

const SOURCE_INTEGRITY_CODE: &str = "source_integrity_failed";
const SOURCE_INTEGRITY_MESSAGE: &str = "The immutable source failed integrity validation.";
const ARTIFACT_INTEGRITY_CODE: &str = "artifact_integrity_failed";
const ARTIFACT_INTEGRITY_MESSAGE: &str = "A published artifact failed integrity validation.";
const RECOVERY_LIMIT_CODE: &str = "recovery_limit_exceeded";
const RECOVERY_LIMIT_MESSAGE: &str =
    "The conversion exceeded the configured startup recovery limit.";

#[derive(Clone, Copy, Debug)]
pub(crate) struct StartupRecovery {
    recovery_limit: usize,
}

impl StartupRecovery {
    pub(crate) fn new(recovery_limit: usize) -> Self {
        Self { recovery_limit }
    }

    pub(crate) async fn run(self, service: &ConversionService) -> Result<(), StartupRecoveryError> {
        self.quarantine_orphans(service).await?;
        let candidates = service
            .recovery_repository()
            .list_recovery_candidates()
            .await?;
        for job in candidates {
            Box::pin(self.reconcile_job(service, job)).await?;
        }
        Ok(())
    }

    async fn quarantine_orphans(
        self,
        service: &ConversionService,
    ) -> Result<(), StartupRecoveryError> {
        let database_ids = service.recovery_repository().list_conversion_ids().await?;
        for job_id in service.recovery_artifacts().list_owned_job_ids().await? {
            if database_ids.contains(&job_id) {
                continue;
            }
            let quarantine = service
                .recovery_artifacts()
                .quarantine_preacceptance(job_id)
                .await?;
            tracing::warn!(
                %job_id,
                quarantine = ?quarantine,
                "quarantined backend storage with no durable conversion owner"
            );
        }
        Ok(())
    }

    async fn reconcile_job(
        self,
        service: &ConversionService,
        job: StoredConversion,
    ) -> Result<(), StartupRecoveryError> {
        validate_candidate_invariants(&job)?;
        match job.state {
            ConversionState::Queued => Ok(()),
            ConversionState::ConvertingLocal => {
                if !source_is_valid(service, &job).await? {
                    return fail_source_integrity(service, &job, FailureStage::ConvertingLocal)
                        .await;
                }
                self.requeue(service, &job, FailureStage::ConvertingLocal)
                    .await
            }
            ConversionState::Finalizing => self.reconcile_finalizing(service, &job).await,
            ConversionState::Succeeded => match service.validate_stored_artifacts(&job).await {
                Ok(()) => Ok(()),
                Err(ArtifactReadFailure::Integrity) => {
                    service
                        .recovery_repository()
                        .mark_artifact_integrity_failed(
                            job.id,
                            job.active_attempt.id,
                            StoredFailure {
                                code: ARTIFACT_INTEGRITY_CODE.to_owned(),
                                message: ARTIFACT_INTEGRITY_MESSAGE.to_owned(),
                            },
                        )
                        .await?;
                    tracing::warn!(
                        job_id = %job.id,
                        attempt_id = %job.active_attempt.id,
                        failure_code = ARTIFACT_INTEGRITY_CODE,
                        "startup recovery failed a corrupted successful conversion"
                    );
                    Ok(())
                }
                Err(ArtifactReadFailure::Transient(error)) => Err(error.into()),
            },
            state => Err(StartupRecoveryError::UnexpectedCandidateState {
                job_id: job.id,
                attempt_id: job.active_attempt.id,
                state,
            }),
        }
    }

    async fn reconcile_finalizing(
        self,
        service: &ConversionService,
        job: &StoredConversion,
    ) -> Result<(), StartupRecoveryError> {
        if !source_is_valid(service, job).await? {
            return fail_source_integrity(service, job, FailureStage::Finalizing).await;
        }

        let publication = match service
            .recovery_artifacts()
            .inspect_publication(job.id, job.active_attempt.id)
            .await
        {
            Ok(publication) => publication,
            Err(error) => match crate::conversion::classify_artifact_error(error) {
                ArtifactReadFailure::Integrity => {
                    return self.requeue(service, job, FailureStage::Finalizing).await;
                }
                ArtifactReadFailure::Transient(error) => return Err(error.into()),
            },
        };

        match publication {
            PublicationState::Published => {
                match service.validate_finalizing_publication(job).await {
                    Ok(artifacts) => {
                        service
                            .recovery_repository()
                            .finish_succeeded(job.id, job.active_attempt.id, artifacts)
                            .await?;
                        tracing::info!(
                            job_id = %job.id,
                            attempt_id = %job.active_attempt.id,
                            "startup recovery completed a durable artifact publication"
                        );
                        Ok(())
                    }
                    Err(ArtifactReadFailure::Integrity) => {
                        self.requeue(service, job, FailureStage::Finalizing).await
                    }
                    Err(ArtifactReadFailure::Transient(error)) => Err(error.into()),
                }
            }
            PublicationState::StagingOnly | PublicationState::Conflicting => {
                service
                    .recovery_artifacts()
                    .discard_publication_staging(job.id, job.active_attempt.id)
                    .await?;
                self.requeue(service, job, FailureStage::Finalizing).await
            }
            PublicationState::Missing => self.requeue(service, job, FailureStage::Finalizing).await,
        }
    }

    async fn requeue(
        self,
        service: &ConversionService,
        job: &StoredConversion,
        stage: FailureStage,
    ) -> Result<(), StartupRecoveryError> {
        match service
            .recovery_repository()
            .interrupt_and_requeue(
                job.id,
                job.active_attempt.id,
                Uuid::new_v4(),
                self.recovery_limit,
            )
            .await?
        {
            RequeueOutcome::Requeued(requeued) => {
                tracing::info!(
                    job_id = %job.id,
                    interrupted_attempt_id = %job.active_attempt.id,
                    recovery_attempt_id = %requeued.active_attempt.id,
                    "startup recovery requeued an interrupted conversion"
                );
                Ok(())
            }
            RequeueOutcome::LimitReached(unchanged) => {
                if unchanged.id != job.id
                    || unchanged.active_attempt.id != job.active_attempt.id
                    || unchanged.state != stage.conversion_state()
                {
                    return Err(StartupRecoveryError::RecoveryLimitStateChanged {
                        job_id: job.id,
                        attempt_id: job.active_attempt.id,
                    });
                }
                service
                    .recovery_repository()
                    .finish_failed(
                        job.id,
                        job.active_attempt.id,
                        FailedResult {
                            stage,
                            failure: StoredFailure {
                                code: RECOVERY_LIMIT_CODE.to_owned(),
                                message: RECOVERY_LIMIT_MESSAGE.to_owned(),
                            },
                        },
                    )
                    .await?;
                tracing::warn!(
                    job_id = %job.id,
                    attempt_id = %job.active_attempt.id,
                    failure_code = RECOVERY_LIMIT_CODE,
                    "startup recovery limit reached"
                );
                Ok(())
            }
        }
    }
}

fn validate_candidate_invariants(job: &StoredConversion) -> Result<(), StartupRecoveryError> {
    let attempt = &job.active_attempt;
    let paired = matches!(
        (job.state, attempt.state),
        (ConversionState::Queued, AttemptState::Queued)
            | (
                ConversionState::ConvertingLocal,
                AttemptState::ConvertingLocal
            )
            | (ConversionState::Finalizing, AttemptState::Finalizing)
            | (ConversionState::Succeeded, AttemptState::Succeeded)
    );
    if !paired {
        return Err(StartupRecoveryError::AttemptStateMismatch {
            job_id: job.id,
            attempt_id: job.active_attempt.id,
            conversion_state: job.state,
            attempt_state: job.active_attempt.state,
        });
    }
    if job.state != ConversionState::Succeeded && !job.artifacts.is_empty() {
        return Err(StartupRecoveryError::UnexpectedArtifactRows {
            job_id: job.id,
            attempt_id: job.active_attempt.id,
            state: job.state,
            count: job.artifacts.len(),
        });
    }

    if job.failure.is_some() || attempt.failure.is_some() {
        return invalid_metadata(job, "active state contains failure metadata");
    }
    let route_matches = job.route == attempt.route;
    let analysis_matches = job.reason_codes == attempt.reason_codes
        && job.warnings == attempt.warnings
        && attempt
            .classification
            .as_deref()
            .is_some_and(is_known_classification);
    let metadata_valid = match job.state {
        ConversionState::Queued => {
            job.route.is_none()
                && job.reason_codes.is_empty()
                && job.warnings.is_empty()
                && attempt.engine.is_none()
                && attempt.route.is_none()
                && attempt.classification.is_none()
                && attempt.inspection.is_none()
                && attempt.reason_codes.is_empty()
                && attempt.warnings.is_empty()
                && attempt.fallback_reason.is_none()
                && attempt.started_at.is_none()
                && attempt.finished_at.is_none()
        }
        ConversionState::ConvertingLocal => {
            route_matches
                && job.route.is_some()
                && job.reason_codes.is_empty()
                && job.warnings.is_empty()
                && attempt.engine.is_some()
                && attempt.classification.is_none()
                && attempt.inspection.is_none()
                && attempt.reason_codes.is_empty()
                && attempt.warnings.is_empty()
                && attempt.fallback_reason.is_none()
                && attempt.started_at.is_some()
                && attempt.finished_at.is_none()
        }
        ConversionState::Finalizing => {
            route_matches
                && job.route.is_some()
                && analysis_matches
                && attempt.engine.is_some()
                && attempt.inspection.is_some()
                && attempt.fallback_reason.is_none()
                && attempt.started_at.is_some()
                && attempt.finished_at.is_none()
        }
        ConversionState::Succeeded => {
            route_matches
                && job.route.is_some()
                && analysis_matches
                && attempt.engine.is_some()
                && attempt.inspection.is_some()
                && attempt.fallback_reason.is_none()
                && attempt.started_at.is_some()
                && attempt.finished_at.is_some()
        }
        ConversionState::Failed | ConversionState::NeedsRemote => false,
    };
    if !metadata_valid {
        return invalid_metadata(job, "active state metadata is incomplete or inconsistent");
    }
    Ok(())
}

fn is_known_classification(value: &str) -> bool {
    matches!(value, "text_based" | "scanned" | "image_based" | "mixed")
}

fn invalid_metadata<T>(
    job: &StoredConversion,
    reason: &'static str,
) -> Result<T, StartupRecoveryError> {
    Err(StartupRecoveryError::PersistedMetadataInvariant {
        job_id: job.id,
        attempt_id: job.active_attempt.id,
        state: job.state,
        reason,
    })
}

async fn source_is_valid(
    service: &ConversionService,
    job: &StoredConversion,
) -> Result<bool, StartupRecoveryError> {
    match service.validate_recovery_source(job).await {
        Ok(()) => Ok(true),
        Err(ArtifactReadFailure::Integrity) => Ok(false),
        Err(ArtifactReadFailure::Transient(error)) => Err(error.into()),
    }
}

async fn fail_source_integrity(
    service: &ConversionService,
    job: &StoredConversion,
    stage: FailureStage,
) -> Result<(), StartupRecoveryError> {
    service
        .recovery_repository()
        .finish_failed(
            job.id,
            job.active_attempt.id,
            FailedResult {
                stage,
                failure: StoredFailure {
                    code: SOURCE_INTEGRITY_CODE.to_owned(),
                    message: SOURCE_INTEGRITY_MESSAGE.to_owned(),
                },
            },
        )
        .await?;
    tracing::warn!(
        job_id = %job.id,
        attempt_id = %job.active_attempt.id,
        failure_code = SOURCE_INTEGRITY_CODE,
        "startup recovery failed a conversion with a corrupted source"
    );
    Ok(())
}

#[derive(Debug, Error)]
pub(crate) enum StartupRecoveryError {
    #[error(transparent)]
    Artifacts(#[from] ArtifactError),
    #[error(transparent)]
    Persistence(#[from] RepositoryError),
    #[error(
        "startup recovery returned unsupported conversion state {state:?} for {job_id}/{attempt_id}"
    )]
    UnexpectedCandidateState {
        job_id: Uuid,
        attempt_id: Uuid,
        state: ConversionState,
    },
    #[error("startup recovery limit result changed conversion {job_id}/{attempt_id}")]
    RecoveryLimitStateChanged { job_id: Uuid, attempt_id: Uuid },
    #[error(
        "startup recovery found mismatched states for {job_id}/{attempt_id}: conversion={conversion_state:?}, attempt={attempt_state:?}"
    )]
    AttemptStateMismatch {
        job_id: Uuid,
        attempt_id: Uuid,
        conversion_state: ConversionState,
        attempt_state: AttemptState,
    },
    #[error(
        "startup recovery found {count} artifact rows for non-success state {state:?} at {job_id}/{attempt_id}"
    )]
    UnexpectedArtifactRows {
        job_id: Uuid,
        attempt_id: Uuid,
        state: ConversionState,
        count: usize,
    },
    #[error(
        "startup recovery found invalid persisted metadata for {state:?} conversion {job_id}/{attempt_id}: {reason}"
    )]
    PersistedMetadataInvariant {
        job_id: Uuid,
        attempt_id: Uuid,
        state: ConversionState,
        reason: &'static str,
    },
}
