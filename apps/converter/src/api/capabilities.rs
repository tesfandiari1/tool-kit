use axum::{extract::State, Json};
use serde::Serialize;

use crate::{
    audio_protocol::AUDIO_ENGINE_NAME,
    conversion::servable_media_types,
    engines::{ANYDOC_ENGINE_NAME, ANYDOC_VERSION},
    vision_protocol::VISION_ENGINE_NAME,
    worker_protocol::{PDF_ENGINE_NAME, PDF_INSPECTOR_VERSION},
    AppState,
};

#[derive(Debug, Serialize)]
pub struct CapabilitiesEnvelope {
    data: Capabilities,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Capabilities {
    api_version: &'static str,
    service_version: &'static str,
    conversion: ConversionCapabilities,
    remote_fallback: RemoteFallbackCapabilities,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConversionCapabilities {
    accepting_jobs: bool,
    durability: &'static str,
    input_formats: Vec<&'static str>,
    output_formats: Vec<&'static str>,
    profiles: Vec<ProfileCapability>,
    engines: Vec<EngineCapability>,
    limits: LimitCapabilities,
}

#[derive(Debug, Serialize)]
struct ProfileCapability {
    name: &'static str,
    available: bool,
}

#[derive(Debug, Serialize)]
struct EngineCapability {
    name: &'static str,
    /// Owned because Vision's version is the running macOS product version,
    /// which is known only once the worker handshakes.
    version: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LimitCapabilities {
    max_upload_bytes: u64,
    /// Audio is bounded on its own key, an order of magnitude above the
    /// document ceiling. One published limit would refuse recordings the
    /// service accepts.
    max_audio_upload_bytes: u64,
    max_output_bytes: u64,
    max_active_jobs: usize,
    max_concurrent_uploads: usize,
}

#[derive(Debug, Serialize)]
struct RemoteFallbackCapabilities {
    provider: &'static str,
    available: bool,
}

pub async fn get(State(state): State<AppState>) -> Json<CapabilitiesEnvelope> {
    let accepting_jobs = state.service().accepting_jobs().await;
    let vision_version = state.service().vision_version().map(str::to_owned);
    let audio_version = state.service().audio_version().map(str::to_owned);
    // Two engines everywhere, plus whichever Swift workers came up. The client
    // reads this to know what it can send, so an entry for an engine that is
    // not here would be an invitation to a job that cannot run.
    let mut engines = vec![
        EngineCapability {
            name: PDF_ENGINE_NAME,
            version: PDF_INSPECTOR_VERSION.to_owned(),
        },
        EngineCapability {
            name: ANYDOC_ENGINE_NAME,
            version: ANYDOC_VERSION.to_owned(),
        },
    ];
    if let Some(version) = vision_version {
        engines.push(EngineCapability {
            name: VISION_ENGINE_NAME,
            version,
        });
    }
    if let Some(version) = audio_version {
        engines.push(EngineCapability {
            name: AUDIO_ENGINE_NAME,
            version,
        });
    }
    Json(CapabilitiesEnvelope {
        data: Capabilities {
            api_version: "v1",
            service_version: env!("CARGO_PKG_VERSION"),
            conversion: ConversionCapabilities {
                accepting_jobs,
                durability: "persistent",
                input_formats: servable_media_types(state.service().engine_availability()),
                output_formats: vec!["text/markdown", "application/json"],
                profiles: vec![
                    ProfileCapability {
                        name: "standard",
                        available: true,
                    },
                    ProfileCapability {
                        name: "local_only",
                        available: true,
                    },
                    ProfileCapability {
                        name: "best_quality",
                        available: false,
                    },
                ],
                engines,
                limits: LimitCapabilities {
                    max_upload_bytes: state.limits().max_upload_bytes,
                    max_audio_upload_bytes: state.limits().max_audio_upload_bytes,
                    max_output_bytes: state.limits().max_output_bytes,
                    max_active_jobs: state.limits().max_jobs,
                    max_concurrent_uploads: state.limits().max_concurrent_uploads,
                },
            },
            remote_fallback: RemoteFallbackCapabilities {
                provider: "datalab",
                available: false,
            },
        },
    })
}
