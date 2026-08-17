use std::{path::PathBuf, sync::Arc};

use tempfile::{Builder, TempDir};
use thiserror::Error;
use tokio::fs;
use uuid::Uuid;

use crate::worker_protocol::MARKDOWN_FILE;

pub const MANIFEST_FILE: &str = "manifest.json";

#[derive(Clone, Debug)]
pub struct ArtifactStore {
    session: Arc<TempDir>,
}

impl ArtifactStore {
    pub fn initialize(parent: &std::path::Path) -> Result<Self, ArtifactError> {
        std::fs::create_dir_all(parent).map_err(ArtifactError::Initialize)?;
        let session = Builder::new()
            .prefix("tool-kit-converter-")
            .tempdir_in(parent)
            .map_err(ArtifactError::Initialize)?;

        Ok(Self {
            session: Arc::new(session),
        })
    }

    pub async fn create_attempt(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<AttemptPaths, ArtifactError> {
        let attempt = self
            .session
            .path()
            .join("jobs")
            .join(job_id.to_string())
            .join("attempts")
            .join(attempt_id.to_string());
        fs::create_dir_all(&attempt)
            .await
            .map_err(ArtifactError::CreateAttempt)?;
        set_private_directory(&attempt).map_err(ArtifactError::CreateAttempt)?;

        Ok(AttemptPaths {
            source: attempt.join("source.pdf"),
            publication_staging: attempt.join("publication.staging"),
            published: attempt.join("artifacts"),
            attempt,
        })
    }

    pub async fn discard(&self, paths: &AttemptPaths) {
        if let Err(error) = fs::remove_dir_all(&paths.attempt).await {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(job_artifact_cleanup = true, %error, "failed to remove attempt scratch");
            }
        }
    }

    pub async fn prepare_publication(&self, paths: &AttemptPaths) -> Result<(), ArtifactError> {
        fs::create_dir(&paths.publication_staging)
            .await
            .map_err(ArtifactError::PreparePublication)?;
        set_private_directory(&paths.publication_staging).map_err(ArtifactError::PreparePublication)
    }

    pub async fn publish(&self, paths: &AttemptPaths) -> Result<(), ArtifactError> {
        validate_publication_contents(&paths.publication_staging).await?;
        fs::rename(&paths.publication_staging, &paths.published)
            .await
            .map_err(ArtifactError::Publish)
    }
}

async fn validate_publication_contents(path: &std::path::Path) -> Result<(), ArtifactError> {
    let mut entries = fs::read_dir(path)
        .await
        .map_err(ArtifactError::ValidatePublication)?;
    let mut markdown = false;
    let mut manifest = false;
    let mut count = 0_usize;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(ArtifactError::ValidatePublication)?
    {
        count += 1;
        let file_type = entry
            .file_type()
            .await
            .map_err(ArtifactError::ValidatePublication)?;
        if !file_type.is_file() || file_type.is_symlink() {
            return Err(ArtifactError::UnexpectedPublicationContents);
        }
        if entry.file_name() == std::ffi::OsStr::new(MARKDOWN_FILE) {
            markdown = true;
        } else if entry.file_name() == std::ffi::OsStr::new(MANIFEST_FILE) {
            manifest = true;
        } else {
            return Err(ArtifactError::UnexpectedPublicationContents);
        }
    }
    if count == 2 && markdown && manifest {
        Ok(())
    } else {
        Err(ArtifactError::UnexpectedPublicationContents)
    }
}

#[derive(Clone, Debug)]
pub struct AttemptPaths {
    pub attempt: PathBuf,
    pub source: PathBuf,
    pub publication_staging: PathBuf,
    pub published: PathBuf,
}

impl AttemptPaths {
    pub fn staged_markdown(&self) -> PathBuf {
        self.publication_staging.join(MARKDOWN_FILE)
    }

    pub fn staged_manifest(&self) -> PathBuf {
        self.publication_staging.join(MANIFEST_FILE)
    }

    pub fn markdown(&self) -> PathBuf {
        self.published.join(MARKDOWN_FILE)
    }

    pub fn manifest(&self) -> PathBuf {
        self.published.join(MANIFEST_FILE)
    }
}

#[cfg(unix)]
fn set_private_directory(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_private_directory(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

#[derive(Debug, Error)]
pub enum ArtifactError {
    #[error("cannot initialize ephemeral artifact storage")]
    Initialize(#[source] std::io::Error),
    #[error("cannot create attempt storage")]
    CreateAttempt(#[source] std::io::Error),
    #[error("cannot prepare artifact publication")]
    PreparePublication(#[source] std::io::Error),
    #[error("cannot atomically publish artifacts")]
    Publish(#[source] std::io::Error),
    #[error("cannot validate staged artifact publication")]
    ValidatePublication(#[source] std::io::Error),
    #[error("staged artifact publication contains unexpected entries")]
    UnexpectedPublicationContents,
}
