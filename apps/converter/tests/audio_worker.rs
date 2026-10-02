#![allow(clippy::unwrap_used)]

//! Grades the Swift audio worker's protocol, not its transcript quality.
//!
//! Every test returns early when the worker binary or the staged diarizer is
//! missing. `workers/audio/build.sh` writes both, and refuses to run off
//! macOS, so Linux CI and a fresh checkout reach this file with nothing to
//! spawn. A skip there is the honest answer; a failure would only say the
//! platform is not macOS.

use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
};

use sha2::{Digest, Sha256};

use tool_kit_converter::audio_protocol::{
    AudioOutcome, AudioRejectionCode, AudioReport, AUDIO_ENGINE_NAME, AUDIO_MARKDOWN_FILE,
    AUDIO_WORKER_DIARIZER_DIR_ENV, AUDIO_WORKER_EXPECTED_SOURCE_BYTES_ENV,
    AUDIO_WORKER_EXPECTED_SOURCE_SHA256_ENV, AUDIO_WORKER_IDENTITY_PREFIX,
    AUDIO_WORKER_MAX_OUTPUT_BYTES_ENV, AUDIO_WORKER_MEDIA_TYPE_ENV, AUDIO_WORKER_PROTOCOL_VERSION,
    AUDIO_WORKER_REPORT_FILE, AUDIO_WORKER_SPEAKER_COUNT_ENV,
};

const MAX_OUTPUT_BYTES: u64 = 1024 * 1024;

struct Tools {
    worker: PathBuf,
    /// The parent FluidAudio populates: it appends `speaker-diarization/`
    /// itself, so the env names the repo directory above that.
    diarizer: String,
}

fn tools() -> Option<Tools> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    // The worker is not part of this crate, so this climbs out of
    // apps/converter/ to the repo root to reach it. A wrong path here skips
    // every test in the file silently, which is the same thing a missing
    // binary means.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../workers/audio");
    let worker = root.join("bin/tool-kit-audio-worker");
    let diarizer = root.join("models/speaker-diarization-coreml");
    if !worker.is_file() || !diarizer.is_dir() {
        // Said out loud: a silent skip reads as a suite that asserted something.
        eprintln!("SKIPPED: run workers/audio/build.sh to stage the worker and its models");
        return None;
    }
    Some(Tools {
        diarizer: diarizer.to_str().unwrap().to_string(),
        worker,
    })
}

/// The fixture with its audio cut to `kept` bytes. Both WAV fixtures are
/// 16 kHz mono 16-bit, so a second is 32,000 bytes and a trim is the data
/// chunk cut short with the two lengths rewritten.
fn trimmed(source: &Path, directory: &Path, kept: usize) -> PathBuf {
    let bytes = fs::read(source).unwrap();
    let mut offset = 12;
    while &bytes[offset..offset + 4] != b"data" {
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        offset += 8 + size + (size & 1);
    }
    let mut out = bytes[..offset + 8 + kept].to_vec();
    out[offset + 4..offset + 8].copy_from_slice(&u32::try_from(kept).unwrap().to_le_bytes());
    let riff = u32::try_from(out.len() - 8).unwrap();
    out[4..8].copy_from_slice(&riff.to_le_bytes());
    let path = directory.join("trimmed.wav");
    fs::write(&path, out).unwrap();
    path
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio")
        .join(name)
}

/// The worker reports Foundation's three components; `sw_vers` drops a
/// trailing zero, so pad before comparing.
fn macos_product_version() -> String {
    let output = Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .unwrap();
    let printed = String::from_utf8(output.stdout).unwrap();
    let mut parts: Vec<&str> = printed.trim().split('.').collect();
    while parts.len() < 3 {
        parts.push("0");
    }
    parts.join(".")
}

struct Run {
    staging: PathBuf,
    status: ExitStatus,
}

