use std::sync::Arc;

use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};

use crate::{
    artifacts::{ArtifactError, ArtifactStore},
    auth::{AuthLoadError, BootstrapAuth},
    config::{Limits, Settings},
    conversion::ConversionService,
    engines::{AnyDocEngine, EngineStartupError, PdfInspectorEngine, VisionEngine},
    faults::FaultBarrier,
    jobs::{JobRuntime, StartupRecovery},
    persistence::{RepositoryError, SqliteRepository},
};

#[derive(Clone, Debug)]
pub struct AppState {
    auth: BootstrapAuth,
    service: ConversionService,
    limits: Limits,
    upload_permits: Arc<Semaphore>,
    jobs: JobRuntime,
}

impl AppState {
    pub async fn initialize(settings: &Settings) -> Result<Self, StartupError> {
        let auth = BootstrapAuth::load(&settings.token_file)?;
        let artifacts = ArtifactStore::initialize(&settings.data_dir)?;
        let max_active_jobs = u32::try_from(settings.limits.max_jobs)
            .map_err(|_| StartupError::JobLimitOutOfRange)?;
        let repository = SqliteRepository::open(
            &settings.data_dir,
            max_active_jobs,
            settings.database_busy_timeout,
        )
        .await?;
        let pdf_engine = PdfInspectorEngine::initialize(
            settings.pdf_worker_path.clone(),
            settings.pdf_bcmaps_dir.clone(),
            settings.limits.pdf_timeout,
            settings.limits.max_output_bytes,
            settings.pdf_threads,
        )?;
        // AnyDoc runs in-process: pure Rust, typed errors, internal resource
        // limits, and `catch_unwind` at the adapter. Parser concurrency one.
        // The hard timeout reuses the worker timeout: an in-process call
        // cannot be killed, so a hang fails the job and leaves a detached
        // blocking task bounded by AnyDoc's internal limits.
        let anydoc_engine = AnyDocEngine::new(
            settings.limits.max_output_bytes,
            1,
            settings.limits.pdf_timeout,
        );

        // Vision is optional and its absence is the normal case: no worker
        // binary, or a host below macOS 26, and the engine is simply not
        // there. A broken one is logged and dropped for the same reason.
        // Failing startup over it would take the whole service down for the
        // formats it never touches.
        let vision_engine = settings.vision_worker_path.as_ref().and_then(|path| {
            match VisionEngine::initialize(
                path.clone(),
                settings.limits.pdf_timeout,
                settings.limits.max_output_bytes,
            ) {
                Ok(engine) => Some(engine),
                Err(error) => {
                    tracing::warn!(%error, "vision engine is unavailable; image conversion is off");
                    None
                }
            }
        });

        let service = ConversionService::new(
            repository,
            artifacts,
            pdf_engine,
            anydoc_engine,
            vision_engine,
            settings.limits.max_output_bytes,
            // A parse that outlives its own hard timeout keeps the permit while
            // it detaches. Give the next claim one more timeout to wait, then
            // exit so startup recovery requeues instead of the runner freezing.
            settings.limits.pdf_timeout,
        );
        StartupRecovery::new(settings.recovery_limit)
            .run(&service)
            .await
            .map_err(|source| StartupError::Recovery(Box::new(source)))?;
        let jobs = JobRuntime::spawn(service.clone(), settings.worker_poll_interval);

        Ok(Self {
            auth,
            service,
            limits: settings.limits.clone(),
            upload_permits: Arc::new(Semaphore::new(settings.limits.max_concurrent_uploads)),
            jobs,
        })
    }

    pub(crate) fn auth(&self) -> &BootstrapAuth {
        &self.auth
    }

    pub(crate) fn service(&self) -> &ConversionService {
        &self.service
    }

    pub(crate) fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Test-only. The barrier is disarmed unless a test arms it, so the live path
    /// only ever pays for the loads inside [`FaultBarrier::hold`].
    pub fn fault_barrier(&self) -> Arc<FaultBarrier> {
        self.service.fault_barrier()
    }

    pub(crate) fn try_acquire_upload(&self) -> Result<OwnedSemaphorePermit, TryAcquireError> {
        self.upload_permits.clone().try_acquire_owned()
    }

    pub fn stop_job_claiming(&self) {
        self.jobs.stop_claiming();
    }

    pub fn force_cancel_jobs(&self) {
        self.jobs.force_cancel();
    }

    pub async fn shutdown_jobs(&self, grace: std::time::Duration) {
        self.jobs.shutdown(grace).await;
    }

    pub async fn shutdown_jobs_until(&self, deadline: tokio::time::Instant) {
        self.jobs.shutdown_until(deadline).await;
    }

    pub fn job_runner_failed(&self) -> bool {
        self.jobs.failed()
    }

    pub fn job_runner_stopped(&self) -> bool {
        self.jobs.stopped()
    }

    pub async fn wait_for_job_runner_exit(&self) -> bool {
        self.jobs.wait_until_stopped().await
    }

    pub async fn wait_for_job_runner_idle(&self) {
        self.jobs.wait_until_idle().await;
    }
}

#[derive(Debug, Error)]
pub enum StartupError {
    #[error(transparent)]
    Authentication(#[from] AuthLoadError),
    #[error(transparent)]
    Artifacts(#[from] ArtifactError),
    #[error(transparent)]
    Persistence(#[from] RepositoryError),
    #[error(transparent)]
    PdfEngine(#[from] EngineStartupError),
    #[error("startup recovery failed")]
    Recovery(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("configured active job limit does not fit the persistence layer")]
    JobLimitOutOfRange,
}
