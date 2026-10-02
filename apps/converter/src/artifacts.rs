use std::{
    ffi::OsStr,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
};
use uuid::Uuid;

use crate::worker_protocol::MARKDOWN_FILE;

const MANIFEST_FILE: &str = "manifest.json";

const JOBS_DIRECTORY: &str = "jobs";
const SOURCE_DIRECTORY: &str = "source";
const SOURCE_FILE: &str = "input";
const SOURCE_STAGING_FILE: &str = "input.staging";
const ATTEMPTS_DIRECTORY: &str = "attempts";
const PUBLICATION_STAGING_DIRECTORY: &str = "publication.staging";
const ARTIFACTS_DIRECTORY: &str = "artifacts";
const QUARANTINE_DIRECTORY: &str = "quarantine";
const PREACCEPTANCE_DIRECTORY: &str = "pre-acceptance";
/// Readiness probe files live here, outside `jobs/`, so nothing that scans job
/// storage can read a probe as conversion state.
const HEALTH_DIRECTORY: &str = ".health";

#[derive(Clone, Debug)]
pub struct ArtifactStore {
    root: Arc<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobPaths {
    pub job: PathBuf,
    pub source_directory: PathBuf,
    pub source_staging: PathBuf,
    pub source: PathBuf,
    pub attempts: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedSubmission {
    pub job_id: Uuid,
    pub paths: JobPaths,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublishedArtifact {
    Markdown,
    Manifest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicationState {
    Missing,
    StagingOnly,
    Published,
    Conflicting,
}

impl PublishedArtifact {
    fn file_name(self) -> &'static str {
        match self {
            Self::Markdown => MARKDOWN_FILE,
            Self::Manifest => MANIFEST_FILE,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedFile {
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Debug)]
pub struct ValidatedOpenFile {
    pub file: fs::File,
    pub byte_length: u64,
    pub sha256: String,
}

impl ValidatedOpenFile {
    fn without_handle(self) -> ValidatedFile {
        ValidatedFile {
            byte_length: self.byte_length,
            sha256: self.sha256,
        }
    }
}

impl ArtifactStore {
    pub fn initialize(root: &Path) -> Result<Self, ArtifactError> {
        // The data root defaults to `/data`, which exists only in the
        // container, so the path is the whole diagnosis of a native start-up
        // failure.
        std::fs::create_dir_all(root).map_err(initialize_error(root))?;
        let metadata = std::fs::symlink_metadata(root).map_err(initialize_error(root))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(ArtifactError::InvalidDataRoot(root.to_owned()));
        }
        let root = std::fs::canonicalize(root).map_err(initialize_error(root))?;
        let store = Self {
            root: Arc::new(root),
        };

        ensure_private_directory(&store.root.join(JOBS_DIRECTORY))?;
        let quarantine = store.root.join(QUARANTINE_DIRECTORY);
        ensure_private_directory(&quarantine)?;
        ensure_private_directory(&quarantine.join(PREACCEPTANCE_DIRECTORY))?;
        ensure_private_directory(&store.root.join(HEALTH_DIRECTORY))?;

        Ok(store)
    }

    /// Proves the data root still accepts writes by creating and removing one
    /// probe file. It never reads, writes, or removes anything under `jobs/`,
    /// so readiness cannot damage a stored conversion.
    pub async fn probe_writable(&self) -> Result<(), ArtifactError> {
        let directory = self.root.join(HEALTH_DIRECTORY);
        match optional_metadata(&directory).await? {
            Some(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(ArtifactError::UnsafeOwnedPath(directory));
            }
            Some(_) => {}
            // A probe directory removed while the service runs is recreated
            // rather than latched as permanently unready.
            None => match create_private_directory(&directory).await {
                Ok(()) => {}
                Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(source) => {
                    return Err(ArtifactError::HealthProbe {
                        path: directory,
                        source,
                    })
                }
            },
        }

        // The guard owns the removal so a failed write or a cancelled readiness
        // check still leaves the directory empty. Readiness runs under a
        // timeout, and a dropped future never resumes to run async cleanup.
        let mut probe = ProbeFile {
            path: directory.join(Uuid::new_v4().to_string()),
            removed: false,
        };
        write_health_probe(&probe.path)
            .await
            .map_err(|source| ArtifactError::HealthProbe {
                path: probe.path.clone(),
                source,
            })?;
        fs::remove_file(&probe.path)
            .await
            .map_err(|source| ArtifactError::HealthProbe {
                path: probe.path.clone(),
                source,
            })?;
        probe.removed = true;
        Ok(())
    }

    fn job_paths(&self, job_id: Uuid) -> JobPaths {
        let job = self.root.join(job_relative_path(job_id));
        let source_directory = job.join(SOURCE_DIRECTORY);
        JobPaths {
            source_staging: source_directory.join(SOURCE_STAGING_FILE),
            source: source_directory.join(SOURCE_FILE),
            attempts: job.join(ATTEMPTS_DIRECTORY),
            source_directory,
            job,
        }
    }

    pub fn attempt_paths(&self, job_id: Uuid, attempt_id: Uuid) -> AttemptPaths {
        let job = self.job_paths(job_id);
        let attempt = job.attempts.join(attempt_id.to_string());
        AttemptPaths {
            source: job.source,
            publication_staging: attempt.join(PUBLICATION_STAGING_DIRECTORY),
            published: attempt.join(ARTIFACTS_DIRECTORY),
            attempt,
        }
    }

    pub async fn prepare_submission(
        &self,
        job_id: Uuid,
    ) -> Result<PreparedSubmission, ArtifactError> {
        self.require_owned_directory_relative(Path::new(JOBS_DIRECTORY))
            .await?;
        let paths = self.job_paths(job_id);
        create_private_directory(&paths.job)
            .await
            .map_err(|source| ArtifactError::CreateSubmission {
                path: paths.job.clone(),
                source,
            })?;

        if let Err(error) = self.create_submission_children(&paths).await {
            if let Err(cleanup_error) = fs::remove_dir_all(&paths.job).await {
                tracing::warn!(
                    %job_id,
                    %cleanup_error,
                    "failed to clean partially prepared submission storage"
                );
            }
            return Err(error);
        }

        Ok(PreparedSubmission { job_id, paths })
    }

    pub async fn publish_source(
        &self,
        submission: &PreparedSubmission,
    ) -> Result<ValidatedFile, ArtifactError> {
        let expected_paths = self.job_paths(submission.job_id);
        if submission.paths != expected_paths {
            return Err(ArtifactError::PathLayoutMismatch);
        }

        self.resolve_existing_relative(&source_staging_relative_path(submission.job_id))
            .await?;
        sync_regular_file(&submission.paths.source_staging)
            .await
            .map_err(|source| ArtifactError::PublishSource {
                path: submission.paths.source_staging.clone(),
                source,
            })?;
        require_absent(&submission.paths.source).await?;
        fs::rename(&submission.paths.source_staging, &submission.paths.source)
            .await
            .map_err(|source| ArtifactError::PublishSource {
                path: submission.paths.source.clone(),
                source,
            })?;
        sync_directory(submission.paths.source_directory.clone())
            .await
            .map_err(|source| ArtifactError::SourceCommittedButNotSynced {
                path: submission.paths.source_directory.clone(),
                source,
            })?;

        self.validate_source(submission.job_id, None, None).await
    }

    /// Refuses to create an attempt until the immutable source exists as a
    /// regular, non-symlink file, so a retry cannot run against a missing or
    /// substituted source.
    pub async fn create_retry_attempt(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<AttemptPaths, ArtifactError> {
        let _source = self
            .open_owned_regular_relative(&source_relative_path(job_id))
            .await?;
        self.create_attempt_directory(job_id, attempt_id).await
    }

    async fn create_attempt_directory(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<AttemptPaths, ArtifactError> {
        let paths = self.attempt_paths(job_id, attempt_id);
        create_private_directory(&paths.attempt)
            .await
            .map_err(|source| ArtifactError::CreateAttempt {
                path: paths.attempt.clone(),
                source,
            })?;
        sync_directory(self.job_paths(job_id).attempts)
            .await
            .map_err(|source| ArtifactError::CreateAttempt {
                path: paths.attempt.clone(),
                source,
            })?;
        Ok(paths)
    }

    pub async fn prepare_artifacts(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<AttemptPaths, ArtifactError> {
        let paths = self
            .ensure_attempt_for_publication(job_id, attempt_id)
            .await?;
        require_absent(&paths.published).await?;
        create_private_directory(&paths.publication_staging)
            .await
            .map_err(|source| ArtifactError::PreparePublication {
                path: paths.publication_staging.clone(),
                source,
            })?;
        sync_directory(paths.attempt.clone())
            .await
            .map_err(|source| ArtifactError::PreparePublication {
                path: paths.attempt.clone(),
                source,
            })?;
        Ok(paths)
    }

    /// Inspects only deterministic, backend-owned publication directories.
    /// Every existing component is checked with `symlink_metadata`; a symlink
    /// or non-directory is corruption, not a publication state.
    pub async fn inspect_publication(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<PublicationState, ArtifactError> {
        if !self.owned_attempt_exists(job_id, attempt_id).await? {
            return Ok(PublicationState::Missing);
        }

        let staging = self
            .optional_owned_directory_relative(&publication_staging_relative_path(
                job_id, attempt_id,
            ))
            .await?;
        let published = self
            .optional_owned_directory_relative(&artifacts_relative_path(job_id, attempt_id))
            .await?;
        Ok(match (staging, published) {
            (false, false) => PublicationState::Missing,
            (true, false) => PublicationState::StagingOnly,
            (false, true) => PublicationState::Published,
            (true, true) => PublicationState::Conflicting,
        })
    }

    /// Removes only the ID-derived staging publication for one known attempt.
    /// Published artifacts and the shared immutable source are never touched.
    pub async fn discard_publication_staging(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<(), ArtifactError> {
        if !self.owned_attempt_exists(job_id, attempt_id).await? {
            return Ok(());
        }
        let relative = publication_staging_relative_path(job_id, attempt_id);
        if !self.optional_owned_directory_relative(&relative).await? {
            return Ok(());
        }
        let path = self.resolve_existing_relative(&relative).await?;
        fs::remove_dir_all(&path)
            .await
            .map_err(|source| ArtifactError::Discard {
                path: path.clone(),
                source,
            })?;
        sync_directory(self.attempt_paths(job_id, attempt_id).attempt)
            .await
            .map_err(|source| ArtifactError::Discard { path, source })
    }

    /// Lists direct canonical UUID job directories without following symlinks
    /// or interpreting any entry name as an arbitrary path.
    pub async fn list_owned_job_ids(&self) -> Result<Vec<Uuid>, ArtifactError> {
        let jobs = self
            .require_owned_directory_relative(Path::new(JOBS_DIRECTORY))
            .await
            .map(|_| self.root.join(JOBS_DIRECTORY))?;
        let mut entries =
            fs::read_dir(&jobs)
                .await
                .map_err(|source| ArtifactError::InspectPath {
                    path: jobs.clone(),
                    source,
                })?;
        let mut job_ids = Vec::new();
        while let Some(entry) =
            entries
                .next_entry()
                .await
                .map_err(|source| ArtifactError::InspectPath {
                    path: jobs.clone(),
                    source,
                })?
        {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Ok(job_id) = Uuid::parse_str(&name) else {
                continue;
            };
            if job_id.to_string() != name {
                continue;
            }
            let file_type =
                entry
                    .file_type()
                    .await
                    .map_err(|source| ArtifactError::InspectPath {
                        path: entry.path(),
                        source,
                    })?;
            if file_type.is_dir() && !file_type.is_symlink() {
                job_ids.push(job_id);
            }
        }
        job_ids.sort_unstable();
        Ok(job_ids)
    }

    /// Validates that one published artifact directory contains exactly the
    /// two backend-owned regular files and no symlinks or extra entries.
    pub async fn validate_published_contents(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<(), ArtifactError> {
        self.require_owned_attempt(job_id, attempt_id).await?;
        let relative = artifacts_relative_path(job_id, attempt_id);
        let published = self.resolve_existing_relative(&relative).await?;
        require_directory(&published).await?;
        validate_publication_contents(&published).await
    }

    pub async fn publish_artifacts(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<(), ArtifactError> {
        let paths = self.attempt_paths(job_id, attempt_id);
        self.require_owned_attempt(job_id, attempt_id).await?;
        self.resolve_existing_relative(&publication_staging_relative_path(job_id, attempt_id))
            .await?;
        validate_publication_contents(&paths.publication_staging).await?;
        sync_regular_file(&paths.staged_markdown())
            .await
            .map_err(|source| ArtifactError::Publish {
                path: paths.staged_markdown(),
                source,
            })?;
        sync_regular_file(&paths.staged_manifest())
            .await
            .map_err(|source| ArtifactError::Publish {
                path: paths.staged_manifest(),
                source,
            })?;
        sync_directory(paths.publication_staging.clone())
            .await
            .map_err(|source| ArtifactError::Publish {
                path: paths.publication_staging.clone(),
                source,
            })?;
        require_absent(&paths.published).await?;
        fs::rename(&paths.publication_staging, &paths.published)
            .await
            .map_err(|source| ArtifactError::Publish {
                path: paths.published.clone(),
                source,
            })?;
        sync_directory(paths.attempt.clone())
            .await
            .map_err(|source| ArtifactError::PublicationCommittedButNotSynced {
                path: paths.attempt,
                source,
            })
    }

    pub async fn validate_source(
        &self,
        job_id: Uuid,
        expected_byte_length: Option<u64>,
        expected_sha256: Option<&str>,
    ) -> Result<ValidatedFile, ArtifactError> {
        self.open_validated_source(job_id, expected_byte_length, expected_sha256)
            .await
            .map(ValidatedOpenFile::without_handle)
    }

    pub async fn open_validated_source(
        &self,
        job_id: Uuid,
        expected_byte_length: Option<u64>,
        expected_sha256: Option<&str>,
    ) -> Result<ValidatedOpenFile, ArtifactError> {
        self.open_validated_relative_file_with_limit(
            &source_relative_path(job_id),
            None,
            expected_byte_length,
            expected_sha256,
        )
        .await
    }

    /// Opens and hashes a published artifact only after its no-follow file
    /// metadata proves that the read is within the caller's byte limit.
    pub async fn open_bounded_validated_artifact(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
        artifact: PublishedArtifact,
        maximum_byte_length: u64,
        expected_byte_length: Option<u64>,
        expected_sha256: Option<&str>,
    ) -> Result<ValidatedOpenFile, ArtifactError> {
        self.open_validated_relative_file_with_limit(
            &artifact_relative_path(job_id, attempt_id, artifact),
            Some(maximum_byte_length),
            expected_byte_length,
            expected_sha256,
        )
        .await
    }

    fn resolve_relative(&self, relative: &Path) -> Result<PathBuf, ArtifactError> {
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(ArtifactError::InvalidRelativePath(relative.to_owned()));
        }
        let resolved = self.root.join(relative);
        if !resolved.starts_with(self.root.as_path()) {
            return Err(ArtifactError::InvalidRelativePath(relative.to_owned()));
        }
        Ok(resolved)
    }

    async fn resolve_existing_relative(&self, relative: &Path) -> Result<PathBuf, ArtifactError> {
        let resolved = self.resolve_relative(relative)?;
        let mut current = self.root.as_path().to_owned();
        // `resolve_relative` admitted only normal components.
        let count = relative.components().count();
        for (index, component) in relative.iter().enumerate() {
            current.push(component);
            let metadata = fs::symlink_metadata(&current).await.map_err(|source| {
                ArtifactError::InspectPath {
                    path: current.clone(),
                    source,
                }
            })?;
            if metadata.file_type().is_symlink() {
                return Err(ArtifactError::SymlinkPath(current));
            }
            if index + 1 < count && !metadata.is_dir() {
                return Err(ArtifactError::NotDirectory(current));
            }
        }
        Ok(resolved)
    }

    pub async fn discard_unaccepted_job(&self, job_id: Uuid) -> Result<(), ArtifactError> {
        self.discard_owned_directory(&job_relative_path(job_id), self.root.join(JOBS_DIRECTORY))
            .await
    }

    pub async fn discard_attempt(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<(), ArtifactError> {
        self.discard_owned_directory(
            &attempt_relative_path(job_id, attempt_id),
            self.job_paths(job_id).attempts,
        )
        .await
    }

    /// Removes one backend-owned directory. A symlink or a non-directory is
    /// corruption, so it is refused rather than followed.
    async fn discard_owned_directory(
        &self,
        relative: &Path,
        parent_to_sync: PathBuf,
    ) -> Result<(), ArtifactError> {
        if !self.optional_owned_directory_relative(relative).await? {
            return Ok(());
        }
        let path = self.resolve_relative(relative)?;
        fs::remove_dir_all(&path)
            .await
            .map_err(|source| ArtifactError::Discard {
                path: path.clone(),
                source,
            })?;
        sync_directory(parent_to_sync)
            .await
            .map_err(|source| ArtifactError::Discard { path, source })
    }

    pub async fn quarantine_preacceptance(&self, job_id: Uuid) -> Result<PathBuf, ArtifactError> {
        for _ in 0..8 {
            let quarantine_id = Uuid::new_v4();
            match self
                .quarantine_preacceptance_as(job_id, quarantine_id)
                .await
            {
                Err(ArtifactError::PathAlreadyExists(_)) => continue,
                result => return result,
            }
        }
        Err(ArtifactError::QuarantineReservationExhausted(job_id))
    }

    async fn quarantine_preacceptance_as(
        &self,
        job_id: Uuid,
        quarantine_id: Uuid,
    ) -> Result<PathBuf, ArtifactError> {
        self.require_owned_directory_relative(
            &PathBuf::from(QUARANTINE_DIRECTORY).join(PREACCEPTANCE_DIRECTORY),
        )
        .await?;
        let job_relative = job_relative_path(job_id);
        let job = self.resolve_existing_relative(&job_relative).await?;
        require_directory(&job).await?;
        let reservation_relative =
            preacceptance_quarantine_reservation_relative_path(job_id, quarantine_id);
        let reservation = self.resolve_relative(&reservation_relative)?;
        match create_private_directory(&reservation).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(ArtifactError::PathAlreadyExists(reservation));
            }
            Err(source) => {
                return Err(ArtifactError::Quarantine {
                    path: reservation,
                    source,
                });
            }
        }
        let quarantine_relative = preacceptance_quarantine_relative_path(job_id, quarantine_id);
        let quarantine = self.resolve_relative(&quarantine_relative)?;
        require_absent(&quarantine).await?;
        if let Err(source) = fs::rename(&job, &quarantine).await {
            let _ = fs::remove_dir(&reservation).await;
            return Err(ArtifactError::Quarantine { path: job, source });
        }
        sync_directory(self.root.join(JOBS_DIRECTORY))
            .await
            .map_err(|source| ArtifactError::Quarantine { path: job, source })?;
        sync_directory(reservation)
            .await
            .map_err(|source| ArtifactError::Quarantine {
                path: quarantine.clone(),
                source,
            })?;
        sync_directory(
            self.root
                .join(QUARANTINE_DIRECTORY)
                .join(PREACCEPTANCE_DIRECTORY),
        )
        .await
        .map_err(|source| ArtifactError::Quarantine {
            path: quarantine,
            source,
        })?;
        Ok(quarantine_relative)
    }

    async fn create_submission_children(&self, paths: &JobPaths) -> Result<(), ArtifactError> {
        self.require_owned_directory_relative(
            paths
                .job
                .strip_prefix(self.root.as_path())
                .map_err(|_| ArtifactError::PathLayoutMismatch)?,
        )
        .await?;
        create_private_directory(&paths.source_directory)
            .await
            .map_err(|source| ArtifactError::CreateSubmission {
                path: paths.source_directory.clone(),
                source,
            })?;
        create_private_directory(&paths.attempts)
            .await
            .map_err(|source| ArtifactError::CreateSubmission {
                path: paths.attempts.clone(),
                source,
            })?;
        sync_directory(paths.source_directory.clone())
            .await
            .map_err(|source| ArtifactError::CreateSubmission {
                path: paths.source_directory.clone(),
                source,
            })?;
        sync_directory(paths.attempts.clone())
            .await
            .map_err(|source| ArtifactError::CreateSubmission {
                path: paths.attempts.clone(),
                source,
            })?;
        sync_directory(paths.job.clone()).await.map_err(|source| {
            ArtifactError::CreateSubmission {
                path: paths.job.clone(),
                source,
            }
        })?;
        sync_directory(self.root.join(JOBS_DIRECTORY))
            .await
            .map_err(|source| ArtifactError::CreateSubmission {
                path: paths.job.clone(),
                source,
            })
    }

    async fn ensure_attempt_for_publication(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<AttemptPaths, ArtifactError> {
        let _source = self
            .open_owned_regular_relative(&source_relative_path(job_id))
            .await?;
        self.require_owned_directory_relative(&attempts_relative_path(job_id))
            .await?;

        let relative = attempt_relative_path(job_id, attempt_id);
        if self.optional_owned_directory_relative(&relative).await? {
            Ok(self.attempt_paths(job_id, attempt_id))
        } else {
            self.create_attempt_directory(job_id, attempt_id).await
        }
    }

    async fn owned_attempt_exists(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<bool, ArtifactError> {
        self.require_owned_directory_relative(Path::new(JOBS_DIRECTORY))
            .await?;
        for relative in [
            job_relative_path(job_id),
            attempts_relative_path(job_id),
            attempt_relative_path(job_id, attempt_id),
        ] {
            if !self.optional_owned_directory_relative(&relative).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    async fn optional_owned_directory_relative(
        &self,
        relative: &Path,
    ) -> Result<bool, ArtifactError> {
        let path = self.resolve_relative(relative)?;
        let Some(metadata) = optional_metadata(&path).await? else {
            return Ok(false);
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(ArtifactError::UnsafeOwnedPath(path));
        }
        self.resolve_existing_relative(relative).await?;
        Ok(true)
    }

    async fn require_owned_attempt(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<(), ArtifactError> {
        self.require_owned_directory_relative(&attempt_relative_path(job_id, attempt_id))
            .await
    }

    async fn require_owned_directory_relative(&self, relative: &Path) -> Result<(), ArtifactError> {
        let path = self.resolve_existing_relative(relative).await?;
        require_directory(&path).await
    }

    async fn open_owned_regular_relative(
        &self,
        relative: &Path,
    ) -> Result<(PathBuf, fs::File, std::fs::Metadata), ArtifactError> {
        let path = self.resolve_existing_relative(relative).await?;
        let file =
            open_read_only_no_follow(&path)
                .await
                .map_err(|source| ArtifactError::InspectPath {
                    path: path.clone(),
                    source,
                })?;
        let metadata = file
            .metadata()
            .await
            .map_err(|source| ArtifactError::InspectPath {
                path: path.clone(),
                source,
            })?;
        if !metadata.is_file() {
            return Err(ArtifactError::NotRegularFile(path));
        }
        Ok((path, file, metadata))
    }

    async fn open_validated_relative_file_with_limit(
        &self,
        relative: &Path,
        maximum_byte_length: Option<u64>,
        expected_byte_length: Option<u64>,
        expected_sha256: Option<&str>,
    ) -> Result<ValidatedOpenFile, ArtifactError> {
        let (path, mut file, metadata) = self.open_owned_regular_relative(relative).await?;
        if let Some(maximum) = maximum_byte_length {
            if metadata.len() > maximum {
                return Err(ArtifactError::ByteLengthLimitExceeded {
                    path,
                    maximum,
                    actual: metadata.len(),
                });
            }
        }
        if let Some(expected) = expected_byte_length {
            if metadata.len() != expected {
                return Err(ArtifactError::ByteLengthMismatch {
                    path,
                    expected,
                    actual: metadata.len(),
                });
            }
        }
        let sha256 = hash_open_file(&path, &mut file, maximum_byte_length).await?;
        let final_metadata =
            file.metadata()
                .await
                .map_err(|source| ArtifactError::InspectPath {
                    path: path.clone(),
                    source,
                })?;
        if final_metadata.len() != metadata.len() {
            return Err(ArtifactError::FileChangedDuringValidation(path));
        }
        if let Some(expected) = expected_sha256 {
            if sha256 != expected {
                return Err(ArtifactError::HashMismatch {
                    path,
                    expected: expected.to_owned(),
                    actual: sha256,
                });
            }
        }
        file.seek(std::io::SeekFrom::Start(0))
            .await
            .map_err(|source| ArtifactError::InspectPath { path, source })?;
        Ok(ValidatedOpenFile {
            file,
            byte_length: metadata.len(),
            sha256,
        })
    }
}

fn job_relative_path(job_id: Uuid) -> PathBuf {
    PathBuf::from(JOBS_DIRECTORY).join(job_id.to_string())
}

fn source_staging_relative_path(job_id: Uuid) -> PathBuf {
    job_relative_path(job_id)
        .join(SOURCE_DIRECTORY)
        .join(SOURCE_STAGING_FILE)
}

pub fn source_relative_path(job_id: Uuid) -> PathBuf {
    job_relative_path(job_id)
        .join(SOURCE_DIRECTORY)
        .join(SOURCE_FILE)
}

fn attempt_relative_path(job_id: Uuid, attempt_id: Uuid) -> PathBuf {
    attempts_relative_path(job_id).join(attempt_id.to_string())
}

fn attempts_relative_path(job_id: Uuid) -> PathBuf {
    job_relative_path(job_id).join(ATTEMPTS_DIRECTORY)
}

fn publication_staging_relative_path(job_id: Uuid, attempt_id: Uuid) -> PathBuf {
    attempt_relative_path(job_id, attempt_id).join(PUBLICATION_STAGING_DIRECTORY)
}

fn artifacts_relative_path(job_id: Uuid, attempt_id: Uuid) -> PathBuf {
    attempt_relative_path(job_id, attempt_id).join(ARTIFACTS_DIRECTORY)
}

pub fn artifact_relative_path(
    job_id: Uuid,
    attempt_id: Uuid,
    artifact: PublishedArtifact,
) -> PathBuf {
    artifacts_relative_path(job_id, attempt_id).join(artifact.file_name())
}

fn preacceptance_quarantine_reservation_relative_path(
    job_id: Uuid,
    quarantine_id: Uuid,
) -> PathBuf {
    PathBuf::from(QUARANTINE_DIRECTORY)
        .join(PREACCEPTANCE_DIRECTORY)
        .join(format!("{job_id}-{quarantine_id}"))
}

fn preacceptance_quarantine_relative_path(job_id: Uuid, quarantine_id: Uuid) -> PathBuf {
    preacceptance_quarantine_reservation_relative_path(job_id, quarantine_id).join("job")
}

async fn validate_publication_contents(path: &Path) -> Result<(), ArtifactError> {
    let mut entries =
        fs::read_dir(path)
            .await
            .map_err(|source| ArtifactError::ValidatePublication {
                path: path.to_owned(),
                source,
            })?;
    let mut markdown = false;
    let mut manifest = false;
    let mut count = 0_usize;
    while let Some(entry) =
        entries
            .next_entry()
            .await
            .map_err(|source| ArtifactError::ValidatePublication {
                path: path.to_owned(),
                source,
            })?
    {
        count += 1;
        let file_type =
            entry
                .file_type()
                .await
                .map_err(|source| ArtifactError::ValidatePublication {
                    path: entry.path(),
                    source,
                })?;
        if !file_type.is_file() || file_type.is_symlink() {
            return Err(ArtifactError::UnexpectedPublicationContents);
        }
        if entry.file_name() == OsStr::new(MARKDOWN_FILE) {
            markdown = true;
        } else if entry.file_name() == OsStr::new(MANIFEST_FILE) {
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

#[derive(Clone, Debug, Eq, PartialEq)]
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

    pub fn manifest(&self) -> PathBuf {
        self.published.join(MANIFEST_FILE)
    }
}

/// Removes a readiness probe file on drop. `Drop` cannot await, so the removal
/// is synchronous, which is exactly what makes it survive cancellation.
struct ProbeFile {
    path: PathBuf,
    removed: bool,
}

impl Drop for ProbeFile {
    fn drop(&mut self) {
        if !self.removed {
            // The file may never have been created. Nothing here can be
            // reported, and a stale probe is the only thing worth preventing.
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Writes bytes as well as creating the file, because a full disk accepts the
/// create and fails the write.
async fn write_health_probe(path: &Path) -> std::io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await?;
    file.write_all(b"ready").await?;
    file.flush().await
}

async fn create_private_directory(path: &Path) -> std::io::Result<()> {
    fs::create_dir(path).await?;
    set_private_directory(path)
}

fn ensure_private_directory(path: &Path) -> Result<(), ArtifactError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(ArtifactError::UnsafeOwnedPath(path.to_owned()));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(path).map_err(initialize_error(path))?;
        }
        Err(source) => return Err(initialize_error(path)(source)),
    }
    set_private_directory(path).map_err(initialize_error(path))
}

fn initialize_error(path: &Path) -> impl FnOnce(std::io::Error) -> ArtifactError + '_ {
    move |source| ArtifactError::Initialize {
        path: path.to_owned(),
        source,
    }
}

async fn optional_metadata(path: &Path) -> Result<Option<std::fs::Metadata>, ArtifactError> {
    match fs::symlink_metadata(path).await {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ArtifactError::InspectPath {
            path: path.to_owned(),
            source,
        }),
    }
}

async fn require_absent(path: &Path) -> Result<(), ArtifactError> {
    if optional_metadata(path).await?.is_some() {
        Err(ArtifactError::PathAlreadyExists(path.to_owned()))
    } else {
        Ok(())
    }
}

async fn require_directory(path: &Path) -> Result<(), ArtifactError> {
    let metadata =
        fs::symlink_metadata(path)
            .await
            .map_err(|source| ArtifactError::InspectPath {
                path: path.to_owned(),
                source,
            })?;
    if metadata.file_type().is_symlink() {
        Err(ArtifactError::SymlinkPath(path.to_owned()))
    } else if !metadata.is_dir() {
        Err(ArtifactError::NotDirectory(path.to_owned()))
    } else {
        Ok(())
    }
}

async fn sync_regular_file(path: &Path) -> std::io::Result<()> {
    let file = open_read_only_no_follow(path).await?;
    let metadata = file.metadata().await?;
    if !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "artifact is not a regular file",
        ));
    }
    file.sync_all().await
}

async fn open_read_only_no_follow(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    options.open(path).await
}

async fn sync_directory(path: PathBuf) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || std::fs::File::open(path)?.sync_all())
        .await
        .map_err(std::io::Error::other)?
}

async fn hash_open_file(
    path: &Path,
    file: &mut fs::File,
    maximum_byte_length: Option<u64>,
) -> Result<String, ArtifactError> {
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let read_capacity = maximum_byte_length.map_or(buffer.len(), |maximum| {
            maximum
                .saturating_sub(total)
                .saturating_add(1)
                .min(buffer.len() as u64) as usize
        });
        let count = file
            .read(&mut buffer[..read_capacity])
            .await
            .map_err(|source| ArtifactError::InspectPath {
                path: path.to_owned(),
                source,
            })?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if let Some(maximum) = maximum_byte_length {
            if total > maximum {
                return Err(ArtifactError::ByteLengthLimitExceeded {
                    path: path.to_owned(),
                    maximum,
                    actual: total,
                });
            }
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn set_private_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[derive(Debug, Error)]
pub enum ArtifactError {
    #[error("cannot initialize artifact storage at {path:?}")]
    Initialize {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("artifact data root is not a private regular directory: {0:?}")]
    InvalidDataRoot(PathBuf),
    #[error("cannot create submission storage at {path:?}")]
    CreateSubmission {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot publish immutable source at {path:?}")]
    PublishSource {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "immutable source was renamed into place but its parent directory could not be synced at {path:?}"
    )]
    SourceCommittedButNotSynced {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot create attempt storage at {path:?}")]
    CreateAttempt {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot prepare artifact publication at {path:?}")]
    PreparePublication {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot atomically publish artifacts at {path:?}")]
    Publish {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "artifacts were renamed into place but their attempt directory could not be synced at {path:?}"
    )]
    PublicationCommittedButNotSynced {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot validate staged artifact publication at {path:?}")]
    ValidatePublication {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("staged artifact publication contains unexpected entries")]
    UnexpectedPublicationContents,
    #[error("invalid artifact-relative path: {0:?}")]
    InvalidRelativePath(PathBuf),
    #[error("artifact path does not match the backend-owned deterministic layout")]
    PathLayoutMismatch,
    #[error("artifact path already exists: {0:?}")]
    PathAlreadyExists(PathBuf),
    #[error("cannot inspect artifact path {path:?}")]
    InspectPath {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("artifact path contains a symlink: {0:?}")]
    SymlinkPath(PathBuf),
    #[error("artifact path is not a directory: {0:?}")]
    NotDirectory(PathBuf),
    #[error("artifact path is not a regular file: {0:?}")]
    NotRegularFile(PathBuf),
    #[error("backend-owned path is unsafe to mutate: {0:?}")]
    UnsafeOwnedPath(PathBuf),
    #[error("artifact byte length mismatch at {path:?}: expected {expected}, got {actual}")]
    ByteLengthMismatch {
        path: PathBuf,
        expected: u64,
        actual: u64,
    },
    #[error("artifact exceeds the byte limit at {path:?}: maximum {maximum}, got {actual}")]
    ByteLengthLimitExceeded {
        path: PathBuf,
        maximum: u64,
        actual: u64,
    },
    #[error("artifact SHA-256 mismatch at {path:?}: expected {expected}, got {actual}")]
    HashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("artifact changed while it was being validated: {0:?}")]
    FileChangedDuringValidation(PathBuf),
    #[error("cannot remove backend-owned storage at {path:?}")]
    Discard {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot quarantine preacceptance storage at {path:?}")]
    Quarantine {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot reserve a collision-free preacceptance quarantine for job {0}")]
    QuarantineReservationExhausted(Uuid),
    #[error("cannot write a readiness probe at {path:?}")]
    HealthProbe {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};
    use tempfile::tempdir;
    use tokio::io::AsyncReadExt;
    use uuid::Uuid;

    use super::{
        artifact_relative_path, artifacts_relative_path, attempt_relative_path, job_relative_path,
        preacceptance_quarantine_relative_path, preacceptance_quarantine_reservation_relative_path,
        publication_staging_relative_path, source_relative_path, source_staging_relative_path,
        ArtifactError, ArtifactStore, ProbeFile, PublicationState, PublishedArtifact,
        HEALTH_DIRECTORY, MANIFEST_FILE,
    };
    use crate::worker_protocol::MARKDOWN_FILE;

    #[tokio::test]
    async fn durable_layout_publishes_and_validates_source_and_artifacts() {
        let directory = tempdir().unwrap();
        let store = ArtifactStore::initialize(directory.path()).unwrap();
        // `initialize` canonicalizes, and a macOS temp dir resolves through
        // /private, so a bare temp path would not match what the store built.
        let root = directory.path().canonicalize().unwrap();
        let job_id = Uuid::new_v4();
        let attempt_id = Uuid::new_v4();

        let prepared = store.prepare_submission(job_id).await.unwrap();
        assert_eq!(
            prepared.paths.source_staging,
            root.join(source_staging_relative_path(job_id))
        );
        assert_eq!(
            prepared.paths.source,
            root.join(source_relative_path(job_id))
        );
        assert!(matches!(
            store.create_retry_attempt(job_id, attempt_id).await,
            Err(ArtifactError::InspectPath { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound
        ));
        tokio::fs::write(&prepared.paths.source_staging, b"source-bytes")
            .await
            .unwrap();
        let source = store.publish_source(&prepared).await.unwrap();
        assert_eq!(source.byte_length, 12);
        assert_eq!(source.sha256, hex::encode(Sha256::digest(b"source-bytes")));
        let mut opened_source = store
            .open_validated_source(job_id, Some(12), Some(&source.sha256))
            .await
            .unwrap();
        let mut opened_bytes = Vec::new();
        opened_source
            .file
            .read_to_end(&mut opened_bytes)
            .await
            .unwrap();
        assert_eq!(opened_bytes, b"source-bytes");

        let attempt = store
            .create_retry_attempt(job_id, attempt_id)
            .await
            .unwrap();
        assert_eq!(
            attempt.attempt,
            root.join(attempt_relative_path(job_id, attempt_id))
        );
        let prepared_attempt = store.prepare_artifacts(job_id, attempt_id).await.unwrap();
        assert_eq!(attempt, prepared_attempt);
        tokio::fs::write(prepared_attempt.staged_markdown(), b"# result\n")
            .await
            .unwrap();
        tokio::fs::write(prepared_attempt.staged_manifest(), b"{}")
            .await
            .unwrap();
        store.publish_artifacts(job_id, attempt_id).await.unwrap();

        let markdown_hash = hex::encode(Sha256::digest(b"# result\n"));
        let mut markdown = store
            .open_bounded_validated_artifact(
                job_id,
                attempt_id,
                PublishedArtifact::Markdown,
                9,
                Some(9),
                Some(&markdown_hash),
            )
            .await
            .unwrap();
        assert_eq!(markdown.byte_length, 9);
        assert_eq!(markdown.sha256, markdown_hash);
        let mut opened_markdown_bytes = Vec::new();
        markdown
            .file
            .read_to_end(&mut opened_markdown_bytes)
            .await
            .unwrap();
        assert_eq!(opened_markdown_bytes, b"# result\n");
        assert!(store
            .resolve_existing_relative(&artifact_relative_path(
                job_id,
                attempt_id,
                PublishedArtifact::Manifest,
            ))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn bounded_artifact_open_rejects_oversize_metadata_before_hash_validation() {
        let directory = tempdir().unwrap();
        let store = ArtifactStore::initialize(directory.path()).unwrap();
        let job_id = Uuid::new_v4();
        let attempt_id = Uuid::new_v4();
        let prepared = store.prepare_submission(job_id).await.unwrap();
        tokio::fs::write(&prepared.paths.source_staging, b"source")
            .await
            .unwrap();
        store.publish_source(&prepared).await.unwrap();
        store
            .create_retry_attempt(job_id, attempt_id)
            .await
            .unwrap();
        let attempt = store.prepare_artifacts(job_id, attempt_id).await.unwrap();
        tokio::fs::write(attempt.staged_markdown(), b"123456789")
            .await
            .unwrap();
        tokio::fs::write(attempt.staged_manifest(), b"{}")
            .await
            .unwrap();
        store.publish_artifacts(job_id, attempt_id).await.unwrap();

        let result = store
            .open_bounded_validated_artifact(
                job_id,
                attempt_id,
                PublishedArtifact::Markdown,
                8,
                None,
                Some(&"0".repeat(64)),
            )
            .await;
        assert!(matches!(
            result,
            Err(ArtifactError::ByteLengthLimitExceeded {
                maximum: 8,
                actual: 9,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn retries_are_isolated_and_cleanup_is_id_derived() {
        let directory = tempdir().unwrap();
        let store = ArtifactStore::initialize(directory.path()).unwrap();
        let job_id = Uuid::new_v4();
        let first_attempt = Uuid::new_v4();
        let second_attempt = Uuid::new_v4();
        let prepared = store.prepare_submission(job_id).await.unwrap();
        tokio::fs::write(&prepared.paths.source_staging, b"source")
            .await
            .unwrap();
        store.publish_source(&prepared).await.unwrap();

        store
            .create_retry_attempt(job_id, first_attempt)
            .await
            .unwrap();
        store
            .create_retry_attempt(job_id, second_attempt)
            .await
            .unwrap();
        store.discard_attempt(job_id, first_attempt).await.unwrap();
        assert!(!directory
            .path()
            .join(attempt_relative_path(job_id, first_attempt))
            .exists());
        assert!(directory
            .path()
            .join(attempt_relative_path(job_id, second_attempt))
            .exists());
        assert!(prepared.paths.source.exists());

        store.discard_unaccepted_job(job_id).await.unwrap();
        assert!(!directory.path().join(job_relative_path(job_id)).exists());
    }

    #[tokio::test]
    async fn quarantine_moves_only_the_named_preacceptance_job() {
        let directory = tempdir().unwrap();
        let store = ArtifactStore::initialize(directory.path()).unwrap();
        let job_id = Uuid::new_v4();
        let other_job_id = Uuid::new_v4();
        store.prepare_submission(job_id).await.unwrap();
        store.prepare_submission(other_job_id).await.unwrap();

        let collision_id = Uuid::new_v4();
        let collision = directory
            .path()
            .join(preacceptance_quarantine_reservation_relative_path(
                job_id,
                collision_id,
            ));
        std::fs::create_dir(&collision).unwrap();
        assert!(matches!(
            store
                .quarantine_preacceptance_as(job_id, collision_id)
                .await,
            Err(ArtifactError::PathAlreadyExists(_))
        ));
        assert!(directory.path().join(job_relative_path(job_id)).exists());

        let quarantine = store.quarantine_preacceptance(job_id).await.unwrap();
        assert!(quarantine.starts_with(std::path::Path::new("quarantine/pre-acceptance")));
        assert!(directory.path().join(&quarantine).exists());
        assert_eq!(quarantine.file_name().unwrap(), "job");
        assert!(quarantine
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(&job_id.to_string()));
        assert!(!directory.path().join(job_relative_path(job_id)).exists());
        assert!(directory
            .path()
            .join(job_relative_path(other_job_id))
            .exists());

        let deterministic_example = preacceptance_quarantine_relative_path(job_id, collision_id);
        assert_eq!(deterministic_example.file_name().unwrap(), "job");
    }

    #[tokio::test]
    async fn validation_rejects_mismatches_traversal_and_forged_deletion_paths() {
        let directory = tempdir().unwrap();
        let store = ArtifactStore::initialize(directory.path()).unwrap();
        let job_id = Uuid::new_v4();
        let attempt_id = Uuid::new_v4();
        let prepared = store.prepare_submission(job_id).await.unwrap();
        tokio::fs::write(&prepared.paths.source_staging, b"source")
            .await
            .unwrap();
        store.publish_source(&prepared).await.unwrap();

        assert!(matches!(
            store.validate_source(job_id, Some(99), None).await,
            Err(ArtifactError::ByteLengthMismatch { .. })
        ));
        assert!(matches!(
            store
                .validate_source(job_id, None, Some(&"0".repeat(64)))
                .await,
            Err(ArtifactError::HashMismatch { .. })
        ));
        assert!(matches!(
            store.resolve_relative(std::path::Path::new("../outside")),
            Err(ArtifactError::InvalidRelativePath(_))
        ));
        assert!(matches!(
            store.resolve_relative(directory.path()),
            Err(ArtifactError::InvalidRelativePath(_))
        ));

        let occupied_job_id = Uuid::new_v4();
        let occupied = store.prepare_submission(occupied_job_id).await.unwrap();
        tokio::fs::write(&occupied.paths.source_staging, b"new")
            .await
            .unwrap();
        tokio::fs::write(&occupied.paths.source, b"existing")
            .await
            .unwrap();
        assert!(matches!(
            store.publish_source(&occupied).await,
            Err(ArtifactError::PathAlreadyExists(_))
        ));

        let attempt = store
            .create_retry_attempt(job_id, attempt_id)
            .await
            .unwrap();
        store.prepare_artifacts(job_id, attempt_id).await.unwrap();
        tokio::fs::write(attempt.staged_markdown(), b"markdown")
            .await
            .unwrap();
        tokio::fs::write(attempt.staged_manifest(), b"{}")
            .await
            .unwrap();
        tokio::fs::create_dir(&attempt.published).await.unwrap();
        assert!(matches!(
            store.publish_artifacts(job_id, attempt_id).await,
            Err(ArtifactError::PathAlreadyExists(_))
        ));

        assert_eq!(
            publication_staging_relative_path(job_id, attempt_id),
            attempt_relative_path(job_id, attempt_id).join("publication.staging")
        );
        assert_eq!(
            artifacts_relative_path(job_id, attempt_id),
            attempt_relative_path(job_id, attempt_id).join("artifacts")
        );
        assert_eq!(MANIFEST_FILE, "manifest.json");
        assert_eq!(MARKDOWN_FILE, "result.md");
    }

    #[tokio::test]
    async fn preparation_creates_missing_retry_attempt_and_preserves_existing_attempt() {
        let directory = tempdir().unwrap();
        let store = ArtifactStore::initialize(directory.path()).unwrap();
        let job_id = Uuid::new_v4();
        let initial_attempt_id = Uuid::new_v4();
        let retry_attempt_id = Uuid::new_v4();
        let prepared = store.prepare_submission(job_id).await.unwrap();
        tokio::fs::write(&prepared.paths.source_staging, b"source")
            .await
            .unwrap();
        store.publish_source(&prepared).await.unwrap();

        let initial = store
            .create_retry_attempt(job_id, initial_attempt_id)
            .await
            .unwrap();
        let marker = initial.attempt.join("preserve-me");
        tokio::fs::write(&marker, b"marker").await.unwrap();
        let prepared_initial = store
            .prepare_artifacts(job_id, initial_attempt_id)
            .await
            .unwrap();
        assert_eq!(prepared_initial, initial);
        assert!(marker.exists());

        let retry = store
            .prepare_artifacts(job_id, retry_attempt_id)
            .await
            .unwrap();
        assert!(retry.attempt.is_dir());
        assert!(retry.publication_staging.is_dir());
        assert!(prepared.paths.source.is_file());
    }

    #[tokio::test]
    async fn publication_inspection_and_staging_cleanup_preserve_published_data() {
        let directory = tempdir().unwrap();
        let store = ArtifactStore::initialize(directory.path()).unwrap();
        let job_id = Uuid::new_v4();
        let attempt_id = Uuid::new_v4();
        let prepared = store.prepare_submission(job_id).await.unwrap();
        tokio::fs::write(&prepared.paths.source_staging, b"source")
            .await
            .unwrap();
        store.publish_source(&prepared).await.unwrap();
        store
            .create_retry_attempt(job_id, attempt_id)
            .await
            .unwrap();

        assert_eq!(
            store.inspect_publication(job_id, attempt_id).await.unwrap(),
            PublicationState::Missing
        );
        let attempt = store.prepare_artifacts(job_id, attempt_id).await.unwrap();
        assert_eq!(
            store.inspect_publication(job_id, attempt_id).await.unwrap(),
            PublicationState::StagingOnly
        );
        tokio::fs::write(attempt.staged_markdown(), b"markdown")
            .await
            .unwrap();
        tokio::fs::write(attempt.staged_manifest(), b"{}")
            .await
            .unwrap();
        store.publish_artifacts(job_id, attempt_id).await.unwrap();
        assert_eq!(
            store.inspect_publication(job_id, attempt_id).await.unwrap(),
            PublicationState::Published
        );

        tokio::fs::create_dir(&attempt.publication_staging)
            .await
            .unwrap();
        tokio::fs::write(attempt.staged_markdown(), b"stale")
            .await
            .unwrap();
        assert_eq!(
            store.inspect_publication(job_id, attempt_id).await.unwrap(),
            PublicationState::Conflicting
        );
        store
            .discard_publication_staging(job_id, attempt_id)
            .await
            .unwrap();
        assert_eq!(
            store.inspect_publication(job_id, attempt_id).await.unwrap(),
            PublicationState::Published
        );
        assert_eq!(
            tokio::fs::read(attempt.published.join(MARKDOWN_FILE))
                .await
                .unwrap(),
            b"markdown"
        );
        assert!(prepared.paths.source.is_file());
    }

    #[tokio::test]
    async fn owned_job_listing_includes_only_canonical_uuid_directories() {
        let directory = tempdir().unwrap();
        let store = ArtifactStore::initialize(directory.path()).unwrap();
        let first = Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef").unwrap();
        let second = Uuid::parse_str("11234567-89ab-cdef-0123-456789abcdef").unwrap();
        store.prepare_submission(second).await.unwrap();
        store.prepare_submission(first).await.unwrap();

        let jobs = directory.path().join("jobs");
        tokio::fs::create_dir(jobs.join(format!("{{{first}}}")))
            .await
            .unwrap();
        tokio::fs::create_dir(jobs.join("not-a-job")).await.unwrap();
        tokio::fs::write(jobs.join(Uuid::new_v4().to_string()), b"not a directory")
            .await
            .unwrap();

        assert_eq!(
            store.list_owned_job_ids().await.unwrap(),
            vec![first, second]
        );
    }

    #[tokio::test]
    async fn readiness_probe_leaves_job_storage_untouched() {
        let directory = tempdir().unwrap();
        let store = ArtifactStore::initialize(directory.path()).unwrap();
        let job_id = Uuid::new_v4();
        store.prepare_submission(job_id).await.unwrap();

        store.probe_writable().await.unwrap();
        store.probe_writable().await.unwrap();

        // The probe directory sits beside `jobs/`, so the startup orphan scan
        // that quarantines unowned storage never sees it.
        assert_eq!(store.list_owned_job_ids().await.unwrap(), vec![job_id]);
        assert!(store.job_paths(job_id).source_directory.is_dir());

        let health = directory.path().join(HEALTH_DIRECTORY);
        let mut left_behind = tokio::fs::read_dir(&health).await.unwrap();
        assert!(left_behind.next_entry().await.unwrap().is_none());

        tokio::fs::remove_dir(&health).await.unwrap();
        store.probe_writable().await.unwrap();
        assert!(health.is_dir());
    }

    #[test]
    fn an_unfinished_readiness_probe_removes_its_file_on_drop() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("probe");

        {
            let guard = ProbeFile {
                path: path.clone(),
                removed: false,
            };
            std::fs::write(&guard.path, b"ready").unwrap();
            assert!(path.is_file());
        }

        // Covers both the failed write and the cancelled check: neither reaches
        // `mark_removed`, so a degraded data root cannot grow probe files.
        assert!(!path.exists(), "a dropped probe file survived");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn validation_rejects_symlink_substitution_and_directories_are_private() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let directory = tempdir().unwrap();
        let store = ArtifactStore::initialize(directory.path()).unwrap();
        let job_id = Uuid::new_v4();
        let attempt_id = Uuid::new_v4();
        let prepared = store.prepare_submission(job_id).await.unwrap();
        let mode = std::fs::metadata(&prepared.paths.job)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);

        let outside = directory.path().join("outside");
        tokio::fs::write(&outside, b"outside").await.unwrap();
        symlink(&outside, &prepared.paths.source).unwrap();
        assert!(super::open_read_only_no_follow(&prepared.paths.source)
            .await
            .is_err());
        assert!(matches!(
            store.validate_source(job_id, None, None).await,
            Err(ArtifactError::SymlinkPath(_))
        ));

        let ancestor_job_id = Uuid::new_v4();
        let ancestor = store.prepare_submission(ancestor_job_id).await.unwrap();
        tokio::fs::remove_dir(&ancestor.paths.source_directory)
            .await
            .unwrap();
        let outside_directory = directory.path().join("outside-directory");
        tokio::fs::create_dir(&outside_directory).await.unwrap();
        tokio::fs::write(outside_directory.join("input"), b"outside")
            .await
            .unwrap();
        symlink(&outside_directory, &ancestor.paths.source_directory).unwrap();
        assert!(matches!(
            store.validate_source(ancestor_job_id, None, None).await,
            Err(ArtifactError::SymlinkPath(_))
        ));

        let attempt = store
            .create_attempt_directory(job_id, attempt_id)
            .await
            .unwrap();
        let attempt_mode = std::fs::metadata(&attempt.attempt)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(attempt_mode, 0o700);

        let symlink_job_id = Uuid::new_v4();
        symlink(
            &outside_directory,
            directory.path().join(job_relative_path(symlink_job_id)),
        )
        .unwrap();
        assert!(!store
            .list_owned_job_ids()
            .await
            .unwrap()
            .contains(&symlink_job_id));

        tokio::fs::remove_dir(&attempt.publication_staging)
            .await
            .ok();
        let outside_publication = directory.path().join("outside-publication");
        tokio::fs::create_dir(&outside_publication).await.unwrap();
        symlink(&outside_publication, &attempt.publication_staging).unwrap();
        assert!(matches!(
            store.inspect_publication(job_id, attempt_id).await,
            Err(ArtifactError::UnsafeOwnedPath(_))
        ));
    }
}
