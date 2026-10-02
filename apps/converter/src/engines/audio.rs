//! Local audio engine: transcribes one recording through the Swift worker.
//!
//! Isolated in a child process on the `vision.rs` shape, and absent the same
//! way: no worker binary means no engine, never a startup error. The one
//! difference is the diarizer models. A staged worker with no models is a
//! packaging bug, so that combination fails startup rather than failing every
//! job at run time, which is how `TOOLKIT_CONVERTER_PDF_BCMAPS_DIR` hid for a
//! release.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use thiserror::Error;
use tokio::{
    fs,
    process::Command,
    sync::{watch, OwnedSemaphorePermit, Semaphore},
};

use super::child::{
    self, is_dotted_number, is_lowercase_sha256, wait_for_child, WorkerStartupError,
};
use super::{EngineAnalysis, EngineFailure, EngineOutcome, EngineRejection, QualitySignals};
use crate::{
    artifacts::{AttemptPaths, ValidatedOpenFile},
    audio_protocol::{
        audio_source_extension, AudioDetail, AudioOutcome, AudioRejectionCode, AudioReport,
        AUDIO_ENGINE_NAME, AUDIO_MARKDOWN_FILE, AUDIO_WORKER_DIARIZER_DIR_ENV,
        AUDIO_WORKER_EXPECTED_SOURCE_BYTES_ENV, AUDIO_WORKER_EXPECTED_SOURCE_SHA256_ENV,
        AUDIO_WORKER_IDENTITY_PREFIX, AUDIO_WORKER_MAX_OUTPUT_BYTES_ENV,
        AUDIO_WORKER_MEDIA_TYPE_ENV, AUDIO_WORKER_PROTOCOL_VERSION, AUDIO_WORKER_REPORT_FILE,
        AUDIO_WORKER_SPEAKER_COUNT_ENV,
    },
    persistence::DocumentClassification,
};

const WORKER_LABEL: &str = "Audio";
const WORKER_IDENTITY_TIMEOUT: Duration = Duration::from_secs(10);
/// The worker runs both stages on every job, so the report says so on a
/// rejection too and the parent checks it either way.
const WORKER_FEATURES: [&str; 2] = ["speech-analyzer", "diarization"];

/// Engine-specific detail persisted as attempt diagnostics and embedded in the
/// manifest. Content-free: durations and speaker counts, never words.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct AudioDiagnostics {
    pub processing_time_ms: u64,
    pub audio_seconds: f64,
    pub speakers_found: u32,
    pub speaker_count_guessed: bool,
    pub locale: String,
}

#[derive(Clone, Debug)]
pub struct AudioEngine {
    worker_path: Arc<PathBuf>,
    diarizer_dir: Arc<PathBuf>,
    /// The macOS product version plus the pinned FluidAudio version, both only
    /// knowable from the handshake.
    version: Arc<str>,
    timeout: Duration,
    max_output_bytes: u64,
    permits: Arc<Semaphore>,
}

