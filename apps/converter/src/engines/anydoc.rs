//! AnyDoc engine: converts proven non-PDF documents in `tool-kit-pdf-worker
//! --anydoc`, the child-worker shape of `pdf_inspector.rs`.
//!
//! In-process parsing (2026-08-18) was measured holding memory: a 1.3 MB DOCX
//! took the converter from 4.5 to 458 MiB, and macOS kept 110 MiB of the freed
//! pages after the job. A child gives all of it back on exit, and a hang or a
//! panic is killed with it. PDFs are refused outright: the isolated PDF worker
//! is the only PDF path, and AnyDoc's embedded pdf-inspector must never
//! bypass it.

use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use tokio::{fs, io::AsyncWriteExt, process::Command, sync::watch};
use tool_kit_worker_protocol::anydoc::{
    AnyDocRejection, AnyDocReport, ANYDOC_MARKDOWN_FILE, ANYDOC_MARKED_FILE,
    ANYDOC_PICTURES_DIRECTORY, ANYDOC_REPORT_FILE,
};

use super::child::{self, wait_for_child};
use super::{EngineAnalysis, EngineFailure, EngineOutcome, EngineRejection, QualitySignals};
use crate::{
    artifacts::{AttemptPaths, ValidatedOpenFile},
    persistence::DocumentClassification,
    worker_protocol::{WORKER_EXPECTED_SOURCE_BYTES_ENV, WORKER_EXPECTED_SOURCE_SHA256_ENV},
};

pub(crate) const ANYDOC_ENGINE_NAME: &str = "anydoc";
pub(crate) const ANYDOC_VERSION: &str = "0.2.4";

/// Engine-specific detail persisted as attempt diagnostics. Content-free: the
/// detected format family and wall time only.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnyDocDiagnostics {
    pub format: String,
    pub processing_time_ms: u64,
}
#[derive(Clone, Debug)]
pub struct AnyDocEngine {
    worker: PathBuf,
    max_output_bytes: u64,
    timeout: Duration,
    /// The Vision worker, which also describes DOCX and PPTX pictures.
    describer: Option<PathBuf>,
}

impl AnyDocEngine {
    pub fn new(
        worker: PathBuf,
        max_output_bytes: u64,
        timeout: Duration,
        describer: Option<PathBuf>,
    ) -> Self {
        Self {
            worker,
            max_output_bytes,
            timeout,
            describer,
        }
    }

    pub async fn convert(
        &self,
        paths: &AttemptPaths,
        source: ValidatedOpenFile,
        cancellation: watch::Receiver<bool>,
        admitted_label: &str,
    ) -> Result<EngineOutcome, EngineFailure> {
        if *cancellation.borrow() {
            return Err(EngineFailure::Interrupted);
        }
        let ValidatedOpenFile {
            file,
            byte_length,
            sha256,
        } = source;
        if byte_length == 0 {
            return Err(EngineFailure::Protocol);
        }

        // Private, not the staging directory: only the Markdown chosen below
        // is published. The worker re-verifies the source before parsing.
        let output = tempfile::tempdir().map_err(|_| EngineFailure::Protocol)?;
        let mut command = Command::new(&self.worker);
        command.arg("--anydoc").arg(admitted_label);
        if self.describer.is_some() {
            command.arg("--mark");
        }
        command
            .arg(output.path())
            .current_dir(&paths.attempt)
            .env_clear()
            .env(WORKER_EXPECTED_SOURCE_BYTES_ENV, byte_length.to_string())
            .env(WORKER_EXPECTED_SOURCE_SHA256_ENV, sha256)
            .stdin(Stdio::from(file.into_std().await))
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|_| EngineFailure::Unavailable)?;
        wait_for_child(&mut child, "anydoc", self.timeout, cancellation.clone()).await?;
        let (processing_time_ms, marked) =
            match child::read_report(&output.path().join(ANYDOC_REPORT_FILE)).await? {
                AnyDocReport::Converted {
                    processing_time_ms,
                    marked,
                } => (processing_time_ms, marked),
                AnyDocReport::Rejected { code } => return Ok(rejected(code)),
            };

