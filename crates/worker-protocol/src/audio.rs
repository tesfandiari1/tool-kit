//! The Swift audio worker's report contract.
//!
//! Deliberately separate from [`crate::vision`]: the two workers share the
//! spawn conventions only (source bytes on stdin, the staging directory as
//! argv[1], a report file written into it, a `--version` handshake, exit 0 or
//! 70). An audio report carries speaker and duration detail no image report
//! has, and its rejection codes are its own.
//!
//! The Swift side is `workers/audio/Sources/tool-kit-audio-worker`. Keep the
//! two in sync.

use serde::{Deserialize, Serialize};

pub const AUDIO_WORKER_PROTOCOL_VERSION: u32 = 1;
pub const AUDIO_ENGINE_NAME: &str = "local-audio";
pub const AUDIO_WORKER_REPORT_FILE: &str = "worker-report.json";
pub const AUDIO_MARKDOWN_FILE: &str = "result.md";

/// The handshake line is this prefix plus the engine version,
/// `"<macOS product version>+fluidaudio-<FluidAudio version>"`, because the
/// worker's behaviour is the OS speech stack plus one pinned Swift package.
pub const AUDIO_WORKER_IDENTITY_PREFIX: &str = "tool-kit-audio-worker protocol=1 local-audio=";

/// The source-binding names are the shared worker ones: identical meaning,
/// identical parent-side values. See [`crate::pdf`], which declares them for
/// the PDF worker. They are repeated rather than imported so this module stays
/// free of that one, which is the whole point of a per-engine protocol.
pub const AUDIO_WORKER_EXPECTED_SOURCE_BYTES_ENV: &str = "TOOLKIT_WORKER_EXPECTED_SOURCE_BYTES";
pub const AUDIO_WORKER_EXPECTED_SOURCE_SHA256_ENV: &str = "TOOLKIT_WORKER_EXPECTED_SOURCE_SHA256";
pub const AUDIO_WORKER_MAX_OUTPUT_BYTES_ENV: &str = "TOOLKIT_WORKER_MAX_OUTPUT_BYTES";

/// The admitted media type. The worker names its scratch file from it because
/// `AVAudioFile` reads the extension.
pub const AUDIO_WORKER_MEDIA_TYPE_ENV: &str = "TOOLKIT_AUDIO_WORKER_MEDIA_TYPE";
/// Absent or empty means guess; a positive integer pins the diarizer to
/// exactly that many speakers.
pub const AUDIO_WORKER_SPEAKER_COUNT_ENV: &str = "TOOLKIT_AUDIO_WORKER_SPEAKER_COUNT";
/// BCP 47. Absent or empty means the system locale.
pub const AUDIO_WORKER_LOCALE_ENV: &str = "TOOLKIT_AUDIO_WORKER_LOCALE";
/// The staged speaker-diarization CoreML directory. Unset is fatal at startup,
/// never a silent fallback.
pub const AUDIO_WORKER_DIARIZER_DIR_ENV: &str = "TOOLKIT_AUDIO_WORKER_DIARIZER_DIR";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioReport {
    pub protocol_version: u32,
    pub engine: AudioEngineIdentity,
    pub outcome: AudioOutcome,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AudioEngineIdentity {
    pub name: String,
    /// The macOS product version plus the pinned FluidAudio version.
    pub version: String,
    pub features: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AudioOutcome {
    Converted {
        artifact: AudioArtifact,
        detail: AudioDetail,
    },
    Rejected {
        code: AudioRejectionCode,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioArtifact {
    pub relative_path: String,
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDetail {
    pub audio_seconds: f64,
    pub speakers_found: u32,
    pub speaker_count_guessed: bool,
    pub locale: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioRejectionCode {
    /// The bytes carry no audio track any installed decoder can read.
    InvalidAudio,
    /// The audio decoded and the speech stack recognized no words in it.
    NoSpeechFound,
    /// The requested locale has no speech assets Apple will install.
    LocaleUnsupported,
    /// The speech assets are missing and the OS refused to install them.
    SpeechAssetsUnavailable,
    /// The Markdown exceeded the run's output ceiling, so nothing was written.
    OutputTooLarge,
}

/// The scratch file name the worker gives an admitted source, mirroring
/// `sourceExtensions` in the Swift worker's `protocol.swift`. `AVAudioFile`
/// reads the extension, so the two tables have to agree.
#[must_use]
pub fn audio_source_extension(media_type: &str) -> Option<&'static str> {
    Some(match media_type {
        "audio/wav" => "wav",
        "audio/mp4" => "m4a",
        "video/mp4" => "mp4",
        "video/quicktime" => "mov",
        "audio/mpeg" => "mp3",
        "audio/flac" => "flac",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // The Swift side is written against these exact strings.
    #[test]
    fn reports_serialize_to_the_documented_json() {
        let converted = AudioReport {
            protocol_version: AUDIO_WORKER_PROTOCOL_VERSION,
            engine: AudioEngineIdentity {
                name: AUDIO_ENGINE_NAME.to_string(),
                version: "26.6.1+fluidaudio-0.15.6".to_string(),
                features: vec!["diarization".to_string()],
            },
            outcome: AudioOutcome::Converted {
                artifact: AudioArtifact {
                    relative_path: AUDIO_MARKDOWN_FILE.to_string(),
                    byte_length: 1234,
                    sha256: "abc123".to_string(),
                },
                detail: AudioDetail {
                    audio_seconds: 972.5,
                    speakers_found: 2,
                    speaker_count_guessed: false,
                    locale: "en-US".to_string(),
                },
            },
        };
        assert_eq!(
            serde_json::to_string(&converted).unwrap(),
            r#"{"protocolVersion":1,"engine":{"name":"local-audio","version":"26.6.1+fluidaudio-0.15.6","features":["diarization"]},"outcome":{"kind":"converted","artifact":{"relativePath":"result.md","byteLength":1234,"sha256":"abc123"},"detail":{"audioSeconds":972.5,"speakersFound":2,"speakerCountGuessed":false,"locale":"en-US"}}}"#
        );

        let rejected = AudioReport {
            protocol_version: AUDIO_WORKER_PROTOCOL_VERSION,
            engine: AudioEngineIdentity {
                name: AUDIO_ENGINE_NAME.to_string(),
                version: "26.6.1+fluidaudio-0.15.6".to_string(),
                features: vec![],
            },
            outcome: AudioOutcome::Rejected {
                code: AudioRejectionCode::NoSpeechFound,
            },
        };
        assert_eq!(
            serde_json::to_string(&rejected).unwrap(),
            r#"{"protocolVersion":1,"engine":{"name":"local-audio","version":"26.6.1+fluidaudio-0.15.6","features":[]},"outcome":{"kind":"rejected","code":"no_speech_found"}}"#
        );
    }
}