impl AudioEngine {
    pub fn initialize(
        worker_path: PathBuf,
        diarizer_dir: PathBuf,
        timeout: Duration,
        max_output_bytes: u64,
    ) -> Result<Self, AudioStartupError> {
        child::validate_worker(&worker_path, WORKER_LABEL)?;
        let metadata = std::fs::metadata(&diarizer_dir).map_err(|source| {
            AudioStartupError::InvalidDiarizer {
                path: diarizer_dir.clone(),
                source,
            }
        })?;
        if !metadata.is_dir() {
            return Err(AudioStartupError::DiarizerNotDirectory(diarizer_dir));
        }
        let version = verify_worker_identity(&worker_path)?;

        Ok(Self {
            worker_path: Arc::new(worker_path),
            diarizer_dir: Arc::new(diarizer_dir),
            version: version.into(),
            timeout,
            max_output_bytes,
            permits: Arc::new(Semaphore::new(1)),
        })
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub async fn acquire(&self) -> Result<OwnedSemaphorePermit, EngineFailure> {
        child::acquire(&self.permits).await
    }

    pub async fn convert(
        &self,
        paths: &AttemptPaths,
        source: ValidatedOpenFile,
        _permit: OwnedSemaphorePermit,
        cancellation: watch::Receiver<bool>,
        media_type: &str,
        speaker_count: Option<u32>,
    ) -> Result<EngineOutcome, EngineFailure> {
        if *cancellation.borrow() {
            return Err(EngineFailure::Interrupted);
        }
        let ValidatedOpenFile {
            file,
            byte_length,
            sha256,
        } = source;
        if byte_length == 0 || !is_lowercase_sha256(&sha256) {
            return Err(EngineFailure::Protocol);
        }
        let source = file.into_std().await;

        let started = Instant::now();
        let mut child = Command::new(self.worker_path.as_path())
            .arg(&paths.publication_staging)
            .current_dir(&paths.attempt)
            .env_clear()
            .env(
                AUDIO_WORKER_MAX_OUTPUT_BYTES_ENV,
                self.max_output_bytes.to_string(),
            )
            .env(
                AUDIO_WORKER_EXPECTED_SOURCE_BYTES_ENV,
                byte_length.to_string(),
            )
            .env(AUDIO_WORKER_EXPECTED_SOURCE_SHA256_ENV, sha256)
            .env(AUDIO_WORKER_MEDIA_TYPE_ENV, media_type)
            .env(AUDIO_WORKER_DIARIZER_DIR_ENV, self.diarizer_dir.as_path())
            // Always sent, empty for "guess", so the worker never falls back to
            // its own default on a request that pinned a count.
            .env(
                AUDIO_WORKER_SPEAKER_COUNT_ENV,
                speaker_count
                    .map(|count| count.to_string())
                    .unwrap_or_default(),
            )
            .stdin(Stdio::from(source))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| EngineFailure::Unavailable)?;
        // A killed worker cannot run its own cleanup, and the attempt survives a
        // requeue, so the parent removes the scratch source copy itself.
        if let Err(failure) = wait_for_child(&mut child, self.timeout, cancellation).await {
            if let Some(extension) = audio_source_extension(media_type) {
                let _ = fs::remove_file(paths.attempt.join(format!("source.{extension}"))).await;
            }
            return Err(failure);
        }

        self.read_and_validate_report(paths, started.elapsed())
            .await
    }

    async fn read_and_validate_report(
        &self,
        paths: &AttemptPaths,
        elapsed: Duration,
    ) -> Result<EngineOutcome, EngineFailure> {
        let report_path = paths.publication_staging.join(AUDIO_WORKER_REPORT_FILE);
        let report: AudioReport = child::read_report(&report_path).await?;
        self.validate_identity(&report)?;

        let outcome = match report.outcome {
            AudioOutcome::Converted { artifact, detail } => {
                let digest = child::validate_staged_markdown(
                    paths,
                    AUDIO_MARKDOWN_FILE,
                    &artifact.relative_path,
                    artifact.byte_length,
                    &artifact.sha256,
                    self.max_output_bytes,
                )
                .await?;
                EngineOutcome::Converted {
                    analysis: analysis(elapsed, detail)?,
                    byte_length: artifact.byte_length,
                    sha256: digest,
                }
            }
            // Every rejection is terminal, so none of them carries an
            // analysis. The desktop answers `needs_remote` by sending the file
            // to Datalab, which cannot transcribe.
            AudioOutcome::Rejected { code } => {
                child::reject_if_markdown_staged(paths).await?;
                EngineOutcome::Rejected {
                    rejection: rejection(code),
                }
            }
        };
        fs::remove_file(report_path)
            .await
            .map_err(|_| EngineFailure::Protocol)?;
        Ok(outcome)
    }

    /// The report must name the same worker the handshake did. One that moved
    /// under us mid-run is not the one startup vetted.
    fn validate_identity(&self, report: &AudioReport) -> Result<(), EngineFailure> {
        if report.protocol_version != AUDIO_WORKER_PROTOCOL_VERSION
            || report.engine.name != AUDIO_ENGINE_NAME
            || report.engine.version != *self.version
            || report.engine.features != WORKER_FEATURES
        {
            return Err(EngineFailure::Protocol);
        }
        Ok(())
    }
}

