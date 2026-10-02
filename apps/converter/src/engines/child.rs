//! Shared child-process supervision for the engine workers.
//!
//! One implementation of the deadline/cancellation semantics the job runner
//! proved in M2, so a second engine cannot drift from them: a cancelled or
//! timed-out worker is always killed and reaped, never left behind.
//!
//! It owns the rest of the plumbing every worker-backed engine repeats too:
//! the startup handshake, the report read, and the staged-Markdown checks.

use std::{
    io::Read as _,
    path::{Path, PathBuf},
    process::ExitStatus,
    process::{Command as StdCommand, Stdio},
    sync::Arc,
    thread,
};

use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{
    fs,
    io::AsyncReadExt,
    process::Child,
    sync::{watch, OwnedSemaphorePermit, Semaphore},
    time::{sleep, Duration},
};

use super::EngineFailure;
use crate::artifacts::AttemptPaths;

/// A worker report is a handful of fields; anything larger is not one.
const MAX_REPORT_BYTES: u64 = 1024 * 1024;

/// Waits for a worker child under a hard deadline and a cancellation watch.
pub(crate) async fn wait_for_child(
    child: &mut Child,
    timeout: Duration,
    mut cancellation: watch::Receiver<bool>,
) -> Result<ExitStatus, EngineFailure> {
    let status = tokio::select! {
        biased;
        () = async {
            let _ = cancellation.wait_for(|cancelled| *cancelled).await;
        } => {
            kill_and_reap(child).await;
            return Err(EngineFailure::Interrupted);
        }
        () = sleep(timeout) => {
            kill_and_reap(child).await;
            return Err(EngineFailure::Timeout);
        }
        result = child.wait() => {
            match result {
                Ok(status) => status,
                Err(_) => {
                    kill_and_reap(child).await;
                    return Err(EngineFailure::Crashed);
                }
            }
        }
    };
    if !status.success() {
        return Err(EngineFailure::Crashed);
    }
    if *cancellation.borrow() {
        return Err(EngineFailure::Interrupted);
    }
    Ok(status)
}

async fn kill_and_reap(child: &mut Child) {
    let _ = child.start_kill();
    let _ = child.wait().await;
}

pub(crate) async fn acquire(
    permits: &Arc<Semaphore>,
) -> Result<OwnedSemaphorePermit, EngineFailure> {
    Arc::clone(permits)
        .acquire_owned()
        .await
        .map_err(|_| EngineFailure::Unavailable)
}

/// Reads one worker report off the staging directory. A symlink, a directory,
/// or a file past the ceiling is a protocol error, never something to parse.
pub(crate) async fn read_report<T: DeserializeOwned>(path: &Path) -> Result<T, EngineFailure> {
    let metadata = fs::symlink_metadata(path)
        .await
        .map_err(|_| EngineFailure::Protocol)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_REPORT_BYTES
    {
        return Err(EngineFailure::Protocol);
    }
    let encoded = fs::read(path).await.map_err(|_| EngineFailure::Protocol)?;
    serde_json::from_slice(&encoded).map_err(|_| EngineFailure::Protocol)
}

/// Checks the staged Markdown against what the report claimed and returns its
/// digest. The report is the worker's word; this is the measurement.
pub(crate) async fn validate_staged_markdown(
    paths: &AttemptPaths,
    expected_name: &str,
    relative_path: &str,
    byte_length: u64,
    sha256: &str,
    max_output_bytes: u64,
) -> Result<String, EngineFailure> {
    if relative_path != expected_name
        || byte_length == 0
        || byte_length > max_output_bytes
        || !is_lowercase_sha256(sha256)
    {
        return Err(EngineFailure::Protocol);
    }
    let markdown_path = paths.staged_markdown();
    let metadata = fs::symlink_metadata(&markdown_path)
        .await
        .map_err(|_| EngineFailure::Protocol)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != byte_length {
        return Err(EngineFailure::Protocol);
    }
    let (digest, has_content) = hash_and_check_content(&markdown_path).await?;
    if !has_content || digest != sha256 {
        return Err(EngineFailure::Protocol);
    }
    Ok(digest)
}