        // Pictures with no alt text get a description where the model is
        // available. A failed run leaves today's output untouched. Once one
        // picture is described, a picture the model skipped also loses
        // Office's own "Description automatically generated" guess.
        let mut filled = None;
        if let (Some(worker), Some(marked)) = (self.describer.as_deref(), marked) {
            let pictures = output.path().join(ANYDOC_PICTURES_DIRECTORY);
            let found = describe(worker, &pictures, &marked.placements, cancellation.clone()).await;
            let path = output.path().join(ANYDOC_MARKED_FILE);
            let bounded = fs::symlink_metadata(&path).await.is_ok_and(|metadata| {
                metadata.is_file() && metadata.len() <= self.max_output_bytes
            });
            if !found.is_empty() && bounded {
                filled = fs::read_to_string(&path)
                    .await
                    .ok()
                    .and_then(|markdown| fill_descriptions(&markdown, &found, marked.rendered));
            }
        }
        // The plain Markdown streams to staging like the PDF worker's, so the
        // converter never holds it. The worker writes none when it is blank.
        let plain = output.path().join(ANYDOC_MARKDOWN_FILE);
        let output_bytes = match &filled {
            Some(markdown) if markdown.trim().is_empty() => {
                return Ok(rejected(AnyDocRejection::InvalidDocument));
            }
            Some(markdown) => markdown.len() as u64,
            None => match fs::symlink_metadata(&plain).await {
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    return Ok(rejected(AnyDocRejection::InvalidDocument));
                }
                Ok(metadata) if metadata.is_file() => metadata.len(),
                _ => return Err(EngineFailure::Protocol),
            },
        };
        if output_bytes > self.max_output_bytes {
            return Ok(rejected(AnyDocRejection::DocumentExceedsLimits));
        }
        if *cancellation.borrow() {
            return Err(EngineFailure::Interrupted);
        }

        let markdown_path = paths.staged_markdown();
        let mut staged = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&markdown_path)
            .await
            .map_err(|_| EngineFailure::Protocol)?;
        let written = match filled {
            Some(markdown) => staged
                .write_all(markdown.as_bytes())
                .await
                .map(|()| output_bytes),
            None => match fs::File::open(&plain).await {
                Ok(mut source) => tokio::io::copy(&mut source, &mut staged).await,
                Err(error) => Err(error),
            },
        };
        if written.ok() != Some(output_bytes) {
            return Err(EngineFailure::Protocol);
        }
        staged
            .sync_all()
            .await
            .map_err(|_| EngineFailure::Protocol)?;
        let (digest, _) = child::hash_and_check_content(&markdown_path).await?;

        Ok(EngineOutcome::Converted {
            analysis: EngineAnalysis {
                classification: DocumentClassification::StructuredDocument,
                // AnyDoc reports no completeness measure. Its own part-level
                // "skip a broken piece and continue" recovery is silent, so
                // claiming a measurement here would be inventing one.
                quality: QualitySignals::unmeasured(),
                // The worker refuses any other family, so the admitted label
                // is the detected one.
                diagnostics: serde_json::to_value(AnyDocDiagnostics {
                    format: admitted_label.to_owned(),
                    processing_time_ms,
                })
                .map_err(|_| EngineFailure::Protocol)?,
            },
            byte_length: output_bytes,
            sha256: digest,
        })
    }
}

fn rejected(rejection: AnyDocRejection) -> EngineOutcome {
    EngineOutcome::Rejected {
        rejection: EngineRejection {
            code: rejection.as_str(),
            message: match rejection {
                AnyDocRejection::UnsupportedDocument => "The document format is not supported.",
                AnyDocRejection::EncryptedDocument => "Encrypted documents are not accepted.",
                AnyDocRejection::InvalidDocument => "The document could not be converted.",
                AnyDocRejection::DocumentExceedsLimits => {
                    "The document exceeds the conversion limits."
                }
            },
        },
    }
}