/// Returns the engine version the worker reported: the macOS product version,
/// `+fluidaudio-`, and the pinned package version. Both halves move with the
/// host and the build, so the check is a shape check, not byte equality.
fn verify_worker_identity(path: &Path) -> Result<String, WorkerStartupError> {
    let output = child::worker_identity_line(path, WORKER_LABEL, WORKER_IDENTITY_TIMEOUT)?;
    let mismatch = || WorkerStartupError::WorkerIdentityMismatch {
        worker: WORKER_LABEL,
        path: path.to_owned(),
    };
    let line = std::str::from_utf8(&output).map_err(|_| mismatch())?;
    let version = line
        .strip_prefix(AUDIO_WORKER_IDENTITY_PREFIX)
        .and_then(|tail| tail.strip_suffix('\n'))
        .ok_or_else(mismatch)?;
    let (os, fluid_audio) = version.split_once("+fluidaudio-").ok_or_else(mismatch)?;
    if !is_dotted_number(os) || !is_dotted_number(fluid_audio) {
        return Err(mismatch());
    }
    Ok(version.to_owned())
}

fn analysis(elapsed: Duration, detail: AudioDetail) -> Result<EngineAnalysis, EngineFailure> {
    // The contract floors speakersFound at one and audioSeconds at zero, so a
    // report under either is a worker out of contract, never a manifest to
    // publish.
    if detail.speakers_found == 0 || detail.audio_seconds < 0.0 {
        return Err(EngineFailure::Protocol);
    }
    Ok(EngineAnalysis {
        classification: DocumentClassification::Audio,
        // A transcript has no pages, no tables and no columns, so every signal
        // the policy reads is a claim this engine cannot make.
        quality: QualitySignals::unmeasured(),
        diagnostics: serde_json::to_value(AudioDiagnostics {
            processing_time_ms: u64::try_from(elapsed.as_millis())
                .map_err(|_| EngineFailure::Protocol)?,
            audio_seconds: detail.audio_seconds,
            speakers_found: detail.speakers_found,
            speaker_count_guessed: detail.speaker_count_guessed,
            locale: detail.locale,
        })
        .map_err(|_| EngineFailure::Protocol)?,
    })
}

/// Maps the worker's rejection wire codes to the engine-owned rejection.
/// The strings are the public failure code and message; do not reword them
/// without a contract review.
fn rejection(code: AudioRejectionCode) -> EngineRejection {
    match code {
        AudioRejectionCode::InvalidAudio => EngineRejection {
            code: "invalid_audio",
            message: "The uploaded file is not readable audio.",
        },
        AudioRejectionCode::NoSpeechFound => EngineRejection {
            code: "no_speech_found",
            message: "No speech was detected in the recording.",
        },
        AudioRejectionCode::LocaleUnsupported => EngineRejection {
            code: "locale_unsupported",
            message: "On-device transcription does not support this language.",
        },
        AudioRejectionCode::SpeechAssetsUnavailable => EngineRejection {
            code: "speech_assets_unavailable",
            message: "The on-device speech model could not be installed.",
        },
        AudioRejectionCode::OutputTooLarge => EngineRejection {
            code: "output_too_large",
            message: "The transcript exceeds the conversion limits.",
        },
    }
}

#[derive(Debug, Error)]
pub enum AudioStartupError {
    #[error(transparent)]
    Startup(#[from] WorkerStartupError),
    #[error("audio diarizer directory is unavailable at {path:?}")]
    InvalidDiarizer {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("audio diarizer path is not a directory: {0:?}")]
    DiarizerNotDirectory(PathBuf),
}

#[cfg(test)]
mod tests {
    use std::{path::Path, time::Duration};

    use sha2::{Digest, Sha256};
    use tokio::sync::watch;

    use super::{
        verify_worker_identity, AudioEngine, EngineFailure, EngineOutcome, WorkerStartupError,
    };
    use crate::artifacts::{AttemptPaths, ValidatedOpenFile};
    use crate::persistence::DocumentClassification;

    const IDENTITY: &str = "26.6.2+fluidaudio-0.15.6";
    const DETAIL: &str = r#""detail":{"audioSeconds":13.5,"speakersFound":2,"speakerCountGuessed":false,"locale":"en-US"}"#;

    #[cfg(unix)]
    fn write_worker(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;

        std::fs::write(path, body).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[cfg(unix)]
    fn worker_script(body: &str) -> String {
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf 'tool-kit-audio-worker protocol=1 local-audio={IDENTITY}\\n'\n  exit 0\nfi\n{body}\n"
        )
    }

