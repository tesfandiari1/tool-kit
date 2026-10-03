use axum::{extract::State, Json};
use serde::Serialize;

use crate::{conversion::servable_media_types, AppState};

#[derive(Debug, Serialize)]
pub struct CapabilitiesEnvelope {
    data: Capabilities,
}

#[derive(Debug, Serialize)]
struct Capabilities {
    conversion: ConversionCapabilities,
}

/// Only what the desktop reads: whether to send, and what it may send.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConversionCapabilities {
    accepting_jobs: bool,
    input_formats: Vec<&'static str>,
}

pub async fn get(State(state): State<AppState>) -> Json<CapabilitiesEnvelope> {
    Json(CapabilitiesEnvelope {
        data: Capabilities {
            conversion: ConversionCapabilities {
                accepting_jobs: state.service().accepting_jobs().await,
                input_formats: servable_media_types(state.service().engine_availability()),
            },
        },
    })
}