/// Runs the Vision worker once over the worker's picture directory and
/// returns the descriptions it wrote by token. A decorative picture, a
/// per-picture error, or no model leaves a token out, and a failed run returns
/// what it finished.
async fn describe(
    worker: &Path,
    directory: &Path,
    placements: &[(u32, String)],
    cancellation: watch::Receiver<bool>,
) -> HashMap<u32, String> {
    let mut found = HashMap::new();
    let Ok(mut child) = Command::new(worker)
        .arg("--describe")
        .arg(directory)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
    else {
        return found;
    };
    let files = placements
        .iter()
        .map(|(_, name)| name)
        .collect::<HashSet<_>>()
        .len();
    // ponytail: about 5 s per picture after a cold model load, measured on
    // one Mac. A huge deck waits that long, cancellation still stops it.
    let budget = Duration::from_secs(20 + 5 * files as u64);
    let _ = wait_for_child(&mut child, "vision-describe", budget, cancellation).await;
    for (n, name) in placements {
        // The worker parsed hostile input, so a name may not leave the
        // directory.
        if Path::new(name).file_name() != Some(OsStr::new(name)) {
            continue;
        }
        let Ok(text) = fs::read_to_string(directory.join(format!("{name}.txt"))).await else {
            continue;
        };
        let text = text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace(['*', '|', '\\', '`', '_', '~', '[', ']', '<', '>'], "");
        if !text.is_empty() {
            found.insert(*n, text);
        }
    }
    found
}

/// Replaces each `tkimg{n}tk` with `*Image: <description>*`, or with nothing
/// when there is none. A token alone on its line takes its blank line with it.
/// `None` when the Markdown holds more tokens than the pictures put there,
/// because then the document's own text spells one.
fn fill_descriptions(
    markdown: &str,
    found: &HashMap<u32, String>,
    expected: usize,
) -> Option<String> {
    let mut seen = 0;
    let mut out = String::with_capacity(markdown.len());
    let mut rest = markdown;
    while let Some(start) = rest.find("tkimg") {
        let (before, tail) = rest.split_at(start);
        let tail = tail.strip_prefix("tkimg").unwrap_or(tail);
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        let (number, after) = tail.split_at(digits);
        let Some(after) = after.strip_prefix("tk").filter(|_| digits > 0) else {
            out.push_str(before);
            out.push_str("tkimg");
            rest = tail;
            continue;
        };
        out.push_str(before);
        rest = after;
        seen += 1;
        match number.parse().ok().and_then(|n: u32| found.get(&n)) {
            Some(text) => {
                out.push_str("*Image: ");
                out.push_str(text);
                out.push('*');
            }
            None if out.is_empty() || out.ends_with("\n\n") => {
                rest = rest.trim_start_matches('\n');
            }
            None => {}
        }
    }
    out.push_str(rest);
    (seen == expected).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The manifest and capabilities report this constant, so a dependency
    /// bump that leaves it behind would publish the wrong engine version.
    #[test]
    fn the_pinned_version_matches_the_crate() {
        assert!(
            include_str!("../../Cargo.toml").contains(&format!("anydoc = \"={ANYDOC_VERSION}\""))
        );
    }

    #[test]
    fn markers_come_out_filled() {
        let found = HashMap::from([(0, "A grid.".to_owned())]);
        assert_eq!(
            fill_descriptions(
                "Intro\n\ntkimg0tk\n\ntkimg1tk\n\nEnd tkimg7tk.\n",
                &found,
                3
            )
            .as_deref(),
            Some("Intro\n\n*Image: A grid.*\n\nEnd .\n")
        );
        // Body text that spells a token makes one too many: today's output.
        assert_eq!(fill_descriptions("tkimg0tk tkimg0tk\n", &found, 1), None);
    }
}