    /// A worker that stages a valid transcript and the report that describes it.
    #[cfg(unix)]
    fn converting_worker_script(extra: &str) -> String {
        worker_script(&format!(
            "set -e\n{extra}staging=\"$1\"\nprintf '[00:00] Speaker 1\\nHello.\\n' > \"$staging/result.md\"\n\
             sha=$(/usr/bin/shasum -a 256 \"$staging/result.md\" | cut -d' ' -f1)\n\
             bytes=$(/usr/bin/wc -c < \"$staging/result.md\" | tr -d ' ')\n\
             printf '{{\"protocolVersion\":1,\"engine\":{{\"name\":\"local-audio\",\"version\":\"{IDENTITY}\",\"features\":[\"speech-analyzer\",\"diarization\"]}},\"outcome\":{{\"kind\":\"converted\",\"artifact\":{{\"relativePath\":\"result.md\",\"byteLength\":%s,\"sha256\":\"%s\"}},{DETAIL}}}}}' \"$bytes\" \"$sha\" > \"$staging/worker-report.json\"\n"
        ))
    }

    async fn source(path: &Path, bytes: &[u8]) -> ValidatedOpenFile {
        tokio::fs::write(path, bytes).await.unwrap();
        ValidatedOpenFile {
            file: tokio::fs::File::open(path).await.unwrap(),
            byte_length: u64::try_from(bytes.len()).unwrap(),
            sha256: hex::encode(Sha256::digest(bytes)),
        }
    }

    fn paths(root: &Path) -> AttemptPaths {
        let attempt = root.join("attempt");
        let publication_staging = attempt.join("publication.staging");
        std::fs::create_dir_all(&publication_staging).unwrap();
        AttemptPaths {
            source: root.join("input"),
            published: attempt.join("artifacts"),
            publication_staging,
            attempt,
        }
    }

