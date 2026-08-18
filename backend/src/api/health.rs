use axum::Json;
use serde::Serialize;

const SERVICE_NAME: &str = "tool-kit-converter";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    status: &'static str,
    service: &'static str,
    service_version: &'static str,
}

pub async fn live() -> Json<HealthResponse> {
    Json(response("ok"))
}

pub async fn ready() -> Json<HealthResponse> {
    Json(response("ready"))
}

fn response(status: &'static str) -> HealthResponse {
    HealthResponse {
        status,
        service: SERVICE_NAME,
        service_version: env!("CARGO_PKG_VERSION"),
    }
}
