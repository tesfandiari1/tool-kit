use axum::{
    http::{header::WWW_AUTHENTICATE, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestId(String);

impl RequestId {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    request_id: RequestId,
    bearer_challenge: bool,
}

impl ApiError {
    pub fn new(
        status: StatusCode,
        code: &'static str,
        message: &'static str,
        request_id: RequestId,
    ) -> Self {
        Self {
            status,
            code,
            message,
            request_id,
            bearer_challenge: false,
        }
    }

    pub fn unauthorized(request_id: RequestId) -> Self {
        let mut error = Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Authentication is required.",
            request_id,
        );
        error.bearer_challenge = true;
        error
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let payload = ErrorEnvelope {
            error: ErrorBody {
                code: self.code,
                message: self.message,
                request_id: self.request_id.as_str(),
                details: Vec::new(),
            },
        };
        let mut response = (self.status, Json(payload)).into_response();
        if self.bearer_challenge {
            response
                .headers_mut()
                .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        response
    }
}

#[derive(Debug, Serialize)]
struct ErrorEnvelope<'a> {
    error: ErrorBody<'a>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorBody<'a> {
    code: &'a str,
    message: &'a str,
    request_id: &'a str,
    details: Vec<ErrorDetail>,
}

#[derive(Debug, Serialize)]
struct ErrorDetail {}