/// A worker that gave up must have staged nothing. Leftover Markdown means the
/// report and the directory disagree, so neither can be trusted.
pub(crate) async fn reject_if_markdown_staged(paths: &AttemptPaths) -> Result<(), EngineFailure> {
    if fs::try_exists(paths.staged_markdown())
        .await
        .map_err(|_| EngineFailure::Protocol)?
    {
        return Err(EngineFailure::Protocol);
    }
    Ok(())
}

pub(crate) fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The shape of an OS or package version in a handshake line: digits and dots.
pub(crate) fn is_dotted_number(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
}

async fn hash_and_check_content(path: &Path) -> Result<(String, bool), EngineFailure> {
    let mut file = fs::File::open(path)
        .await
        .map_err(|_| EngineFailure::Protocol)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut has_content = false;
    loop {
        let count = file
            .read(&mut buffer)
            .await
            .map_err(|_| EngineFailure::Protocol)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        has_content |= buffer[..count]
            .iter()
            .any(|byte| !byte.is_ascii_whitespace());
    }
    Ok((hex::encode(digest.finalize()), has_content))
}

pub(crate) fn validate_worker(path: &Path, worker: &'static str) -> Result<(), WorkerStartupError> {
    let metadata = std::fs::metadata(path).map_err(|source| WorkerStartupError::InvalidWorker {
        worker,
        path: path.to_owned(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(WorkerStartupError::WorkerNotFile {
            worker,
            path: path.to_owned(),
        });
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(WorkerStartupError::WorkerNotExecutable {
                worker,
                path: path.to_owned(),
            });
        }
    }
    Ok(())
}

/// Runs the `--version` handshake and returns the line the worker printed.
/// A non-zero exit is a mismatch here rather than in each caller's tail, so an
/// engine cannot forget to check it. What the line has to *say* is the tail's
/// business: pinned bytes for one engine, a version parse for another.
pub(crate) fn worker_identity_line(
    path: &Path,
    worker: &'static str,
    timeout: Duration,
) -> Result<Vec<u8>, WorkerStartupError> {
    let handshake = |source| WorkerStartupError::WorkerHandshake {
        worker,
        path: path.to_owned(),
        source,
    };
    let mut child = StdCommand::new(path)
        .arg("--version")
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(handshake)?;
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(WorkerStartupError::WorkerIdentityTimeout {
                    worker,
                    path: path.to_owned(),
                });
            }
            Err(source) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(handshake(source));
            }
        }
    };
    let mut output = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        stdout
            .take(4097)
            .read_to_end(&mut output)
            .map_err(handshake)?;
    }
    if !status.success() {
        return Err(WorkerStartupError::WorkerIdentityMismatch {
            worker,
            path: path.to_owned(),
        });
    }
    Ok(output)
}

/// The refusals every worker-backed engine shares. `worker` names the engine
/// in the message, so the two read exactly as they did when each owned a copy.
#[derive(Debug, Error)]
pub enum WorkerStartupError {
    #[error("{worker} worker is unavailable at {path:?}")]
    InvalidWorker {
        worker: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{worker} worker path is not a regular file: {path:?}")]
    WorkerNotFile { worker: &'static str, path: PathBuf },
    #[error("{worker} worker path is not executable: {path:?}")]
    WorkerNotExecutable { worker: &'static str, path: PathBuf },
    #[error("{worker} worker handshake failed at {path:?}")]
    WorkerHandshake {
        worker: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{worker} worker identity does not match the pinned protocol: {path:?}")]
    WorkerIdentityMismatch { worker: &'static str, path: PathBuf },
    #[error("{worker} worker identity check timed out: {path:?}")]
    WorkerIdentityTimeout { worker: &'static str, path: PathBuf },
}
