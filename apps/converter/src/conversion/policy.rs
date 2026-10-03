//! Routing and quality policy (M4, CVR-041/042/044).
//!
//! Pure maps from an engine and its measurements to the published strings.
//! No IO and no clock.
//! Everything the policy reads is a measurement an engine reported, which is
//! what CVR-041 means by "from real engine output": no filename heuristics, no
//! page-count rules, no guessing from the media type.

use crate::engines::QualitySignals;
use crate::worker_protocol::FallbackReason;

use super::model::LocalEngineKind;

impl LocalEngineKind {
    /// The route a job converted by this engine records. Every route runs on
    /// this machine, and the shared prefix says so.
    pub(crate) fn route_str(self) -> &'static str {
        match self {
            Self::Pdf => "local_pdf",
            Self::AnyDoc => "local_anydoc",
            Self::Vision => "local_vision",
            Self::Audio => "local_audio",
        }
    }

    /// The reason code a conversion this engine published carries.
    pub(crate) fn reason(self) -> ReasonCode {
        match self {
            Self::Pdf => ReasonCode::NativeTextPdf,
            Self::AnyDoc => ReasonCode::StructuredDocument,
            Self::Vision => ReasonCode::RecognizedImageText,
            Self::Audio => ReasonCode::TranscribedAudio,
        }
    }
}

/// Why a conversion routed the way it did. Closed vocabulary: the desktop
/// renders these verbatim, so a new one is a contract change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReasonCode {
    /// The whole PDF parsed as native text.
    NativeTextPdf,
    /// A non-PDF document parsed by AnyDoc.
    StructuredDocument,
    /// Text read off an image by OCR. Neither of the codes above is true of a
    /// photograph or a screenshot, and the difference matters to a reader
    /// judging the output.
    RecognizedImageText,
    /// Speech read off an audio or video track. The Markdown is a transcript,
    /// not a rendering of anything that was written down.
    TranscribedAudio,
    /// The engine gave up before producing Markdown. Carries the engine's own
    /// reason.
    Engine(FallbackReason),
}

impl ReasonCode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NativeTextPdf => "native_text_pdf",
            Self::StructuredDocument => "structured_document",
            Self::RecognizedImageText => "recognized_image_text",
            Self::TranscribedAudio => "transcribed_audio",
            Self::Engine(reason) => reason.as_str(),
        }
    }
}

/// A caveat about output that was still published. Warnings never change the
/// route; they exist so a degraded success stops being a silent one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Warning {
    /// Some pages produced no extractable text. That is all this means: the
    /// engine cannot say whether those pages were blank, or a scan whose
    /// content is now missing from the Markdown.
    PagesWithoutExtractableText,
    /// The layout has tables. Local table reconstruction is weaker than the
    /// remote route's.
    DenseTables,
    /// The layout has multiple columns; reading order may be imperfect.
    MultiColumnLayout,
}

impl Warning {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::PagesWithoutExtractableText => "pages_without_extractable_text",
            Self::DenseTables => "dense_tables",
            Self::MultiColumnLayout => "multi_column_layout",
        }
    }
}

