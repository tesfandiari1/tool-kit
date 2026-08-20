use std::{fmt::Debug, future::Future, time::Duration};

use axum::{extract::State, http::StatusCode, Json};
use serde::Serialize;
use tokio::time::timeout;

use crate::AppState;

const SERVICE_NAME: &str = "tool-kit-converter";
const CHECK_OK: &str = "ok";
const CHECK_FAILED: &str = "failed";
/// Each probe answers inside the container healthcheck timeout, so a stuck
/// dependency reports `not_ready` instead of hanging the endpoint.
const CHECK_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    status: &'static str,
    service: &'static str,
    service_version: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessResponse {
    status: &'static str,
    service: &'static str,
    service_version: &'static str,
    checks: ReadinessChecks,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessChecks {
    database: &'static str,
    data_root: &'static str,
    worker: &'static str,
}

pub async fn live() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: SERVICE_NAME,
        service_version: env!("CARGO_PKG_VERSION"),
    })
}

pub async fn ready(State(state): State<AppState>) -> (StatusCode, Json<ReadinessResponse>) {
    let (database, data_root, worker) = tokio::join!(
        bounded("database", state.service().health_check()),
        bounded("dataRoot", state.service().probe_data_root()),
        bounded("worker", std::future::ready(worker_health(&state))),
    );

    let ready = database && data_root && worker;
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(ReadinessResponse {
            status: if ready { "ready" } else { "not_ready" },
            service: SERVICE_NAME,
            service_version: env!("CARGO_PKG_VERSION"),
            checks: ReadinessChecks {
                database: label(database),
                data_root: label(data_root),
                worker: label(worker),
            },
        }),
    )
}

fn worker_health(state: &AppState) -> Result<(), &'static str> {
    if state.job_runner_failed() {
        return Err("the job runner exited unexpectedly");
    }
    if state.job_runner_stopped() {
        return Err("the job runner stopped claiming work");
    }
    Ok(())
}

/// Reports pass or fail only. The endpoint is unauthenticated, so the reason
/// goes to the log rather than the response body.
async fn bounded<T, E>(check: &'static str, probe: impl Future<Output = Result<T, E>>) -> bool
where
    E: Debug,
{
    match timeout(CHECK_TIMEOUT, probe).await {
        Ok(Ok(_)) => true,
        Ok(Err(error)) => {
            // Debug, not Display: every probe error wraps its cause as a
            // `#[source]` that Display drops, and the cause is the diagnosis.
            tracing::warn!(check, ?error, "readiness check failed");
            false
        }
        Err(_) => {
            tracing::warn!(
                check,
                timeout_secs = CHECK_TIMEOUT.as_secs(),
                "readiness check timed out"
            );
            false
        }
    }
}

fn label(passed: bool) -> &'static str {
    if passed {
        CHECK_OK
    } else {
        CHECK_FAILED
    }
}
