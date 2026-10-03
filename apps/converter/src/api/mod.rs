mod capabilities;
mod conversions;
mod health;

use std::time::Instant;

use axum::{
    extract::{DefaultBodyLimit, Extension, Request, State},
    http::{HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use tracing::Instrument;
use uuid::Uuid;

use crate::{
    auth::BootstrapAuth,
    error::{ApiError, RequestId},
    AppState,
};

const REQUEST_ID_HEADER: &str = "x-request-id";
const MULTIPART_OVERHEAD_BYTES: u64 = 1024 * 1024;

pub fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/api/v1/conversions", post(conversions::create))
        .route("/api/v1/conversions/{id}", get(conversions::get))
        .route(
            "/api/v1/conversions/{id}/artifacts/markdown",
            get(conversions::download_markdown),
        )
        .route_layer(middleware::from_fn_with_state(
            state.auth().clone(),
            require_auth,
        ));
    // The widest ceiling any format can claim: audio carries its own, and a
    // layer sized to the document limit would answer every recording with 413
    // before admission ever read the extension.
    let body_limit = usize::try_from(
        state
            .limits()
            .max_upload_bytes
            .max(state.limits().max_audio_upload_bytes)
            .saturating_add(MULTIPART_OVERHEAD_BYTES),
    )
    .expect("configured body limit must fit usize");

    Router::new()
        .route("/health/live", get(health::live))
        .route("/health/ready", get(health::ready))
        .route("/api/v1/capabilities", get(capabilities::get))
        .merge(protected)
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .with_state(state)
        .layer(DefaultBodyLimit::max(body_limit))
        .layer(middleware::from_fn(trace_request))
}

async fn require_auth(State(auth): State<BootstrapAuth>, request: Request, next: Next) -> Response {
    if auth.authorizes(request.headers()) {
        return next.run(request).await;
    }
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .cloned()
        .unwrap_or_else(|| RequestId::new(Uuid::new_v4().to_string()));
    ApiError::unauthorized(request_id).into_response()
}

async fn not_found(Extension(request_id): Extension<RequestId>) -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "route_not_found",
        "The requested route does not exist.",
        request_id,
    )
}

async fn method_not_allowed(Extension(request_id): Extension<RequestId>) -> ApiError {
    ApiError::new(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "The HTTP method is not allowed for this route.",
        request_id,
    )
}

async fn trace_request(mut request: Request, next: Next) -> Response {
    let request_id = Uuid::new_v4().to_string();
    let request_id_header = HeaderValue::from_str(&request_id)
        .expect("a UUID must always be a valid HTTP header value");
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let started = Instant::now();

    request
        .headers_mut()
        .insert(REQUEST_ID_HEADER, request_id_header.clone());
    request
        .extensions_mut()
        .insert(RequestId::new(request_id.clone()));

    let span = tracing::info_span!(
        "http.request",
        request_id = %request_id,
        method = %method,
        path = %path,
    );
    let mut response = next.run(request).instrument(span).await;

    response
        .headers_mut()
        .insert(REQUEST_ID_HEADER, request_id_header);
    tracing::info!(
        request_id = %request_id,
        method = %method,
        path = %path,
        status = response.status().as_u16(),
        elapsed_ms = started.elapsed().as_millis(),
        "http request completed"
    );

    response
}