/// The warnings published Markdown carries. An engine that gave up publishes
/// nothing, so it gets none.
///
/// **Profile changes none of this, on purpose.** The only signal for routing a
/// partly-textless document remote, `QualitySignals::native_text_ratio`, is
/// wrong in both directions: a report with a one-line cover page reports 0.9
/// with nothing missing, and a page holding text and a full-page scan reports
/// 1.0 with the scan lost. Warning is what the signal supports. Routing needs
/// pages that reference an image XObject but yielded no text, which is a
/// worker protocol change.
///
/// **An engine that measures nothing publishes with no warning, on purpose.**
/// AnyDoc cannot report completeness, and its part-level recovery is silent.
/// Warning on all 19 AnyDoc formats would teach users to ignore warnings, so
/// `structured_document` claims a parser ran, not that the output is complete.
pub(crate) fn warnings(signals: QualitySignals) -> Vec<String> {
    let mut warnings = Vec::new();
    if signals.has_pages_without_text() {
        warnings.push(Warning::PagesWithoutExtractableText);
    }
    if signals.has_tables {
        warnings.push(Warning::DenseTables);
    }
    if signals.has_columns {
        warnings.push(Warning::MultiColumnLayout);
    }
    warnings
        .into_iter()
        .map(|warning| warning.as_str().to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native() -> QualitySignals {
        QualitySignals {
            native_text_ratio: Some(1.0),
            has_tables: false,
            has_columns: false,
        }
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn each_engine_records_its_own_route_and_reason() {
        for (engine, route, reason) in [
            (LocalEngineKind::Pdf, "local_pdf", "native_text_pdf"),
            (LocalEngineKind::AnyDoc, "local_anydoc", "structured_document"),
            (LocalEngineKind::Vision, "local_vision", "recognized_image_text"),
            (LocalEngineKind::Audio, "local_audio", "transcribed_audio"),
        ] {
            assert_eq!(engine.route_str(), route);
            assert_eq!(engine.reason().as_str(), reason);
        }
    }

    #[test]
    fn a_fully_native_document_publishes_with_no_warnings() {
        assert!(warnings(native()).is_empty());
    }

    /// A partly-textless document must never publish *silently*. Nor may this
    /// signal alone route it remote: it fires on documents that lost nothing.
    #[test]
    fn a_document_with_textless_pages_publishes_with_a_warning_not_a_bill() {
        for ratio in [0.9_f32, 0.8, 0.75, 0.6666667, 0.5] {
            let signals = QualitySignals {
                native_text_ratio: Some(ratio),
                ..native()
            };
            assert_eq!(
                warnings(signals),
                strings(&["pages_without_extractable_text"]),
                "publishing without the warning is the silent success CVR-045 forbids"
            );
        }
    }

    #[test]
    fn an_engine_that_gave_up_keeps_its_reason() {
        for reason in [
            FallbackReason::ScannedPdf,
            FallbackReason::ImageBasedPdf,
            FallbackReason::MixedPdf,
            FallbackReason::GarbledText,
            FallbackReason::OcrRequired,
            FallbackReason::LocalQualityFailed,
            FallbackReason::OutputTooLarge,
        ] {
            assert_eq!(ReasonCode::Engine(reason).as_str(), reason.as_str());
        }
    }

    #[test]
    fn layout_complexity_warns_without_changing_the_route() {
        let signals = QualitySignals {
            has_tables: true,
            has_columns: true,
            ..native()
        };
        assert_eq!(
            warnings(signals),
            strings(&["dense_tables", "multi_column_layout"])
        );
    }

    /// AnyDoc measures nothing, so it must not be treated as having measured
    /// and found the document complete.
    #[test]
    fn an_unmeasured_engine_publishes_without_claiming_completeness() {
        assert!(warnings(QualitySignals::unmeasured()).is_empty());
        assert!(!QualitySignals::unmeasured().has_pages_without_text());
    }

    /// Every published vocabulary string is snake_case and stable. The desktop
    /// renders them verbatim, so a rename is a contract change.
    #[test]
    fn the_published_vocabulary_is_stable() {
        for code in [
            ReasonCode::NativeTextPdf,
            ReasonCode::StructuredDocument,
            ReasonCode::RecognizedImageText,
            ReasonCode::TranscribedAudio,
            ReasonCode::Engine(FallbackReason::ScannedPdf),
        ] {
            let value = code.as_str();
            assert!(!value.is_empty());
            assert!(value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_'));
        }
        for warning in [
            Warning::PagesWithoutExtractableText,
            Warning::DenseTables,
            Warning::MultiColumnLayout,
        ] {
            assert!(warning
                .as_str()
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_'));
        }
    }

    /// The eval corpus manifest is prose about behaviour, so it drifts: the
    /// file this one replaced expected routes and outcomes the service has
    /// never been able to emit (`datalab`, `policy_calibrated`, `reject`).
    /// Every string in it is checked against the code that produces it.
    #[test]
    fn the_eval_corpus_manifest_uses_only_strings_the_service_can_emit() {
        use crate::conversion::{ConversionProfile, JobStatus};
        use crate::engines::EngineFailure;
        use crate::worker_protocol::RejectionCode;

        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Manifest {
            schema_version: u32,
            cases: Vec<Case>,
        }

        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Case {
            id: String,
            generator: String,
            generator_args: Vec<u32>,
            profile: String,
            expected_status: String,
            expected_route: String,
            expected_reason_codes: Vec<String>,
            expected_warnings: Vec<String>,
            expected_artifact: bool,
            #[serde(default)]
            expected_failure_code: Option<String>,
        }

        let manifest: Manifest =
            serde_yaml_ng::from_str(include_str!("../../evals/corpus-manifest.yaml"))
                .expect("the corpus manifest must parse");
        assert_eq!(manifest.schema_version, 2);
        assert!(!manifest.cases.is_empty());

        let generators = include_str!("../../tests/support/corpus.rs");
        let status_string = |status: JobStatus| {
            serde_json::to_value(status)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .expect("JobStatus serializes as a string")
        };
        let statuses: Vec<String> = [
            JobStatus::Queued,
            JobStatus::ConvertingLocal,
            JobStatus::Finalizing,
            JobStatus::Succeeded,
            JobStatus::Failed,
            JobStatus::NeedsRemote,
        ]
        .into_iter()
        .map(status_string)
        .collect();
        let profiles = [
            ConversionProfile::Standard.as_str(),
            ConversionProfile::LocalOnly.as_str(),
            ConversionProfile::BestQuality.as_str(),
        ];
        let routes = [
            LocalEngineKind::Pdf.route_str(),
            LocalEngineKind::AnyDoc.route_str(),
            LocalEngineKind::Vision.route_str(),
        ];
        let reasons = [
            ReasonCode::NativeTextPdf,
            ReasonCode::StructuredDocument,
            ReasonCode::RecognizedImageText,
            ReasonCode::Engine(FallbackReason::ScannedPdf),
            ReasonCode::Engine(FallbackReason::ImageBasedPdf),
            ReasonCode::Engine(FallbackReason::MixedPdf),
            ReasonCode::Engine(FallbackReason::GarbledText),
            ReasonCode::Engine(FallbackReason::OcrRequired),
            ReasonCode::Engine(FallbackReason::LocalQualityFailed),
            ReasonCode::Engine(FallbackReason::OutputTooLarge),
        ]
        .map(ReasonCode::as_str);
        let warnings = [
            Warning::PagesWithoutExtractableText,
            Warning::DenseTables,
            Warning::MultiColumnLayout,
        ]
        .map(Warning::as_str);
        // The corpus is PDF-only, so a case fails either as a worker rejection
        // or as a local engine failure.
        let failure_codes = [
            RejectionCode::EncryptedPdf.as_str(),
            RejectionCode::InvalidPdf.as_str(),
            RejectionCode::InvalidPdfStructure.as_str(),
            EngineFailure::Unavailable.code(),
            EngineFailure::Timeout.code(),
            EngineFailure::Interrupted.code(),
            EngineFailure::Crashed.code(),
            EngineFailure::Protocol.code(),
        ];

        let mut seen: Vec<&str> = Vec::new();
        for case in &manifest.cases {
            let id = case.id.as_str();
            assert!(!seen.contains(&id), "duplicate case id {id}");
            seen.push(id);
            assert!(
                generators.contains(&format!("fn {}(", case.generator)),
                "{id} names generator {}, which the corpus does not define",
                case.generator
            );
            assert!(
                case.generator_args.iter().all(|pages| *pages > 0),
                "{id} asks for a zero-page PDF"
            );
            assert!(
                profiles.contains(&case.profile.as_str()),
                "{id} uses profile {}",
                case.profile
            );
            assert!(
                statuses.contains(&case.expected_status),
                "{id} expects status {}",
                case.expected_status
            );
            assert!(
                routes.contains(&case.expected_route.as_str()),
                "{id} expects route {}",
                case.expected_route
            );
            for code in &case.expected_reason_codes {
                assert!(
                    reasons.contains(&code.as_str()),
                    "{id} expects reason code {code}"
                );
            }
            for warning in &case.expected_warnings {
                assert!(
                    warnings.contains(&warning.as_str()),
                    "{id} expects warning {warning}"
                );
            }
            assert_eq!(
                case.expected_artifact,
                case.expected_status == status_string(JobStatus::Succeeded),
                "{id}: Markdown is published by a succeeded job and by nothing else"
            );
            assert_eq!(
                case.expected_failure_code.is_some(),
                case.expected_status == status_string(JobStatus::Failed),
                "{id}: a failure code and a failed status travel together"
            );
            if let Some(code) = &case.expected_failure_code {
                assert!(
                    failure_codes.contains(&code.as_str()),
                    "{id} expects failure code {code}"
                );
            }
        }
    }
}
