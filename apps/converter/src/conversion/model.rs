use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::fs::File;
use uuid::Uuid;

use super::service::{ARTIFACT_INTEGRITY_CODE, ARTIFACT_INTEGRITY_MESSAGE};
use super::{ArtifactKind, ConversionProfile, JobStatus};
use crate::persistence::{StoredArtifact, StoredConversion};

/// One advertised upload format: the accepted extension, the canonical media
/// type stored with the source, the container signature the upload path
/// requires, and the local engine that converts it. Authoritative format
/// detection happens from content at execution time; this table is the
/// admission boundary.
///
/// A format appears here only with a passing round-trip fixture, and the
/// newest migration's CHECK constraint matches this table exactly. That pairing
/// is enforced by `advertised_media_types_match_the_migration_check`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SourceFormat {
    pub extension: &'static str,
    pub media_type: &'static str,
    pub magic: ContainerMagic,
    pub engine: LocalEngineKind,
    /// The family label AnyDoc reports in its diagnostics; used to check
    /// manifest consistency for AnyDoc jobs. Unused for PDF and Vision.
    pub format_label: &'static str,
}

impl SourceFormat {
    /// Whether the engine that converts this format came up in this process.
    /// Only the spawned Swift workers can be absent normally: a broken PDF
    /// worker fails startup, and AnyDoc runs in-process.
    pub(crate) fn is_servable(&self, available: EngineAvailability) -> bool {
        match self.engine {
            LocalEngineKind::Pdf | LocalEngineKind::AnyDoc => true,
            LocalEngineKind::Vision => available.vision,
            LocalEngineKind::Audio => available.audio,
        }
    }
}

