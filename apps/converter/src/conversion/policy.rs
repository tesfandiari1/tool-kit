//! Routing and quality policy (M4, CVR-041/042/044).
//!
//! One pure function over one table. No IO, no clock, no engine knowledge.
//! Everything the policy reads is a measurement an engine reported, which is
//! what CVR-041 means by "from real engine output": no filename heuristics, no
//! page-count rules, no guessing from the media type.

use crate::engines::QualitySignals;
use crate::worker_protocol::FallbackReason;

use super::ConversionProfile;

/// Where a conversion's Markdown comes from.
///
/// Every route runs on this machine, and the shared prefix says so: the remote
/// leg is a separate contract, not another variant here.
#[expect(clippy::enum_variant_names)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RouteKind {
    LocalPdf,
    LocalAnyDoc,
    LocalVision,
    LocalAudio,
}

impl RouteKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::LocalPdf => "local_pdf",
            Self::LocalAnyDoc => "local_anydoc",
            Self::LocalVision => "local_vision",
            Self::LocalAudio => "local_audio",
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

/// What the local engine came back with. The policy never sees an engine type.
#[derive(Clone, Copy, Debug)]
pub(crate) enum LocalResult {
    /// Markdown exists on disk, with these measurements.
    Converted(QualitySignals),
    /// No Markdown. The engine named why.
    GaveUp(FallbackReason),
}

/// What to do with the attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PolicyDecision {
    /// Publish the local Markdown.
    Publish {
        reason_codes: Vec<ReasonCode>,
        warnings: Vec<Warning>,
    },
    /// Do not publish. The job ends `needs_remote`, and whoever owns the remote
    /// leg decides from there.
    ///
    /// One reason code, never zero, so the durable `fallback_reason` cannot be
    /// written empty. Warnings ride with published bytes, and there are none.
    NeedsRemote { reason_code: ReasonCode },
}

impl PolicyDecision {
    pub(crate) fn reason_strings(&self) -> Vec<String> {
        match self {
            Self::Publish { reason_codes, .. } => reason_codes
                .iter()
                .map(|code| code.as_str().to_owned())
                .collect(),
            Self::NeedsRemote { reason_code } => vec![reason_code.as_str().to_owned()],
        }
    }

    pub(crate) fn warning_strings(&self) -> Vec<String> {
        match self {
            Self::Publish { warnings, .. } => warnings
                .iter()
                .map(|warning| warning.as_str().to_owned())
                .collect(),
            Self::NeedsRemote { .. } => Vec::new(),
        }
    }
}

/// The whole policy.
///
/// Read it as a table, top to bottom, first match wins:
///
/// | local result | decision |
/// |---|---|
/// | gave up | `needs_remote`, engine's reason, no warnings |
/// | converted, some pages had no text | publish + `pages_without_extractable_text` |
/// | converted, complex layout | publish + layout warnings |
/// | converted | publish |
///
/// **Profile does not change any of this today**, and that is deliberate.
///
/// The obvious profile split would be "route a partly-textless document remote
/// under `standard`". It is not implemented because the only signal for it,
/// `QualitySignals::native_text_ratio`, is wrong in both directions: a report
/// with a one-line cover page and no images reports 0.9 with nothing missing,
/// and a page holding both text and a full-page scan reports 1.0 with the scan
/// lost. Routing on it would bill Datalab for ordinary documents and still
/// miss real losses. Warning is what the signal actually supports.
///
/// Getting the split back needs evidence the engine does not report yet:
/// pages that reference an image XObject but yielded no text. That is a worker
/// protocol change, not a policy change.
///
/// `best_quality` is likewise unrepresented. It means "always prefer remote",
/// which needs a remote leg to exist, so the API rejects it until M5.
///
/// **An engine that measures nothing publishes with no warning, on purpose.**
/// AnyDoc cannot report completeness, and its own part-level "skip a broken
/// piece and continue" recovery is silent, so a degraded AnyDoc conversion is
/// indistinguishable from a clean one here. Warning on all 19 AnyDoc formats
/// would attach a caveat to almost every non-PDF conversion and teach users to
/// ignore warnings, which costs more than it buys. The honest statement is
/// that `structured_document` claims a parser ran, not that the output is
/// complete. Revisit if AnyDoc ever reports what it skipped.
pub(crate) fn decide(
    // Unused today, kept because M5's remote leg is the first thing that reads
    // it. See the note above on why the obvious profile split is not here yet.
    _profile: ConversionProfile,
    route: RouteKind,
    local: LocalResult,
) -> PolicyDecision {
    let signals = match local {
        LocalResult::GaveUp(reason) => {
            return PolicyDecision::NeedsRemote {
                reason_code: ReasonCode::Engine(reason),
            };
        }
        LocalResult::Converted(signals) => signals,
    };

    // Warnings only ever accompany published bytes, so they are built after
    // every branch that publishes nothing.
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

    PolicyDecision::Publish {
        reason_codes: vec![converted_reason(route)],
        warnings,
    }
}

