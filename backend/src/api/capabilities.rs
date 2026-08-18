use axum::{extract::State, Json};
use serde::Serialize;

use crate::{worker_protocol::PDF_INSPECTOR_VERSION, AppState};

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
    engine: EngineCapability,
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
    version: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LimitCapabilities {
    max_upload_bytes: u64,
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
    Json(CapabilitiesEnvelope {
        data: Capabilities {
            api_version: "v1",
            service_version: env!("CARGO_PKG_VERSION"),
            conversion: ConversionCapabilities {
                accepting_jobs,
                durability: "persistent",
                input_formats: vec!["application/pdf"],
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
                engine: EngineCapability {
                    name: "pdf-inspector",
                    version: PDF_INSPECTOR_VERSION,
                },
                limits: LimitCapabilities {
                    max_upload_bytes: state.limits().max_upload_bytes,
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
