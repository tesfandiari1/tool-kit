use std::sync::Arc;

use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};

use crate::{
    artifacts::{ArtifactError, ArtifactStore},
    auth::{AuthLoadError, BootstrapAuth},
    config::{Limits, Settings},
    conversion::{ConversionService, JobRegistry},
    engines::{EngineStartupError, PdfInspectorEngine},
};

#[derive(Clone, Debug)]
pub struct AppState {
    auth: BootstrapAuth,
    service: ConversionService,
    limits: Limits,
    upload_permits: Arc<Semaphore>,
}

impl AppState {
    pub fn initialize(settings: &Settings) -> Result<Self, StartupError> {
        let auth = BootstrapAuth::load(&settings.token_file)?;
        let artifacts = ArtifactStore::initialize(&settings.scratch_parent)?;
        let engine = PdfInspectorEngine::initialize(
            settings.pdf_worker_path.clone(),
            settings.pdf_bcmaps_dir.clone(),
            settings.limits.pdf_timeout,
            settings.limits.max_output_bytes,
            settings.pdf_threads,
        )?;
        let registry = JobRegistry::new(settings.limits.max_jobs);

        Ok(Self {
            auth,
            service: ConversionService::new(registry, artifacts, engine),
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
    PdfEngine(#[from] EngineStartupError),
}