fn converted_reason(route: RouteKind) -> ReasonCode {
    match route {
        RouteKind::LocalPdf => ReasonCode::NativeTextPdf,
        RouteKind::LocalAnyDoc => ReasonCode::StructuredDocument,
        RouteKind::LocalVision => ReasonCode::RecognizedImageText,
        RouteKind::LocalAudio => ReasonCode::TranscribedAudio,
    }
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

    fn strings(decision: &PolicyDecision) -> (Vec<String>, Vec<String>) {
        (decision.reason_strings(), decision.warning_strings())
    }

    #[test]
    fn a_fully_native_document_publishes_with_no_warnings() {
        for profile in [
            ConversionProfile::Standard,
            ConversionProfile::LocalOnly,
            ConversionProfile::BestQuality,
        ] {
            let decision = decide(
                profile,
                RouteKind::LocalPdf,
                LocalResult::Converted(native()),
            );
            assert_eq!(
                strings(&decision),
                (vec!["native_text_pdf".to_owned()], Vec::new())
            );
            assert!(matches!(decision, PolicyDecision::Publish { .. }));
        }
    }

    /// The defect this milestone exists for, and the correction the review
    /// forced. A partly-textless document must never publish *silently*. It
    /// must also never be routed remote on this signal alone, because the
    /// signal fires on ordinary documents that lost nothing.
    #[test]
    fn a_document_with_textless_pages_publishes_with_a_warning_not_a_bill() {
        for ratio in [0.9_f32, 0.8, 0.75, 0.6666667, 0.5] {
            let signals = QualitySignals {
                native_text_ratio: Some(ratio),
                ..native()
            };
            for profile in [
                ConversionProfile::Standard,
                ConversionProfile::LocalOnly,
                ConversionProfile::BestQuality,
            ] {
                let decision = decide(
                    profile,
                    RouteKind::LocalPdf,
                    LocalResult::Converted(signals),
                );
                let PolicyDecision::Publish { warnings, .. } = &decision else {
                    panic!(
                        "ratio {ratio} under {profile:?} must publish: a cover page and a scanned \
                         page are indistinguishable here, so routing remote bills for documents \
                         that lost nothing"
                    );
                };
                assert!(
                    warnings.contains(&Warning::PagesWithoutExtractableText),
                    "publishing without the warning is the silent success CVR-045 forbids"
                );
            }
        }
    }

    #[test]
    fn an_engine_that_gave_up_always_needs_remote_and_keeps_its_reason() {
        for reason in [
            FallbackReason::ScannedPdf,
            FallbackReason::ImageBasedPdf,
            FallbackReason::MixedPdf,
            FallbackReason::GarbledText,
            FallbackReason::OcrRequired,
            FallbackReason::LocalQualityFailed,
            FallbackReason::OutputTooLarge,
        ] {
            for profile in [ConversionProfile::Standard, ConversionProfile::LocalOnly] {
                let decision = decide(profile, RouteKind::LocalPdf, LocalResult::GaveUp(reason));
                assert_eq!(
                    decision,
                    PolicyDecision::NeedsRemote {
                        reason_code: ReasonCode::Engine(reason),
                    }
                );
            }
        }
    }

    #[test]
    fn layout_complexity_warns_without_changing_the_route() {
        let decision = decide(
            ConversionProfile::Standard,
            RouteKind::LocalPdf,
            LocalResult::Converted(QualitySignals {
                has_tables: true,
                has_columns: true,
                ..native()
            }),
        );
        assert_eq!(
            strings(&decision),
            (
                vec!["native_text_pdf".to_owned()],
                vec!["dense_tables".to_owned(), "multi_column_layout".to_owned()],
            )
        );
        assert!(matches!(decision, PolicyDecision::Publish { .. }));
    }

    /// AnyDoc measures nothing, so it must not be treated as having measured
    /// and found the document complete.
    #[test]
    fn an_unmeasured_engine_publishes_without_claiming_completeness() {
        let decision = decide(
            ConversionProfile::Standard,
            RouteKind::LocalAnyDoc,
            LocalResult::Converted(QualitySignals::unmeasured()),
        );
        assert_eq!(
            strings(&decision),
            (vec!["structured_document".to_owned()], Vec::new())
        );
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
        use crate::conversion::JobStatus;
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
            RouteKind::LocalPdf.as_str(),
            RouteKind::LocalAnyDoc.as_str(),
            RouteKind::LocalVision.as_str(),
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