/// Which optional engines came up in this process. A record rather than one
/// positional bool per engine, because a second bool is how audio ends up
/// gated on the Vision worker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EngineAvailability {
    pub vision: bool,
    pub audio: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContainerMagic {
    Pdf,
    Zip,
    Ole,
    /// `{\rtf`, the group that must open every RTF file.
    Rtf,
    // One variant per raster signature rather than a shared `Image`: a shared
    // one would admit a `.png` upload carrying JPEG bytes.
    Png,
    Jpeg,
    Webp,
    Tiff,
    Gif,
    Bmp,
    /// `RIFF` then `WAVE` at byte 8. The four bytes after the length name the
    /// payload, which is what separates a WAV from a WebP or an AVI.
    Wave,
    /// `ftyp` at byte 4. m4a, mp4 and mov are one container under three media
    /// types; the worker treats them alike and the brand does not gate
    /// admission.
    IsoBmff,
    /// An `ID3` tag or a bare MPEG frame sync. An MP3 may start with either.
    MpegAudio,
    Flac,
    /// CSV carries no signature at all, so admission cannot check one. The
    /// extension and declared media type are the whole gate, and the engine
    /// names the format explicitly because detection returns `None`.
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LocalEngineKind {
    Pdf,
    AnyDoc,
    Vision,
    Audio,
}

pub(crate) const SOURCE_FORMATS: &[SourceFormat] = &[
    SourceFormat {
        extension: "pdf",
        media_type: "application/pdf",
        magic: ContainerMagic::Pdf,
        engine: LocalEngineKind::Pdf,
        format_label: "pdf",
    },
    SourceFormat {
        extension: "docx",
        media_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "docx",
    },
    SourceFormat {
        extension: "doc",
        media_type: "application/msword",
        magic: ContainerMagic::Ole,
        engine: LocalEngineKind::AnyDoc,
        format_label: "doc",
    },
    SourceFormat {
        extension: "pptx",
        media_type: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "pptx",
    },
    SourceFormat {
        extension: "ppt",
        media_type: "application/vnd.ms-powerpoint",
        magic: ContainerMagic::Ole,
        engine: LocalEngineKind::AnyDoc,
        format_label: "ppt",
    },
    SourceFormat {
        extension: "xlsx",
        media_type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "excel",
    },
    SourceFormat {
        extension: "xls",
        media_type: "application/vnd.ms-excel",
        magic: ContainerMagic::Ole,
        engine: LocalEngineKind::AnyDoc,
        format_label: "excel",
    },
    SourceFormat {
        extension: "epub",
        media_type: "application/epub+zip",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "epub",
    },
    SourceFormat {
        extension: "odt",
        media_type: "application/vnd.oasis.opendocument.text",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "odt",
    },
    SourceFormat {
        extension: "ods",
        media_type: "application/vnd.oasis.opendocument.spreadsheet",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "ods",
    },
    SourceFormat {
        extension: "odp",
        media_type: "application/vnd.oasis.opendocument.presentation",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "odp",
    },
    SourceFormat {
        extension: "rtf",
        media_type: "application/rtf",
        magic: ContainerMagic::Rtf,
        engine: LocalEngineKind::AnyDoc,
        format_label: "rtf",
    },
    SourceFormat {
        extension: "csv",
        media_type: "text/csv",
        magic: ContainerMagic::None,
        engine: LocalEngineKind::AnyDoc,
        format_label: "csv",
    },
    // Extension variants of the families above. AnyDoc routes each to the same
    // parser, so they carry the same `format_label`; only admission differs.
    SourceFormat {
        extension: "docm",
        media_type: "application/vnd.ms-word.document.macroEnabled.12",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "docx",
    },
    SourceFormat {
        extension: "xlsm",
        media_type: "application/vnd.ms-excel.sheet.macroEnabled.12",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "excel",
    },
    SourceFormat {
        extension: "pptm",
        media_type: "application/vnd.ms-powerpoint.presentation.macroEnabled.12",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "pptx",
    },
    SourceFormat {
        extension: "ppsx",
        media_type: "application/vnd.openxmlformats-officedocument.presentationml.slideshow",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "pptx",
    },
    SourceFormat {
        extension: "ppsm",
        media_type: "application/vnd.ms-powerpoint.slideshow.macroEnabled.12",
        magic: ContainerMagic::Zip,
        engine: LocalEngineKind::AnyDoc,
        format_label: "pptx",
    },
    // `pps` and `pot` are the same OLE container as `ppt` and share its media
    // type; the extension only tells PowerPoint how to open the file.
    SourceFormat {
        extension: "pps",
        media_type: "application/vnd.ms-powerpoint",
        magic: ContainerMagic::Ole,
        engine: LocalEngineKind::AnyDoc,
        format_label: "ppt",
    },
    SourceFormat {
        extension: "pot",
        media_type: "application/vnd.ms-powerpoint",
        magic: ContainerMagic::Ole,
        engine: LocalEngineKind::AnyDoc,
        format_label: "ppt",
    },
    // Raster images, OCRed by the Vision engine. `jpg`/`jpeg` and `tif`/`tiff`
    // are one media type on two extensions each.
    SourceFormat {
        extension: "png",
        media_type: "image/png",
        magic: ContainerMagic::Png,
        engine: LocalEngineKind::Vision,
        format_label: "png",
    },
    SourceFormat {
        extension: "jpg",
        media_type: "image/jpeg",
        magic: ContainerMagic::Jpeg,
        engine: LocalEngineKind::Vision,
        format_label: "jpeg",
    },
    SourceFormat {
        extension: "jpeg",
        media_type: "image/jpeg",
        magic: ContainerMagic::Jpeg,
        engine: LocalEngineKind::Vision,
        format_label: "jpeg",
    },
    SourceFormat {
        extension: "webp",
        media_type: "image/webp",
        magic: ContainerMagic::Webp,
        engine: LocalEngineKind::Vision,
        format_label: "webp",
    },
    SourceFormat {
        extension: "tiff",
        media_type: "image/tiff",
        magic: ContainerMagic::Tiff,
        engine: LocalEngineKind::Vision,
        format_label: "tiff",
    },
    SourceFormat {
        extension: "tif",
        media_type: "image/tiff",
        magic: ContainerMagic::Tiff,
        engine: LocalEngineKind::Vision,
        format_label: "tiff",
    },
    SourceFormat {
        extension: "gif",
        media_type: "image/gif",
        magic: ContainerMagic::Gif,
        engine: LocalEngineKind::Vision,
        format_label: "gif",
    },
    SourceFormat {
        extension: "bmp",
        media_type: "image/bmp",
        magic: ContainerMagic::Bmp,
        engine: LocalEngineKind::Vision,
        format_label: "bmp",
    },
    // Audio and video containers, transcribed by the Audio engine. `mp4` and
    // `mov` are here for the audio track: the worker never looks at video.
    SourceFormat {
        extension: "wav",
        media_type: "audio/wav",
        magic: ContainerMagic::Wave,
        engine: LocalEngineKind::Audio,
        format_label: "wav",
    },
    SourceFormat {
        extension: "m4a",
        media_type: "audio/mp4",
        magic: ContainerMagic::IsoBmff,
        engine: LocalEngineKind::Audio,
        format_label: "m4a",
    },
    SourceFormat {
        extension: "mp4",
        media_type: "video/mp4",
        magic: ContainerMagic::IsoBmff,
        engine: LocalEngineKind::Audio,
        format_label: "mp4",
    },
    SourceFormat {
        extension: "mov",
        media_type: "video/quicktime",
        magic: ContainerMagic::IsoBmff,
        engine: LocalEngineKind::Audio,
        format_label: "mov",
    },
    SourceFormat {
        extension: "mp3",
        media_type: "audio/mpeg",
        magic: ContainerMagic::MpegAudio,
        engine: LocalEngineKind::Audio,
        format_label: "mp3",
    },
    SourceFormat {
        extension: "flac",
        media_type: "audio/flac",
        magic: ContainerMagic::Flac,
        engine: LocalEngineKind::Audio,
        format_label: "flac",
    },
];

pub(crate) fn source_format_by_extension(extension: &str) -> Option<&'static SourceFormat> {
    SOURCE_FORMATS
        .iter()
        .find(|format| format.extension.eq_ignore_ascii_case(extension))
}