    /// The models are the engine. A worker without them fails every job, so it
    /// fails the boot instead.
    #[cfg(unix)]
    #[test]
    fn startup_rejects_a_diarizer_directory_that_is_not_one() {
        let directory = tempfile::tempdir().unwrap();
        let worker = directory.path().join("worker");
        write_worker(&worker, &worker_script("exit 0"));
        let not_a_directory = directory.path().join("models");
        std::fs::write(&not_a_directory, "").unwrap();

        assert!(matches!(
            AudioEngine::initialize(
                worker.clone(),
                not_a_directory,
                Duration::from_secs(5),
                1024
            ),
            Err(super::AudioStartupError::DiarizerNotDirectory(_))
        ));
        assert!(matches!(
            AudioEngine::initialize(
                worker,
                directory.path().join("absent"),
                Duration::from_secs(5),
                1024
            ),
            Err(super::AudioStartupError::InvalidDiarizer { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn startup_rejects_a_worker_with_the_wrong_identity() {
        let directory = tempfile::tempdir().unwrap();
        let worker = directory.path().join("wrong-worker");
        write_worker(&worker, "#!/bin/sh\necho wrong-worker\n");

        assert!(matches!(
            verify_worker_identity(&worker),
            Err(WorkerStartupError::WorkerIdentityMismatch { .. })
        ));
    }

    /// The handshake carries two versions that both move under us, so neither
    /// can be compared byte for byte. Both still have to be versions.
    #[cfg(unix)]
    #[test]
    fn startup_rejects_a_handshake_whose_version_is_not_one() {
        for tail in [
            "whenever+fluidaudio-0.15.6",
            "26.6.2+fluidaudio-latest",
            "26.6.2",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let worker = directory.path().join("worker");
            write_worker(
                &worker,
                &format!(
                    "#!/bin/sh\nprintf 'tool-kit-audio-worker protocol=1 local-audio={tail}\\n'\n"
                ),
            );

            assert!(
                matches!(
                    verify_worker_identity(&worker),
                    Err(WorkerStartupError::WorkerIdentityMismatch { .. })
                ),
                "{tail} is not a version pair"
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_converted_report_is_validated_against_the_staged_markdown() {
        let directory = tempfile::tempdir().unwrap();
        let worker = directory.path().join("worker");
        write_worker(&worker, &converting_worker_script(""));
        let models = directory.path().join("models");
        std::fs::create_dir(&models).unwrap();
        let paths = paths(directory.path());
        let source = source(&paths.source, b"audio bytes").await;

        let engine = AudioEngine::initialize(worker, models, Duration::from_secs(5), 1024).unwrap();
        assert_eq!(engine.version(), IDENTITY);
        let permit = engine.acquire().await.unwrap();
        let (_cancel, cancellation) = watch::channel(false);
        let outcome = engine
            .convert(&paths, source, permit, cancellation, "audio/wav", Some(2))
            .await
            .unwrap();

        let EngineOutcome::Converted { analysis, .. } = outcome else {
            panic!("a converted report must convert");
        };
        assert_eq!(analysis.classification, DocumentClassification::Audio);
        assert_eq!(analysis.diagnostics["speakersFound"], 2);
        assert_eq!(analysis.diagnostics["locale"], "en-US");
        assert!(analysis.quality.native_text_ratio.is_none());
        assert!(!paths
            .publication_staging
            .join("worker-report.json")
            .exists());
    }

    /// No local rejection reaches a remote engine: Datalab does not transcribe.
    #[cfg(unix)]
    #[tokio::test]
    async fn every_rejection_is_terminal() {
        for code in [
            "invalid_audio",
            "no_speech_found",
            "locale_unsupported",
            "speech_assets_unavailable",
            "output_too_large",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let worker = directory.path().join("worker");
            let report = format!(
                "{{\"protocolVersion\":1,\"engine\":{{\"name\":\"local-audio\",\"version\":\"{IDENTITY}\",\"features\":[\"speech-analyzer\",\"diarization\"]}},\"outcome\":{{\"kind\":\"rejected\",\"code\":\"{code}\"}}}}"
            );
            write_worker(
                &worker,
                &worker_script(&format!(
                    "printf '%s' '{report}' > \"$1/worker-report.json\"\n"
                )),
            );
            let models = directory.path().join("models");
            std::fs::create_dir(&models).unwrap();
            let paths = paths(directory.path());
            let source = source(&paths.source, b"audio bytes").await;

            let engine =
                AudioEngine::initialize(worker, models, Duration::from_secs(5), 1024).unwrap();
            let permit = engine.acquire().await.unwrap();
            let (_cancel, cancellation) = watch::channel(false);
            let outcome = engine
                .convert(&paths, source, permit, cancellation, "audio/wav", None)
                .await
                .unwrap();

            let EngineOutcome::Rejected { rejection } = outcome else {
                panic!("{code} must end the job here");
            };
            assert_eq!(rejection.code, code);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_worker_that_dies_without_a_report_is_a_crash() {
        let directory = tempfile::tempdir().unwrap();
        let worker = directory.path().join("worker");
        write_worker(&worker, &worker_script("exit 70"));
        let models = directory.path().join("models");
        std::fs::create_dir(&models).unwrap();
        let paths = paths(directory.path());
        let source = source(&paths.source, b"audio bytes").await;

        let engine = AudioEngine::initialize(worker, models, Duration::from_secs(5), 1024).unwrap();
        let permit = engine.acquire().await.unwrap();
        let (_cancel, cancellation) = watch::channel(false);
        let result = engine
            .convert(&paths, source, permit, cancellation, "audio/wav", None)
            .await;

        assert_eq!(result.unwrap_err(), EngineFailure::Crashed);
    }

    /// The contract floors speakersFound at one, so a report of none is a
    /// worker out of contract rather than a transcript.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_report_naming_no_speakers_is_a_protocol_error() {
        let directory = tempfile::tempdir().unwrap();
        let worker = directory.path().join("worker");
        write_worker(
            &worker,
            &converting_worker_script("").replace("\"speakersFound\":2", "\"speakersFound\":0"),
        );
        let models = directory.path().join("models");
        std::fs::create_dir(&models).unwrap();
        let paths = paths(directory.path());
        let source = source(&paths.source, b"audio bytes").await;

        let engine = AudioEngine::initialize(worker, models, Duration::from_secs(5), 1024).unwrap();
        let permit = engine.acquire().await.unwrap();
        let (_cancel, cancellation) = watch::channel(false);
        let result = engine
            .convert(&paths, source, permit, cancellation, "audio/wav", None)
            .await;

        assert_eq!(result.unwrap_err(), EngineFailure::Protocol);
    }

    /// The contract floors audioSeconds at zero, so a negative duration is a
    /// worker out of contract and must not reach a published manifest.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_report_with_a_negative_duration_is_a_protocol_error() {
        let directory = tempfile::tempdir().unwrap();
        let worker = directory.path().join("worker");
        write_worker(
            &worker,
            &converting_worker_script("").replace("\"audioSeconds\":13.5", "\"audioSeconds\":-1"),
        );
        let models = directory.path().join("models");
        std::fs::create_dir(&models).unwrap();
        let paths = paths(directory.path());
        let source = source(&paths.source, b"audio bytes").await;

        let engine = AudioEngine::initialize(worker, models, Duration::from_secs(5), 1024).unwrap();
        let permit = engine.acquire().await.unwrap();
        let (_cancel, cancellation) = watch::channel(false);
        let result = engine
            .convert(&paths, source, permit, cancellation, "audio/wav", None)
            .await;

        assert_eq!(result.unwrap_err(), EngineFailure::Protocol);
    }

    /// A killed worker never runs its own cleanup, and a requeue mints a new
    /// attempt, so the parent removes the scratch source copy it left behind.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_timed_out_worker_leaves_no_scratch_source_behind() {
        let directory = tempfile::tempdir().unwrap();
        let worker = directory.path().join("worker");
        write_worker(
            &worker,
            &worker_script("printf 'audio' > source.wav\n/bin/sleep 2\n"),
        );
        let models = directory.path().join("models");
        std::fs::create_dir(&models).unwrap();
        let paths = paths(directory.path());
        let source = source(&paths.source, b"audio bytes").await;

        let engine =
            AudioEngine::initialize(worker, models, Duration::from_millis(200), 1024).unwrap();
        let permit = engine.acquire().await.unwrap();
        let (_cancel, cancellation) = watch::channel(false);
        let result = engine
            .convert(&paths, source, permit, cancellation, "audio/wav", None)
            .await;

        assert_eq!(result.unwrap_err(), EngineFailure::Timeout);
        assert!(!paths.attempt.join("source.wav").exists());
    }

    /// The media type names the worker's scratch file, the diarizer directory
    /// is the only path to the models, and the speaker count is always sent so
    /// a blank one means guess rather than "use your own default".
    #[cfg(unix)]
    #[tokio::test]
    async fn the_worker_receives_the_media_type_diarizer_and_speaker_count() {
        for (speaker_count, expected) in [(Some(3_u32), "3"), (None, "")] {
            let directory = tempfile::tempdir().unwrap();
            let worker = directory.path().join("worker");
            write_worker(
                &worker,
                &converting_worker_script(
                    "printf '%s\\n%s\\n%s\\n%s\\n' \"$TOOLKIT_AUDIO_WORKER_MEDIA_TYPE\" \
                     \"$TOOLKIT_AUDIO_WORKER_DIARIZER_DIR\" \
                     \"$TOOLKIT_AUDIO_WORKER_SPEAKER_COUNT\" \
                     \"$TOOLKIT_AUDIO_WORKER_LOCALE\" > env.txt\n",
                ),
            );
            let models = directory.path().join("models");
            std::fs::create_dir(&models).unwrap();
            let paths = paths(directory.path());
            let source = source(&paths.source, b"audio bytes").await;

            let engine =
                AudioEngine::initialize(worker, models.clone(), Duration::from_secs(5), 1024)
                    .unwrap();
            let permit = engine.acquire().await.unwrap();
            let (_cancel, cancellation) = watch::channel(false);
            engine
                .convert(
                    &paths,
                    source,
                    permit,
                    cancellation,
                    "audio/mpeg",
                    speaker_count,
                )
                .await
                .unwrap();

            let env = std::fs::read_to_string(paths.attempt.join("env.txt")).unwrap();
            let lines: Vec<&str> = env.lines().collect();
            assert_eq!(lines[0], "audio/mpeg");
            assert_eq!(lines[1], models.to_str().unwrap());
            assert_eq!(lines[2], expected);
            // v1 has no locale setting; the worker follows the system locale.
            assert_eq!(lines[3], "");
        }
    }
}
