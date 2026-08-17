use std::sync::Arc;

use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};

use crate::{
    artifacts::{ArtifactError, ArtifactStore},
    auth::{AuthLoadError, BootstrapAuth},
    config::{Limits, Settings},
    conversion::ConversionService,
    engines::{EngineStartupError, PdfInspectorEngine},
    persistence::{RepositoryError, SqliteRepository},
};

#[derive(Clone, Debug)]
pub struct AppState {
    auth: BootstrapAuth,
    service: ConversionService,
    limits: Limits,
    upload_permits: Arc<Semaphore>,
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
        let engine = PdfInspectorEngine::initialize(
            settings.pdf_worker_path.clone(),
            settings.pdf_bcmaps_dir.clone(),
            settings.limits.pdf_timeout,
            settings.limits.max_output_bytes,
            settings.pdf_threads,
        )?;

        Ok(Self {
            auth,
            service: ConversionService::new(repository, artifacts, engine),
            limits: settings.limits.clone(),
            upload_permits: Arc::new(Semaphore::new(settings.limits.max_concurrent_uploads)),
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

    pub(crate) fn try_acquire_upload(&self) -> Result<OwnedSemaphorePermit, TryAcquireError> {
        self.upload_permits.clone().try_acquire_owned()
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
    #[error("configured active job limit does not fit the persistence layer")]
    JobLimitOutOfRange,
}