pub(crate) fn source_format_by_media_type(media_type: &str) -> Option<&'static SourceFormat> {
    SOURCE_FORMATS
        .iter()
        .find(|format| format.media_type == media_type)
}

/// The media types the service advertises and accepts, in table order.
/// Deduplicated: `ppt`, `pps`, and `pot` are one media type on three
/// extensions.
pub(crate) fn advertised_media_types() -> Vec<&'static str> {
    let mut seen = Vec::new();
    for format in SOURCE_FORMATS {
        if !seen.contains(&format.media_type) {
            seen.push(format.media_type);
        }
    }
    seen
}

/// The media types this process can actually convert.
///
/// The contract is one list and a deployment is another. Vision and Audio are
/// Swift workers that ship only on macOS 26 and up, so a Linux container knows
/// every image and audio format and can convert none of them. Advertising what
/// the host cannot serve turns a clean refusal at admission into a job that
/// never runs.
pub(crate) fn servable_media_types(available: EngineAvailability) -> Vec<&'static str> {
    let mut servable = advertised_media_types();
    servable.retain(|media_type| {
        SOURCE_FORMATS
            .iter()
            .any(|format| format.media_type == *media_type && format.is_servable(available))
    });
    servable
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub id: Uuid,
    pub active_attempt_id: Uuid,
    pub client_run_id: Uuid,
    pub profile: ConversionProfile,
    pub status: JobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<RouteView>,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<JobFailure>,
    pub created_at: String,
    pub updated_at: String,
}

impl JobView {
    pub fn from_stored(job: &StoredConversion) -> Self {
        Self {
            id: job.id,
            active_attempt_id: job.active_attempt.id,
            client_run_id: job.client_run_id,
            profile: job.profile,
            status: job.state,
            route: job.route.as_ref().map(|kind| RouteView {
                kind: kind.clone(),
                reason_codes: job.reason_codes.clone(),
            }),
            warnings: job.warnings.clone(),
            failure: job.failure.as_ref().map(|failure| JobFailure {
                code: failure.code.clone(),
                message: failure.message.clone(),
            }),
            created_at: job.created_at.clone(),
            updated_at: job.updated_at.clone(),
        }
    }