/// Spawns the worker exactly as `engines::vision` spawns the Vision worker:
/// the staging directory as argv[1], the attempt directory as the cwd, a
/// cleared environment, the source on stdin, and both output streams on
/// /dev/null. Every test runs through it, so the cleared environment is what
/// every assertion below is made under. A `settings` entry replaces the base
/// environment, which is how a wrong byte count or a low ceiling gets in.
fn run_worker(
    worker: &Path,
    root: &Path,
    source: &Path,
    sha256: &str,
    settings: &[(&str, &str)],
) -> Run {
    let attempt = root.join("attempt");
    let staging = attempt.join("publication.staging");
    fs::create_dir_all(&staging).unwrap();
    let byte_length = fs::metadata(source).unwrap().len();

    let mut command = Command::new(worker);
    command
        .arg(&staging)
        .current_dir(&attempt)
        .env_clear()
        .env(
            AUDIO_WORKER_MAX_OUTPUT_BYTES_ENV,
            MAX_OUTPUT_BYTES.to_string(),
        )
        .env(
            AUDIO_WORKER_EXPECTED_SOURCE_BYTES_ENV,
            byte_length.to_string(),
        )
        .env(AUDIO_WORKER_EXPECTED_SOURCE_SHA256_ENV, sha256)
        .stdin(Stdio::from(File::open(source).unwrap()))
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (name, value) in settings {
        command.env(name, value);
    }
    let status = command.status().unwrap();
    Run { staging, status }
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn read_report(staging: &Path) -> AudioReport {
    let encoded = fs::read(staging.join(AUDIO_WORKER_REPORT_FILE)).unwrap();
    serde_json::from_slice(&encoded).unwrap()
}

#[test]
fn the_handshake_line_is_the_identity_the_contract_declares() {
    let Some(tools) = tools() else {
        return;
    };

    let output = Command::new(&tools.worker)
        .arg("--version")
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(output.status.success());
    let line = String::from_utf8(output.stdout).unwrap();
    let version = line.strip_prefix(AUDIO_WORKER_IDENTITY_PREFIX).unwrap();
    // The engine is the OS speech stack plus the pinned package, so the version
    // the parent parses names both. The package pin itself is the worker's to
    // declare, so only its marker is asserted here.
    assert!(
        version.starts_with(&format!("{}+fluidaudio-", macos_product_version())),
        "unexpected identity line: {line}"
    );
}

/// Also the cleared-environment case: the worker gets no PATH, HOME, or TMPDIR
/// and still loads the speech stack, reads stdin, and publishes into the
/// staging directory.
#[test]
fn two_speakers_with_the_count_pinned_transcribe_into_named_turns() {
    let Some(tools) = tools() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let source = fixture("two-speakers.wav");
    let sha256 = digest(&fs::read(&source).unwrap());

    let run = run_worker(
        &tools.worker,
        directory.path(),
        &source,
        &sha256,
        &[
            (AUDIO_WORKER_MEDIA_TYPE_ENV, "audio/wav"),
            (AUDIO_WORKER_DIARIZER_DIR_ENV, &tools.diarizer),
            (AUDIO_WORKER_SPEAKER_COUNT_ENV, "2"),
        ],
    );

    assert!(run.status.success(), "worker exited {:?}", run.status);
    let report = read_report(&run.staging);
    assert_eq!(report.protocol_version, AUDIO_WORKER_PROTOCOL_VERSION);
    assert_eq!(report.engine.name, AUDIO_ENGINE_NAME);
    assert!(report.engine.version.starts_with(&macos_product_version()));

    let AudioOutcome::Converted { artifact, detail } = report.outcome else {
        panic!("a two speaker recording should transcribe");
    };
    assert_eq!(artifact.relative_path, AUDIO_MARKDOWN_FILE);
    let markdown = fs::read(run.staging.join(AUDIO_MARKDOWN_FILE)).unwrap();
    assert_eq!(artifact.byte_length, markdown.len() as u64);
    assert_eq!(artifact.sha256, digest(&markdown));
    assert_eq!(detail.speakers_found, 2);
    assert!(!detail.speaker_count_guessed);

    let markdown = String::from_utf8(markdown).unwrap();
    assert!(
        markdown.contains("Speaker 1") && markdown.contains("Speaker 2"),
        "a pinned count of two should name two speakers:\n{markdown}"
    );
    assert!(
        !markdown.contains("Speakers guessed"),
        "a pinned count is not a guess:\n{markdown}"
    );
}

/// The count the reader should doubt is the one the header names.
#[test]
fn a_run_with_no_speaker_count_says_the_count_was_guessed() {
    let Some(tools) = tools() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let source = fixture("two-speakers.wav");
    let sha256 = digest(&fs::read(&source).unwrap());

    let run = run_worker(
        &tools.worker,
        directory.path(),
        &source,
        &sha256,
        &[
            (AUDIO_WORKER_MEDIA_TYPE_ENV, "audio/wav"),
            (AUDIO_WORKER_DIARIZER_DIR_ENV, &tools.diarizer),
        ],
    );

    assert!(run.status.success(), "worker exited {:?}", run.status);
    let AudioOutcome::Converted { detail, .. } = read_report(&run.staging).outcome else {
        panic!("a guessed count is still a transcript");
    };
    assert!(detail.speaker_count_guessed);
    let markdown = fs::read_to_string(run.staging.join(AUDIO_MARKDOWN_FILE)).unwrap();
    assert!(
        markdown
            .lines()
            .next()
            .unwrap()
            .starts_with("Speakers guessed:"),
        "the guess belongs in the first line:\n{markdown}"
    );
}

/// One clip per admitted container, all from the same voice, so the only thing
/// under test is whether the worker's scratch extension lets AVAudioFile open
/// it. A pinned count of one skips the diarizer entirely.
#[test]
fn every_admitted_container_decodes_to_a_single_speaker_transcript() {
    let Some(tools) = tools() else {
        return;
    };
    for (name, media_type) in [
        ("clip.m4a", "audio/mp4"),
        ("clip.mp4", "video/mp4"),
        ("clip.mov", "video/quicktime"),
        ("clip.mp3", "audio/mpeg"),
        ("clip.flac", "audio/flac"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = fixture(name);
        let sha256 = digest(&fs::read(&source).unwrap());

        let run = run_worker(
            &tools.worker,
            directory.path(),
            &source,
            &sha256,
            &[
                (AUDIO_WORKER_MEDIA_TYPE_ENV, media_type),
                (AUDIO_WORKER_DIARIZER_DIR_ENV, &tools.diarizer),
                (AUDIO_WORKER_SPEAKER_COUNT_ENV, "1"),
            ],
        );

        assert!(run.status.success(), "{name} exited {:?}", run.status);
        let AudioOutcome::Converted { detail, .. } = read_report(&run.staging).outcome else {
            panic!("{name} carries the same speech every other container does");
        };
        assert_eq!(detail.speakers_found, 1, "{name}");
        let markdown = fs::read_to_string(run.staging.join(AUDIO_MARKDOWN_FILE)).unwrap();
        assert!(!markdown.trim().is_empty(), "{name} published nothing");
    }
}

/// The recogniser and the diarizer disagree about short speech: FluidAudio's
/// detector finds none where SpeechAnalyzer read words. That is one speaker,
/// not a crashed worker.
#[test]
fn a_clip_the_diarizer_hears_no_speech_in_still_transcribes_as_one_speaker() {
    let Some(tools) = tools() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let source = trimmed(
        &fixture("two-speakers.wav"),
        directory.path(),
        32_000 * 12 / 10,
    );
    let sha256 = digest(&fs::read(&source).unwrap());

    let run = run_worker(
        &tools.worker,
        directory.path(),
        &source,
        &sha256,
        &[
            (AUDIO_WORKER_MEDIA_TYPE_ENV, "audio/wav"),
            (AUDIO_WORKER_DIARIZER_DIR_ENV, &tools.diarizer),
        ],
    );

    assert!(run.status.success(), "worker exited {:?}", run.status);
    let AudioOutcome::Converted { detail, .. } = read_report(&run.staging).outcome else {
        panic!("the recogniser read words, so this is a transcript");
    };
    assert_eq!(detail.speakers_found, 1);
    let markdown = fs::read_to_string(run.staging.join(AUDIO_MARKDOWN_FILE)).unwrap();
    assert!(
        markdown.contains("Speaker 1") && !markdown.contains("unknown"),
        "every turn needs a speaker the count names:\n{markdown}"
    );
}

#[test]
fn audio_holding_no_words_is_rejected_rather_than_converted_empty() {
    let Some(tools) = tools() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let source = fixture("silence.wav");
    let sha256 = digest(&fs::read(&source).unwrap());

    let run = run_worker(
        &tools.worker,
        directory.path(),
        &source,
        &sha256,
        &[
            (AUDIO_WORKER_MEDIA_TYPE_ENV, "audio/wav"),
            (AUDIO_WORKER_DIARIZER_DIR_ENV, &tools.diarizer),
        ],
    );

    assert!(run.status.success(), "worker exited {:?}", run.status);
    let AudioOutcome::Rejected { code } = read_report(&run.staging).outcome else {
        panic!("two seconds of silence is not a transcript");
    };
    assert_eq!(code, AudioRejectionCode::NoSpeechFound);
    assert!(!run.staging.join(AUDIO_MARKDOWN_FILE).exists());
}

#[test]
fn bytes_no_decoder_can_read_are_rejected_as_invalid_audio() {
    let Some(tools) = tools() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let source = fixture("garbage.bin");
    let sha256 = digest(&fs::read(&source).unwrap());

    let run = run_worker(
        &tools.worker,
        directory.path(),
        &source,
        &sha256,
        &[
            (AUDIO_WORKER_MEDIA_TYPE_ENV, "audio/wav"),
            (AUDIO_WORKER_DIARIZER_DIR_ENV, &tools.diarizer),
        ],
    );

    assert!(run.status.success(), "worker exited {:?}", run.status);
    let AudioOutcome::Rejected { code } = read_report(&run.staging).outcome else {
        panic!("random bytes declared as WAV hold no audio track");
    };
    assert_eq!(code, AudioRejectionCode::InvalidAudio);
    assert!(!run.staging.join(AUDIO_MARKDOWN_FILE).exists());
}

/// A 1 Hz WAV opens as audio but the speech analyzer cannot convert it. That
/// is an engine failure, so it exits 70 rather than filing the install-only
/// `speech_assets_unavailable`.
#[test]
fn an_analyzer_error_fails_the_run_instead_of_blaming_the_speech_assets() {
    let Some(tools) = tools() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    // 16-bit mono PCM at 1 Hz, 64 silent frames.
    let mut bytes = b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0".to_vec();
    for field in [1u32, 2] {
        bytes.extend_from_slice(&field.to_le_bytes()); // sample rate, byte rate
    }
    bytes.extend_from_slice(b"\x02\0\x10\0data\x80\0\0\0");
    bytes.extend_from_slice(&[0; 128]);
    let riff = u32::try_from(bytes.len() - 8).unwrap();
    bytes[4..8].copy_from_slice(&riff.to_le_bytes());
    let source = directory.path().join("one-hertz.wav");
    fs::write(&source, &bytes).unwrap();

    let run = run_worker(
        &tools.worker,
        directory.path(),
        &source,
        &digest(&bytes),
        &[
            (AUDIO_WORKER_MEDIA_TYPE_ENV, "audio/wav"),
            (AUDIO_WORKER_DIARIZER_DIR_ENV, &tools.diarizer),
        ],
    );

    assert_eq!(run.status.code(), Some(70));
    assert!(!run.staging.join(AUDIO_WORKER_REPORT_FILE).exists());
    assert!(!run.staging.join(AUDIO_MARKDOWN_FILE).exists());
}

/// A source that does not match its binding is a broken spawn, not a bad
/// recording, so it fails the run instead of filing a rejection.
#[test]
fn a_source_that_does_not_match_its_binding_fails_the_run_and_publishes_nothing() {
    let Some(tools) = tools() else {
        return;
    };
    let source = fixture("clip.m4a");
    let bytes = fs::read(&source).unwrap();
    let sha256 = digest(&bytes);
    let one_short = (bytes.len() - 1).to_string();
    let wrong_digest = digest(b"some other bytes");

    for settings in [
        (AUDIO_WORKER_EXPECTED_SOURCE_BYTES_ENV, one_short.as_str()),
        (
            AUDIO_WORKER_EXPECTED_SOURCE_SHA256_ENV,
            wrong_digest.as_str(),
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut env = vec![
            (AUDIO_WORKER_MEDIA_TYPE_ENV, "audio/mp4"),
            (AUDIO_WORKER_DIARIZER_DIR_ENV, tools.diarizer.as_str()),
        ];
        env.push(settings);

        let run = run_worker(&tools.worker, directory.path(), &source, &sha256, &env);

        assert_eq!(run.status.code(), Some(70));
        assert!(!run.staging.join(AUDIO_WORKER_REPORT_FILE).exists());
        assert!(!run.staging.join(AUDIO_MARKDOWN_FILE).exists());
    }
}

#[test]
fn markdown_over_the_ceiling_is_rejected_and_nothing_is_published() {
    let Some(tools) = tools() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let source = fixture("two-speakers.wav");
    let sha256 = digest(&fs::read(&source).unwrap());

    let run = run_worker(
        &tools.worker,
        directory.path(),
        &source,
        &sha256,
        &[
            (AUDIO_WORKER_MEDIA_TYPE_ENV, "audio/wav"),
            (AUDIO_WORKER_DIARIZER_DIR_ENV, &tools.diarizer),
            (AUDIO_WORKER_SPEAKER_COUNT_ENV, "2"),
            (AUDIO_WORKER_MAX_OUTPUT_BYTES_ENV, "16"),
        ],
    );

    assert!(run.status.success(), "worker exited {:?}", run.status);
    let AudioOutcome::Rejected { code } = read_report(&run.staging).outcome else {
        panic!("a sixteen byte ceiling cannot hold two speaker turns");
    };
    assert_eq!(code, AudioRejectionCode::OutputTooLarge);
    assert!(!run.staging.join(AUDIO_MARKDOWN_FILE).exists());
}

/// The models directory is fatal when unset rather than a silent fallback to
/// FluidAudio's own cache, which is the trap `TOOLKIT_CONVERTER_PDF_BCMAPS_DIR`
/// already sprang once.
#[test]
fn a_missing_diarizer_directory_fails_the_run_before_any_work() {
    let Some(tools) = tools() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let source = fixture("two-speakers.wav");
    let sha256 = digest(&fs::read(&source).unwrap());

    let run = run_worker(
        &tools.worker,
        directory.path(),
        &source,
        &sha256,
        &[(AUDIO_WORKER_MEDIA_TYPE_ENV, "audio/wav")],
    );

    assert_eq!(run.status.code(), Some(70));
    assert!(!run.staging.join(AUDIO_WORKER_REPORT_FILE).exists());
}

/// The worker names its scratch copy from the media type, so one it does not
/// know is a spawn the converter should never have made.
#[test]
fn a_media_type_the_worker_does_not_know_fails_the_run() {
    let Some(tools) = tools() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let source = fixture("two-speakers.wav");
    let sha256 = digest(&fs::read(&source).unwrap());

    let run = run_worker(
        &tools.worker,
        directory.path(),
        &source,
        &sha256,
        &[
            (AUDIO_WORKER_MEDIA_TYPE_ENV, "application/pdf"),
            (AUDIO_WORKER_DIARIZER_DIR_ENV, &tools.diarizer),
        ],
    );

    assert_eq!(run.status.code(), Some(70));
    assert!(!run.staging.join(AUDIO_WORKER_REPORT_FILE).exists());
}
