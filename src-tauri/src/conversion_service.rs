//! Native HTTP boundary for the conversion service.
//!
//! The webview supplies only contract paths, ordinary request metadata, and a
//! desktop source path. This module owns the configured service URL, Keychain
//! token, file streaming, response bounds, and artifact writes.

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use reqwest::{multipart, Client, Method, Response, Url};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;
use tauri::AppHandle;
use tokio::io::AsyncWriteExt;

use crate::{secrets, settings};

const GENERIC_BODY_LIMIT: usize = 1024 * 1024;
/// Matches the backend's documented default output ceiling. The service may
/// advertise less at runtime; later jobs integration can apply that tighter
/// capability before reaching this host safety boundary.
const MARKDOWN_BODY_LIMIT: usize = 50 * 1024 * 1024;
const SMALL_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const STREAM_REQUEST_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_OUTPUT_COLLISIONS: u32 = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversionCapabilities {
    pub(crate) accepting_jobs: bool,
    pub(crate) input_formats: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConversionRoute {
    pub(crate) kind: String,
    pub(crate) reason_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConversionFailure {
    pub(crate) code: String,
    pub(crate) message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConversionJob {
    pub(crate) id: String,
    pub(crate) status: String,
    pub(crate) route: Option<ConversionRoute>,
    #[serde(default)]
    pub(crate) warnings: Vec<String>,
    pub(crate) failure: Option<ConversionFailure>,
}

#[derive(Debug, Deserialize)]
struct ConversionJobEnvelope {
    data: ConversionJob,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapabilitiesEnvelope {
    data: CapabilitiesData,
}

#[derive(Debug, Deserialize)]
struct CapabilitiesData {
    conversion: ConversionCapabilitiesWire,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConversionCapabilitiesWire {
    accepting_jobs: bool,
    input_formats: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ServiceRequestPayload {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ServiceResponsePayload {
    status: u16,
    headers: BTreeMap<String, String>,
    body: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConversionSubmission {
    source: String,
    client_run_id: String,
    profile: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContractRoute {
    Liveness,
    Readiness,
    Capabilities,
    CreateConversion,
    GetConversion,
    ListArtifacts,
    DownloadManifest,
    DownloadMarkdown,
}

impl ContractRoute {
    fn requires_authentication(self) -> bool {
        !matches!(self, Self::Liveness | Self::Readiness | Self::Capabilities)
    }
}

#[tauri::command]
pub(crate) async fn service_request(
    app: AppHandle,
    request: ServiceRequestPayload,
) -> Result<ServiceResponsePayload, String> {
    let base_url = current_base_url(&app);
    let route = classify_route(&request.method, &request.path)?;
    let token = if route.requires_authentication() {
        Some(current_token()?)
    } else {
        None
    };
    send_service_request(&base_url, token.as_deref(), request).await
}

#[tauri::command]
pub(crate) async fn download_conversion_markdown(
    app: AppHandle,
    conversion_id: String,
    output_dir: String,
    file_name: String,
) -> Result<String, String> {
    let base_url = current_base_url(&app);
    let token = current_token()?;
    download_markdown(&base_url, &token, &conversion_id, &output_dir, &file_name).await
}

/// Fetch the live routing contract without authentication. Callers must use
/// `input_formats`; support is intentionally not duplicated in desktop code.
pub(crate) async fn fetch_capabilities(base_url: &str) -> Result<ConversionCapabilities, String> {
    let response = send_service_request(
        base_url,
        None,
        ServiceRequestPayload {
            method: "GET".into(),
            path: "/api/v1/capabilities".into(),
            headers: BTreeMap::new(),
            body: None,
        },
    )
    .await?;
    let envelope: CapabilitiesEnvelope = parse_success(response, None, "capabilities")?;
    Ok(ConversionCapabilities {
        accepting_jobs: envelope.data.conversion.accepting_jobs,
        input_formats: envelope.data.conversion.input_formats,
    })
}

/// Submit a source path through the host-only multipart door. The webview
/// never observes the bearer token or the source bytes.
pub(crate) async fn submit_conversion(
    base_url: &str,
    token: &str,
    source_path: &str,
    client_run_id: &str,
    profile: &str,
    idempotency_key: &str,
) -> Result<ConversionJob, String> {
    let body = serde_json::to_string(&serde_json::json!({
        "source": source_path,
        "clientRunId": client_run_id,
        "profile": profile,
    }))
    .map_err(|e| format!("Could not prepare conversion submission: {e}"))?;
    let response = send_service_request(
        base_url,
        Some(token),
        ServiceRequestPayload {
            method: "POST".into(),
            path: "/api/v1/conversions".into(),
            headers: BTreeMap::from([("idempotency-key".into(), idempotency_key.into())]),
            body: Some(body),
        },
    )
    .await?;
    let envelope: ConversionJobEnvelope = parse_success(response, Some(token), "submission")?;
    validate_uuid(&envelope.data.id, "conversion id returned by submission")?;
    Ok(envelope.data)
}

/// Poll an accepted conversion using its original service origin.
pub(crate) async fn poll_conversion(
    base_url: &str,
    token: &str,
    conversion_id: &str,
) -> Result<ConversionJob, String> {
    validate_uuid(conversion_id, "conversion id")?;
    let response = send_service_request(
        base_url,
        Some(token),
        ServiceRequestPayload {
            method: "GET".into(),
            path: format!("/api/v1/conversions/{conversion_id}"),
            headers: BTreeMap::new(),
            body: None,
        },
    )
    .await?;
    let envelope: ConversionJobEnvelope = parse_success(response, Some(token), "poll")?;
    validate_uuid(&envelope.data.id, "conversion id returned by poll")?;
    if envelope.data.id != conversion_id {
        return Err(
            "Conversion service poll returned a different conversion id than requested".into(),
        );
    }
    Ok(envelope.data)
}

pub(crate) fn validate_base_url(base_url: &str) -> Result<(), String> {
    endpoint_url(base_url, "/health/live").map(|_| ())
}

pub(crate) fn validate_conversion_id(conversion_id: &str) -> Result<(), String> {
    validate_uuid(conversion_id, "conversion id")
}

fn parse_success<T: for<'de> Deserialize<'de>>(
    response: ServiceResponsePayload,
    token: Option<&str>,
    operation: &str,
) -> Result<T, String> {
    if !(200..300).contains(&response.status) {
        let detail = redact_token(&response.body, token);
        return Err(format!(
            "Conversion service {operation} returned HTTP {}: {detail}",
            response.status
        ));
    }
    serde_json::from_str(&response.body)
        .map_err(|_| format!("Conversion service returned an invalid {operation} response"))
}

/// Send one bounded, contract-allowlisted request against an explicit service.
/// Keeping the service URL outside Settings here lets restart recovery replay
/// against the server recorded with the original in-flight conversion.
pub(crate) async fn send_service_request(
    base_url: &str,
    token: Option<&str>,
    request: ServiceRequestPayload,
) -> Result<ServiceResponsePayload, String> {
    let route = classify_route(&request.method, &request.path)?;
    if route == ContractRoute::DownloadMarkdown {
        return Err(
            "Markdown artifacts must use download_conversion_markdown and cannot cross IPC".into(),
        );
    }

    let endpoint = endpoint_url(base_url, &request.path)?;
    let headers = safe_request_headers(&request.headers)?;
    let client = http_client()?;
    let response = if route == ContractRoute::CreateConversion {
        send_conversion(&client, endpoint, token, headers, request.body).await?
    } else {
        if request.body.is_some() {
            return Err("GET conversion-service requests cannot carry a body".into());
        }
        let builder = client
            .request(Method::GET, endpoint)
            .headers(headers)
            .timeout(SMALL_REQUEST_TIMEOUT);
        let builder = if route.requires_authentication() {
            builder.bearer_auth(required_token(token)?)
        } else {
            builder
        };
        builder.send().await.map_err(transport_error)?
    };

    response_payload(response, token).await
}

/// Stream a Markdown artifact from an explicit service directly to disk.
/// Only the final path is returned; response bytes are never serialized.
pub(crate) async fn download_markdown(
    base_url: &str,
    token: &str,
    conversion_id: &str,
    output_dir: &str,
    file_name: &str,
) -> Result<String, String> {
    download_markdown_with_limit(
        base_url,
        token,
        conversion_id,
        output_dir,
        file_name,
        MARKDOWN_BODY_LIMIT,
    )
    .await
}

async fn download_markdown_with_limit(
    base_url: &str,
    token: &str,
    conversion_id: &str,
    output_dir: &str,
    file_name: &str,
    body_limit: usize,
) -> Result<String, String> {
    validate_uuid(conversion_id, "conversion id")?;
    let (output_dir, stem) = validate_output(output_dir, file_name).await?;
    let path = format!("/api/v1/conversions/{conversion_id}/artifacts/markdown");
    let endpoint = endpoint_url(base_url, &path)?;
    let client = http_client()?;
    let mut response = client
        .get(endpoint)
        .bearer_auth(valid_token(token)?)
        .timeout(STREAM_REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(transport_error)?;

    if !response.status().is_success() {
        let status = response.status();
        let body = read_bounded_body(&mut response, GENERIC_BODY_LIMIT).await?;
        let detail = redact_token(&String::from_utf8_lossy(&body), Some(token));
        return Err(format!(
            "Conversion service returned {status} while downloading Markdown: {detail}"
        ));
    }
    let is_markdown = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/markdown"));
    if !is_markdown {
        return Err("Conversion service returned a non-Markdown artifact".into());
    }
    if response
        .content_length()
        .is_some_and(|length| length > body_limit as u64)
    {
        return Err(format!(
            "Markdown artifact exceeds the {body_limit}-byte host limit"
        ));
    }

    let (path, mut file) = reserve_markdown_path(&output_dir, &stem).await?;
    let write_result: Result<(), String> = async {
        let mut written = 0usize;
        while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
            written = written.saturating_add(chunk.len());
            if written > body_limit {
                return Err(format!(
                    "Markdown artifact exceeds the {body_limit}-byte host limit"
                ));
            }
            file.write_all(&chunk)
                .await
                .map_err(|e| format!("Could not write {}: {e}", path.display()))?;
        }
        if written == 0 {
            return Err("Conversion service returned an empty Markdown artifact".into());
        }
        file.sync_all()
            .await
            .map_err(|e| format!("Could not sync {}: {e}", path.display()))
    }
    .await;
    drop(file);

    if let Err(error) = write_result {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(error);
    }

    Ok(path.to_string_lossy().into_owned())
}

fn current_base_url(app: &AppHandle) -> String {
    settings::load(app).backend_url
}

fn current_token() -> Result<String, String> {
    secrets::get_key("backend")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Backend token is not configured in Keychain".to_string())
}

/// One client for the process. A `Client` owns the connection pool, so building
/// one per request threw the pooled connection away every time and made a ten
/// minute conversion open roughly 120 of them. Cloning is a refcount bump.
fn http_client() -> Result<Client, String> {
    static CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .build()
                .map_err(|e| format!("Could not initialize conversion-service HTTP: {e}"))
        })
        .clone()
}

fn valid_token(token: &str) -> Result<&str, String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err("Backend token is not configured".into());
    }
    HeaderValue::from_str(&format!("Bearer {trimmed}"))
        .map_err(|_| "Backend token contains invalid header characters".to_string())?;
    Ok(trimmed)
}

fn required_token(token: Option<&str>) -> Result<&str, String> {
    valid_token(token.ok_or_else(|| "Backend token is not configured".to_string())?)
}

fn endpoint_url(base_url: &str, path: &str) -> Result<Url, String> {
    let mut url = Url::parse(base_url.trim()).map_err(|_| "Backend URL is invalid".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        return Err(
            "Backend URL must be an HTTP(S) origin without credentials, path, query, or fragment"
                .into(),
        );
    }
    url.set_path(path);
    Ok(url)
}

fn classify_route(method: &str, path: &str) -> Result<ContractRoute, String> {
    if path.contains(['?', '#']) || !path.starts_with('/') {
        return Err("Conversion-service paths cannot contain a query or fragment".into());
    }
    let method = match method {
        value if value.eq_ignore_ascii_case("GET") => Method::GET,
        value if value.eq_ignore_ascii_case("POST") => Method::POST,
        _ => return Err("Conversion-service method is not allowed".into()),
    };

    match (method, path) {
        (Method::GET, "/health/live") => Ok(ContractRoute::Liveness),
        (Method::GET, "/health/ready") => Ok(ContractRoute::Readiness),
        (Method::GET, "/api/v1/capabilities") => Ok(ContractRoute::Capabilities),
        (Method::POST, "/api/v1/conversions") => Ok(ContractRoute::CreateConversion),
        (Method::GET, dynamic) => classify_conversion_get(dynamic),
        _ => Err("Conversion-service method/path combination is not allowed".into()),
    }
}

fn classify_conversion_get(path: &str) -> Result<ContractRoute, String> {
    let segments = path.split('/').collect::<Vec<_>>();
    let (id, tail) = match segments.as_slice() {
        ["", "api", "v1", "conversions", id] => (*id, &[][..]),
        ["", "api", "v1", "conversions", id, "artifacts"] => (*id, &["artifacts"][..]),
        ["", "api", "v1", "conversions", id, "artifacts", artifact] => {
            (*id, std::slice::from_ref(artifact))
        }
        _ => return Err("Conversion-service path is not in the frozen contract".into()),
    };
    validate_uuid(id, "conversion id")?;
    match tail {
        [] => Ok(ContractRoute::GetConversion),
        ["artifacts"] => Ok(ContractRoute::ListArtifacts),
        ["manifest"] => Ok(ContractRoute::DownloadManifest),
        ["markdown"] => Ok(ContractRoute::DownloadMarkdown),
        _ => Err("Conversion-service artifact path is not allowed".into()),
    }
}

fn validate_uuid(value: &str, label: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    let valid = bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                *byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        });
    if valid {
        Ok(())
    } else {
        Err(format!("Invalid {label}"))
    }
}

fn safe_request_headers(input: &BTreeMap<String, String>) -> Result<HeaderMap, String> {
    let mut output = HeaderMap::new();
    for (name, value) in input {
        let lower = name.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "authorization" | "host" | "proxy-authorization"
        ) {
            return Err(format!("Caller cannot override the {name} header"));
        }
        if !matches!(lower.as_str(), "accept" | "idempotency-key") {
            continue;
        }

        let name = HeaderName::from_bytes(lower.as_bytes())
            .map_err(|_| "Request contains an invalid header name".to_string())?;
        let value = HeaderValue::from_str(value)
            .map_err(|_| "Request contains an invalid header value".to_string())?;
        if output.insert(name, value).is_some() {
            return Err("Request contains duplicate case-insensitive headers".into());
        }
    }
    Ok(output)
}

async fn send_conversion(
    client: &Client,
    endpoint: Url,
    token: Option<&str>,
    headers: HeaderMap,
    body: Option<String>,
) -> Result<Response, String> {
    let body = body.ok_or_else(|| "Conversion submission body is required".to_string())?;
    let submission: ConversionSubmission = serde_json::from_str(&body)
        .map_err(|_| "Conversion submission body is invalid".to_string())?;
    validate_uuid(&submission.client_run_id, "clientRunId")?;
    if !matches!(
        submission.profile.as_str(),
        "standard" | "local_only" | "best_quality"
    ) {
        return Err("Conversion profile is not in the frozen contract".into());
    }

    let idempotency = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| "Idempotency-Key is required for conversion submission".to_string())?;
    validate_idempotency_key(idempotency)?;

    let source = Path::new(&submission.source);
    let metadata = tokio::fs::metadata(source)
        .await
        .map_err(|e| format!("Could not open conversion source: {e}"))?;
    if !metadata.is_file() {
        return Err("Conversion source must be a regular file".into());
    }
    let media_type = mime_guess::from_path(source)
        .first_or_octet_stream()
        .to_string();
    let source_part = multipart::Part::file(source)
        .await
        .map_err(|e| format!("Could not open conversion source: {e}"))?
        .mime_str(&media_type)
        .map_err(|_| "Conversion source media type is invalid".to_string())?;
    let form = multipart::Form::new()
        .part("source", source_part)
        .text("clientRunId", submission.client_run_id)
        .text("profile", submission.profile);

    client
        .post(endpoint)
        .headers(headers)
        .bearer_auth(required_token(token)?)
        .multipart(form)
        .timeout(STREAM_REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(transport_error)
}

fn validate_idempotency_key(value: &str) -> Result<(), String> {
    if (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._~-".contains(&byte))
    {
        Ok(())
    } else {
        Err("Idempotency-Key must be 1-128 URL-safe characters".into())
    }
}

async fn response_payload(
    mut response: Response,
    token: Option<&str>,
) -> Result<ServiceResponsePayload, String> {
    let status = response.status().as_u16();
    let headers = response_headers(response.headers(), token);
    let bytes = read_bounded_body(&mut response, GENERIC_BODY_LIMIT).await?;
    let body = String::from_utf8(bytes)
        .map_err(|_| "Conversion service returned a non-text response".to_string())?;
    let body = redact_token(&body, token);
    Ok(ServiceResponsePayload {
        status,
        headers,
        body,
    })
}

fn response_headers(headers: &HeaderMap, token: Option<&str>) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter(|(name, _)| {
            matches!(
                name.as_str(),
                "content-type"
                    | "x-request-id"
                    | "location"
                    | "retry-after"
                    | "idempotency-replayed"
                    | "etag"
            )
        })
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_string(), redact_token(value, token)))
        })
        .collect()
}