    pub fn artifact_integrity_failed(job: &StoredConversion) -> Self {
        let mut view = Self::from_stored(job);
        view.status = JobStatus::Failed;
        view.failure = Some(JobFailure {
            code: ARTIFACT_INTEGRITY_CODE.to_owned(),
            message: ARTIFACT_INTEGRITY_MESSAGE.to_owned(),
        });
        view.updated_at = now();
        view
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteView {
    pub kind: String,
    pub reason_codes: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct JobFailure {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceMetadata {
    pub media_type: String,
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Debug)]
pub struct ArtifactRecord {
    pub file: File,
    pub media_type: String,
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactView {
    pub kind: ArtifactKind,
    pub attempt_id: Uuid,
    pub media_type: String,
    pub byte_length: u64,
    pub sha256: String,
    pub href: String,
}

impl ArtifactView {
    pub fn from_stored(job_id: Uuid, artifact: &StoredArtifact) -> Self {
        let kind = artifact.kind;
        Self {
            kind,
            attempt_id: artifact.attempt_id,
            media_type: artifact.media_type.clone(),
            byte_length: artifact.byte_length,
            sha256: artifact.sha256.clone(),
            href: format!("/api/v1/conversions/{job_id}/artifacts/{}", kind.as_str()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ConversionManifest {
    pub(crate) schema_version: u32,
    pub(crate) job_id: Uuid,
    pub(crate) attempt_id: Uuid,
    pub(crate) client_run_id: Uuid,
    pub(crate) profile: ConversionProfile,
    pub(crate) source: ManifestSource,
    pub(crate) engine: ManifestEngine,
    pub(crate) route: ManifestRoute,
    pub(crate) document: Value,
    pub(crate) warnings: Vec<String>,
    pub(crate) output: ManifestSource,
    pub(crate) started_at: String,
    pub(crate) completed_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ManifestSource {
    pub(crate) media_type: String,
    pub(crate) byte_length: u64,
    pub(crate) sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManifestEngine {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) features: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ManifestRoute {
    pub(crate) kind: String,
    pub(crate) reason_codes: Vec<String>,
}

pub fn now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMAGE_MEDIA_TYPES: [&str; 6] = [
        "image/png",
        "image/jpeg",
        "image/webp",
        "image/tiff",
        "image/gif",
        "image/bmp",
    ];

    const AUDIO_MEDIA_TYPES: [&str; 6] = [
        "audio/wav",
        "audio/mp4",
        "video/mp4",
        "video/quicktime",
        "audio/mpeg",
        "audio/flac",
    ];

    fn sorted(media_types: impl IntoIterator<Item = &'static str>) -> Vec<&'static str> {
        let mut media_types: Vec<&'static str> = media_types.into_iter().collect();
        media_types.sort_unstable();
        media_types
    }

    /// The contract never shrinks and a deployment often is smaller. Every
    /// availability shape is pinned to an exact set, because 18 documents plus
    /// 6 images and 18 plus 6 audio types are both 24: a count cannot tell a
    /// Vision-only host from an Audio-only one.
    #[test]
    fn servable_media_types_follow_the_engines_this_process_started() {
        let documents: Vec<&'static str> = advertised_media_types()
            .into_iter()
            .filter(|media_type| {
                !IMAGE_MEDIA_TYPES.contains(media_type) && !AUDIO_MEDIA_TYPES.contains(media_type)
            })
            .collect();

        for (vision, audio, expected) in [
            (false, false, sorted(documents.clone())),
            (
                true,
                false,
                sorted(documents.iter().copied().chain(IMAGE_MEDIA_TYPES)),
            ),
            (
                false,
                true,
                sorted(documents.iter().copied().chain(AUDIO_MEDIA_TYPES)),
            ),
            (true, true, sorted(advertised_media_types())),
        ] {
            let available = EngineAvailability { vision, audio };
            assert_eq!(
                sorted(servable_media_types(available)),
                expected,
                "vision={vision} audio={audio}"
            );
        }

        // Every engine up serves the whole contract, in table order.
        let all = EngineAvailability {
            vision: true,
            audio: true,
        };
        assert_eq!(servable_media_types(all), advertised_media_types());
    }

    /// The admission table and the database CHECK are two copies of one list.
    /// If they drift, an upload the API accepts fails at INSERT with a
    /// constraint error instead of a clean 415, so pin them together.
    #[test]
    fn advertised_media_types_match_the_migration_check() {
        let sql = include_str!("../../migrations/0005_audio_source_formats.sql");
        let check = sql
            .split_once("source_media_type     TEXT NOT NULL")
            .expect("the conversions CHECK must exist")
            .1
            .split_once("))")
            .expect("the CHECK must close")
            .0;
        for media_type in advertised_media_types() {
            assert!(
                check.contains(&format!("'{media_type}'")),
                "{media_type} is advertised but the migration CHECK rejects it"
            );
        }
        let quoted = check.matches('\'').count() / 2;
        assert_eq!(
            quoted,
            advertised_media_types().len(),
            "the migration CHECK lists media types the admission table does not"
        );
    }

    /// The upload contract and the admission table are the same list. When
    /// they drifted, OpenAPI advertised 8 media types while the service
    /// accepted 18.
    #[test]
    fn advertised_media_types_match_the_openapi_upload_contract() {
        let spec = include_str!("../../../../contract/http/openapi.yaml");
        let declared = spec
            .split_once(
                "            encoding:\n              source:\n                contentType: ",
            )
            .expect("the multipart source encoding must exist")
            .1
            .split_once('\n')
            .expect("the contentType line must end")
            .0;
        let declared: Vec<&str> = declared.split(", ").map(str::trim).collect();
        assert_eq!(declared, advertised_media_types());
    }

    #[test]
    fn every_extension_is_unique_and_lowercase() {
        let mut seen = Vec::new();
        for format in SOURCE_FORMATS {
            assert_eq!(format.extension, format.extension.to_ascii_lowercase());
            assert!(
                !seen.contains(&format.extension),
                "duplicate extension {}",
                format.extension
            );
            seen.push(format.extension);
        }
    }
}