fn redact_token(value: &str, token: Option<&str>) -> String {
    let Some(token) = token.map(str::trim).filter(|token| !token.is_empty()) else {
        return value.to_string();
    };
    value.replace(token, "[REDACTED]")
}

async fn read_bounded_body(response: &mut Response, limit: usize) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(format!(
            "Conversion-service response exceeds the {limit}-byte IPC limit"
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(format!(
                "Conversion-service response exceeds the {limit}-byte IPC limit"
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn validate_output(output_dir: &str, file_name: &str) -> Result<(PathBuf, String), String> {
    let directory = PathBuf::from(output_dir);
    let metadata = tokio::fs::metadata(&directory)
        .await
        .map_err(|e| format!("Could not open output folder: {e}"))?;
    if !metadata.is_dir() {
        return Err("Output path must be a directory".into());
    }

    let path = Path::new(file_name);
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
        || file_name.contains('\0')
    {
        return Err("Output filename must be one plain filename".into());
    }
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Output filename must have a valid stem".to_string())?;
    Ok((directory, stem.to_string()))
}

async fn reserve_markdown_path(
    output_dir: &Path,
    stem: &str,
) -> Result<(PathBuf, tokio::fs::File), String> {
    for number in 0..MAX_OUTPUT_COLLISIONS {
        let name = if number == 0 {
            format!("{stem}.md")
        } else {
            format!("{stem} ({number}).md")
        };
        let candidate = output_dir.join(name);
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
            .await
        {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Could not write to the output folder: {error}")),
        }
    }
    Err(format!(
        "Could not find a free filename for {stem}.md in the output folder"
    ))
}

fn transport_error(error: reqwest::Error) -> String {
    format!("Conversion-service HTTP failed: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{AUTHORIZATION, HOST};
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::oneshot;

    const UUID: &str = "11111111-1111-4111-8111-111111111111";
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "tool-kit-conversion-service-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).expect("test directory should be created");
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct MockResponse {
        status: &'static str,
        content_type: &'static str,
        body: Vec<u8>,
        declared_length: Option<usize>,
        split_body: bool,
        chunked: bool,
    }

    struct MockServer {
        base_url: String,
        request: oneshot::Receiver<Vec<u8>>,
        task: tokio::task::JoinHandle<()>,
    }

    async fn mock_server(response: MockResponse) -> MockServer {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should have an address");
        let (request_tx, request_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("mock should accept");
            let request = read_request(&mut socket).await;
            let _ = request_tx.send(request);
            let length = response.declared_length.unwrap_or(response.body.len());
            let framing = if response.chunked {
                "Transfer-Encoding: chunked".to_string()
            } else {
                format!("Content-Length: {length}")
            };
            let headers = format!(
                "HTTP/1.1 {}\r\nContent-Type: {}\r\n{framing}\r\nX-Wire-Case: preserved\r\nConnection: close\r\n\r\n",
                response.status, response.content_type
            );
            if socket.write_all(headers.as_bytes()).await.is_err() {
                return;
            }
            if response.chunked {
                let chunk = format!("{:X}\r\n", response.body.len());
                if socket.write_all(chunk.as_bytes()).await.is_err()
                    || socket.write_all(&response.body).await.is_err()
                {
                    return;
                }
                let _ = socket.write_all(b"\r\n0\r\n\r\n").await;
                return;
            }
            if response.split_body && response.body.len() > 1 {
                let split = response.body.len() / 2;
                if socket.write_all(&response.body[..split]).await.is_err() {
                    return;
                }
                tokio::task::yield_now().await;
                let _ = socket.write_all(&response.body[split..]).await;
            } else {
                let _ = socket.write_all(&response.body).await;
            }
        });
        MockServer {
            base_url: format!("http://{address}"),
            request: request_rx,
            task,
        }
    }

    async fn read_request(socket: &mut TcpStream) -> Vec<u8> {
        let mut request = Vec::new();
        let mut buffer = [0u8; 8192];
        let mut expected = None;
        loop {
            let read = socket.read(&mut buffer).await.expect("request should read");
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if expected.is_none() {
                if let Some(end) = header_end(&request) {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let content_length = headers.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    });
                    expected = Some(end + 4 + content_length.unwrap_or(0));
                }
            }
            if expected.is_some_and(|length| request.len() >= length) {
                break;
            }
        }
        request
    }

    fn header_end(bytes: &[u8]) -> Option<usize> {
        bytes.windows(4).position(|window| window == b"\r\n\r\n")
    }

    fn get_request(path: &str) -> ServiceRequestPayload {
        ServiceRequestPayload {
            method: "GET".into(),
            path: path.into(),
            headers: BTreeMap::new(),
            body: None,
        }
    }

    #[test]
    fn only_frozen_method_and_path_pairs_are_allowed() {
        assert_eq!(
            classify_route("GET", "/api/v1/capabilities").unwrap(),
            ContractRoute::Capabilities
        );
        assert_eq!(
            classify_route("get", &format!("/api/v1/conversions/{UUID}")).unwrap(),
            ContractRoute::GetConversion
        );
        assert!(classify_route("DELETE", "/api/v1/conversions").is_err());
        assert!(classify_route("GET", "/api/v1/private").is_err());
        assert!(classify_route("GET", "/health/live?redirect=https://example.com").is_err());
        assert!(classify_route("GET", "/api/v1/conversions/not-a-uuid").is_err());
    }

    #[test]
    fn backend_url_is_an_origin_not_an_open_redirect_surface() {
        assert!(endpoint_url("https://converter.local", "/health/live").is_ok());
        assert!(endpoint_url("file:///tmp/service", "/health/live").is_err());
        assert!(endpoint_url("https://user:secret@converter.local", "/health/live").is_err());
        assert!(endpoint_url("https://converter.local/prefix", "/health/live").is_err());
    }

    #[tokio::test]
    async fn generic_request_filters_headers_and_preserves_non_success_body() {
        let server = mock_server(MockResponse {
            status: "418 I'm a teapot",
            content_type: "application/json",
            body: br#"{"error":{"code":"teapot"}}"#.to_vec(),
            declared_length: None,
            split_body: false,
            chunked: false,
        })
        .await;
        let mut headers = BTreeMap::new();
        headers.insert("Accept".into(), "application/json".into());
        headers.insert("X-Drop-Me".into(), "not-contract-metadata".into());
        let response = send_service_request(
            &server.base_url,
            None,
            ServiceRequestPayload {
                headers,
                ..get_request("/api/v1/capabilities")
            },
        )
        .await
        .unwrap();
        assert_eq!(response.status, 418);
        assert_eq!(response.body, r#"{"error":{"code":"teapot"}}"#);
        assert_eq!(
            response.headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
        assert!(!response.headers.contains_key("x-wire-case"));

        let request = String::from_utf8(server.request.await.unwrap()).unwrap();
        let lower = request.to_ascii_lowercase();
        assert!(request.starts_with("GET /api/v1/capabilities HTTP/1.1\r\n"));
        assert!(!lower.contains("authorization:"));
        assert!(lower.contains("accept: application/json\r\n"));
        assert!(!lower.contains("x-drop-me"));
        server.task.await.unwrap();
    }

    #[tokio::test]
    async fn caller_cannot_override_host_or_authorization() {
        for forbidden in [AUTHORIZATION.as_str(), HOST.as_str(), "Proxy-Authorization"] {
            let mut request = get_request("/api/v1/capabilities");
            request.headers.insert(forbidden.into(), "attacker".into());
            let error = send_service_request("http://127.0.0.1:9", Some("real-token"), request)
                .await
                .unwrap_err();
            assert!(error.contains("cannot override"));
        }

        let error = send_service_request(
            "http://127.0.0.1:9",
            None,
            get_request(&format!("/api/v1/conversions/{UUID}")),
        )
        .await
        .unwrap_err();
        assert!(error.contains("token is not configured"));
    }

    #[tokio::test]
    async fn authenticated_response_cannot_echo_the_keychain_token_to_ipc() {
        let server = mock_server(MockResponse {
            status: "400 Bad Request",
            content_type: "application/json",
            body: br#"{"error":"private-token"}"#.to_vec(),
            declared_length: None,
            split_body: false,
            chunked: false,
        })
        .await;
        let response = send_service_request(
            &server.base_url,
            Some("private-token"),
            get_request(&format!("/api/v1/conversions/{UUID}")),
        )
        .await
        .unwrap();
        assert_eq!(response.status, 400);
        assert!(!response.body.contains("private-token"));
        assert!(response.body.contains("[REDACTED]"));
        let _ = server.request.await.unwrap();
        server.task.await.unwrap();
    }

    #[tokio::test]
    async fn conversion_upload_streams_a_real_file_as_contract_cased_multipart() {
        let dir = TestDir::new();
        let source = dir.0.join("source.pdf");
        tokio::fs::write(&source, b"%PDF-real-file-bytes")
            .await
            .unwrap();
        let server = mock_server(MockResponse {
            status: "202 Accepted",
            content_type: "application/json",
            body: br#"{"data":{"id":"accepted"}}"#.to_vec(),
            declared_length: None,
            split_body: false,
            chunked: false,
        })
        .await;
        let mut headers = BTreeMap::new();
        headers.insert("Idempotency-Key".into(), "stable-key_1".into());
        headers.insert("Content-Type".into(), "application/json".into());
        let response = send_service_request(
            &server.base_url,
            Some("upload-token"),
            ServiceRequestPayload {
                method: "POST".into(),
                path: "/api/v1/conversions".into(),
                headers,
                body: Some(
                    serde_json::json!({
                        "source": source,
                        "clientRunId": UUID,
                        "profile": "standard"
                    })
                    .to_string(),
                ),
            },
        )
        .await
        .unwrap();
        assert_eq!(response.status, 202);

        let request = String::from_utf8_lossy(&server.request.await.unwrap()).into_owned();
        let lower = request.to_ascii_lowercase();
        assert!(request.starts_with("POST /api/v1/conversions HTTP/1.1\r\n"));
        assert!(lower.contains("authorization: bearer upload-token\r\n"));
        assert!(lower.contains("idempotency-key: stable-key_1\r\n"));
        assert!(lower.contains("content-type: multipart/form-data; boundary="));
        assert!(request.contains("name=\"source\"; filename=\"source.pdf\""));
        assert!(request.contains("Content-Type: application/pdf"));
        assert!(request.contains("%PDF-real-file-bytes"));
        assert!(request.contains("name=\"clientRunId\""));
        assert!(request.contains(UUID));
        assert!(request.contains("name=\"profile\""));
        assert!(request.contains("standard"));
        server.task.await.unwrap();
    }

    #[tokio::test]
    async fn typed_capabilities_and_unknown_job_status_are_future_safe() {
        let capabilities = mock_server(MockResponse {
            status: "200 OK",
            content_type: "application/json",
            body: br#"{"data":{"conversion":{"acceptingJobs":true,"inputFormats":["application/pdf","application/epub+zip"]}}}"#.to_vec(),
            declared_length: None,
            split_body: false,
            chunked: false,
        })
        .await;
        let parsed = fetch_capabilities(&capabilities.base_url).await.unwrap();
        assert!(parsed.accepting_jobs);
        assert_eq!(
            parsed.input_formats,
            ["application/pdf", "application/epub+zip"]
        );
        let _ = capabilities.request.await.unwrap();
        capabilities.task.await.unwrap();

        let poll = mock_server(MockResponse {
            status: "200 OK",
            content_type: "application/json",
            body: format!(
                r#"{{"data":{{"id":"{UUID}","status":"paused_by_future_backend","route":{{"kind":"future_engine","reasonCodes":["future_reason"]}},"warnings":["still safe"]}}}}"#
            )
            .into_bytes(),
            declared_length: None,
            split_body: false,
            chunked: false,
        })
        .await;
        let parsed = poll_conversion(&poll.base_url, "poll-token", UUID)
            .await
            .unwrap();
        assert_eq!(parsed.status, "paused_by_future_backend");
        assert_eq!(parsed.warnings, ["still safe"]);
        let route = parsed.route.unwrap();
        assert_eq!(route.kind, "future_engine");
        assert_eq!(route.reason_codes, ["future_reason"]);
        let request = String::from_utf8(poll.request.await.unwrap()).unwrap();
        assert!(request.starts_with(&format!("GET /api/v1/conversions/{UUID} HTTP/1.1\r\n")));
        poll.task.await.unwrap();
    }

    #[tokio::test]
    async fn submit_rejects_an_invalid_response_conversion_id_without_leaking_token() {
        let dir = TestDir::new();
        let source = dir.0.join("source.pdf");
        tokio::fs::write(&source, b"%PDF-invalid-response-id")
            .await
            .unwrap();
        let server = mock_server(MockResponse {
            status: "202 Accepted",
            content_type: "application/json",
            body: br#"{"data":{"id":"not-a-uuid","status":"queued","warnings":[]}}"#.to_vec(),
            declared_length: None,
            split_body: false,
            chunked: false,
        })
        .await;
        let token = "private-submit-token";

        let error = submit_conversion(
            &server.base_url,
            token,
            &source.to_string_lossy(),
            UUID,
            "standard",
            "stable-invalid-id-key",
        )
        .await
        .unwrap_err();

        assert_eq!(error, "Invalid conversion id returned by submission");
        assert!(!error.contains(token));
        let _ = server.request.await.unwrap();
        server.task.await.unwrap();
    }

    #[tokio::test]
    async fn poll_rejects_a_different_response_conversion_id_without_leaking_token() {
        const OTHER_UUID: &str = "22222222-2222-4222-8222-222222222222";
        let server = mock_server(MockResponse {
            status: "200 OK",
            content_type: "application/json",
            body: format!(
                r#"{{"data":{{"id":"{OTHER_UUID}","status":"paused_by_future_backend","route":{{"kind":"future_engine","reasonCodes":["future_reason"]}},"warnings":[]}}}}"#
            )
            .into_bytes(),
            declared_length: None,
            split_body: false,
            chunked: false,
        })
        .await;
        let token = "private-poll-token";

        let error = poll_conversion(&server.base_url, token, UUID)
            .await
            .unwrap_err();

        assert_eq!(
            error,
            "Conversion service poll returned a different conversion id than requested"
        );
        assert!(!error.contains(token));
        let request = String::from_utf8(server.request.await.unwrap()).unwrap();
        assert!(request.starts_with(&format!("GET /api/v1/conversions/{UUID} HTTP/1.1\r\n")));
        server.task.await.unwrap();
    }

    #[tokio::test]
    async fn submit_helper_keeps_the_stable_key_and_parses_job_metadata() {
        let dir = TestDir::new();
        let source = dir.0.join("source.pdf");
        tokio::fs::write(&source, b"%PDF-helper").await.unwrap();
        let server = mock_server(MockResponse {
            status: "202 Accepted",
            content_type: "application/json",
            body: format!(
                r#"{{"data":{{"id":"{UUID}","status":"queued","route":{{"kind":"local_pdf","reasonCodes":["pdf_supported"]}},"warnings":[]}}}}"#
            )
            .into_bytes(),
            declared_length: None,
            split_body: false,
            chunked: false,
        })
        .await;

        let job = submit_conversion(
            &server.base_url,
            "submit-token",
            &source.to_string_lossy(),
            UUID,
            "standard",
            "stable-replay-key",
        )
        .await
        .unwrap();
        assert_eq!(job.id, UUID);
        assert_eq!(job.route.unwrap().reason_codes, ["pdf_supported"]);
        let request = String::from_utf8_lossy(&server.request.await.unwrap()).into_owned();
        assert!(request
            .to_ascii_lowercase()
            .contains("idempotency-key: stable-replay-key\r\n"));
        server.task.await.unwrap();
    }

    #[tokio::test]
    async fn submit_loss_can_replay_the_same_idempotency_key() {
        let dir = TestDir::new();
        let source = dir.0.join("replay.pdf");
        tokio::fs::write(&source, b"%PDF-replay").await.unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (requests_tx, mut requests_rx) = tokio::sync::mpsc::unbounded_channel();
        let server = tokio::spawn(async move {
            let (mut lost_socket, _) = listener.accept().await.unwrap();
            requests_tx
                .send(read_request(&mut lost_socket).await)
                .unwrap();
            drop(lost_socket);

            let (mut replay_socket, _) = listener.accept().await.unwrap();
            requests_tx
                .send(read_request(&mut replay_socket).await)
                .unwrap();
            let body = format!(r#"{{"data":{{"id":"{UUID}","status":"queued","warnings":[]}}}}"#);
            let response = format!(
                "HTTP/1.1 202 Accepted\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            replay_socket.write_all(response.as_bytes()).await.unwrap();
        });
        let base_url = format!("http://{address}");
        let source_path = source.to_string_lossy().into_owned();
        let submit = || {
            submit_conversion(
                &base_url,
                "submit-token",
                &source_path,
                UUID,
                "standard",
                "same-key-after-loss",
            )
        };

        assert!(submit().await.is_err());
        assert_eq!(submit().await.unwrap().id, UUID);
        let first = String::from_utf8(requests_rx.recv().await.unwrap()).unwrap();
        let second = String::from_utf8(requests_rx.recv().await.unwrap()).unwrap();
        for request in [first, second] {
            assert!(request
                .to_ascii_lowercase()
                .contains("idempotency-key: same-key-after-loss\r\n"));
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn generic_request_refuses_markdown_and_caps_other_bodies() {
        let markdown = format!("/api/v1/conversions/{UUID}/artifacts/markdown");
        let error = send_service_request("http://127.0.0.1:9", None, get_request(&markdown))
            .await
            .unwrap_err();
        assert!(error.contains("cannot cross IPC"));

        let server = mock_server(MockResponse {
            status: "200 OK",
            content_type: "application/json",
            body: vec![b'x'; GENERIC_BODY_LIMIT + 1],
            declared_length: None,
            split_body: false,
            chunked: false,
        })
        .await;
        let error =
            send_service_request(&server.base_url, None, get_request("/api/v1/capabilities"))
                .await
                .unwrap_err();
        assert!(error.contains("IPC limit"));
        let _ = server.request.await.unwrap();
        server.task.await.unwrap();
    }

    #[tokio::test]
    async fn markdown_download_streams_to_the_next_collision_safe_name() {
        let dir = TestDir::new();
        tokio::fs::write(dir.0.join("report.md"), b"existing")
            .await
            .unwrap();
        let server = mock_server(MockResponse {
            status: "200 OK",
            content_type: "text/markdown",
            body: b"# streamed\n\nbody".to_vec(),
            declared_length: None,
            split_body: true,
            chunked: false,
        })
        .await;
        let path = download_markdown(
            &server.base_url,
            "download-token",
            UUID,
            dir.0.to_str().unwrap(),
            "report.pdf",
        )
        .await
        .unwrap();
        assert_eq!(Path::new(&path).file_name().unwrap(), "report (1).md");
        assert_eq!(
            tokio::fs::read_to_string(path).await.unwrap(),
            "# streamed\n\nbody"
        );

        let request = String::from_utf8(server.request.await.unwrap()).unwrap();
        assert!(request.starts_with(&format!(
            "GET /api/v1/conversions/{UUID}/artifacts/markdown HTTP/1.1\r\n"
        )));
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer download-token\r\n"));
        server.task.await.unwrap();
    }

    #[tokio::test]
    async fn markdown_download_leaves_no_file_on_non_success_or_truncated_body() {
        let non_success_dir = TestDir::new();
        let server = mock_server(MockResponse {
            status: "409 Conflict",
            content_type: "application/json",
            body: br#"{"error":{"code":"conversion_not_ready"}}"#.to_vec(),
            declared_length: None,
            split_body: false,
            chunked: false,
        })
        .await;
        let error = download_markdown(
            &server.base_url,
            "token",
            UUID,
            non_success_dir.0.to_str().unwrap(),
            "report.pdf",
        )
        .await
        .unwrap_err();
        assert!(error.contains("conversion_not_ready"));
        assert_eq!(std::fs::read_dir(&non_success_dir.0).unwrap().count(), 0);
        let _ = server.request.await.unwrap();
        server.task.await.unwrap();

        let partial_dir = TestDir::new();
        let server = mock_server(MockResponse {
            status: "200 OK",
            content_type: "text/markdown",
            body: b"short".to_vec(),
            declared_length: Some(100),
            split_body: false,
            chunked: false,
        })
        .await;
        let error = download_markdown(
            &server.base_url,
            "token",
            UUID,
            partial_dir.0.to_str().unwrap(),
            "report.pdf",
        )
        .await
        .unwrap_err();
        assert!(error.contains("HTTP failed"));
        assert_eq!(std::fs::read_dir(&partial_dir.0).unwrap().count(), 0);
        let _ = server.request.await.unwrap();
        server.task.await.unwrap();

        let overrun_dir = TestDir::new();
        let server = mock_server(MockResponse {
            status: "200 OK",
            content_type: "text/markdown",
            body: b"12345".to_vec(),
            declared_length: None,
            split_body: false,
            chunked: true,
        })
        .await;
        let error = download_markdown_with_limit(
            &server.base_url,
            "token",
            UUID,
            overrun_dir.0.to_str().unwrap(),
            "report.pdf",
            4,
        )
        .await
        .unwrap_err();
        assert!(error.contains("host limit"));
        assert_eq!(std::fs::read_dir(&overrun_dir.0).unwrap().count(), 0);
        let _ = server.request.await.unwrap();
        server.task.await.unwrap();
    }

    #[tokio::test]
    async fn markdown_download_validates_id_directory_and_plain_filename() {
        let dir = TestDir::new();
        assert!(download_markdown(
            "http://127.0.0.1:9",
            "token",
            "bad-id",
            dir.0.to_str().unwrap(),
            "report.pdf"
        )
        .await
        .is_err());
        assert!(download_markdown(
            "http://127.0.0.1:9",
            "token",
            UUID,
            dir.0.to_str().unwrap(),
            "../report.pdf"
        )
        .await
        .is_err());
        let file = dir.0.join("not-a-directory");
        tokio::fs::write(&file, b"file").await.unwrap();
        assert!(download_markdown(
            "http://127.0.0.1:9",
            "token",
            UUID,
            file.to_str().unwrap(),
            "report.pdf"
        )
        .await
        .is_err());
    }
}
