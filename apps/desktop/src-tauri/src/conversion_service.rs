//! Native HTTP boundary for the conversion service.
//!
//! The webview never talks to the service: it asks the host for capabilities
//! and nothing else. This module owns file streaming, response bounds and
//! artifact writes. The origin and the token come from `backend_host`.

use reqwest::header::{HeaderValue, CONTENT_TYPE};
use reqwest::{multipart, Client, Response, Url};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};
use std::sync::OnceLock;
use std::time::Duration;
use tauri::AppHandle;
use tokio::io::AsyncWriteExt;

use crate::backend_host;

const GENERIC_BODY_LIMIT: usize = 1024 * 1024;
/// Matches the backend's documented output ceiling. A service may advertise
/// less at runtime, which is the tighter check.
const MARKDOWN_BODY_LIMIT: usize = 50 * 1024 * 1024;
const SMALL_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const STREAM_REQUEST_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// The contract's ceiling on every multipart text part. Checked here, so the
/// offending field is named rather than 422ing every job in the run.
const MAX_METADATA_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConversionCapabilities {
    pub(crate) accepting_jobs: bool,
    pub(crate) input_formats: Vec<String>,
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
    conversion: ConversionCapabilities,
}

/// A bounded response, its body already scrubbed of the token.
#[derive(Debug)]
struct ServiceResponse {
    status: u16,
    body: String,
}

/// Every per-run engine setting the service folds into its replay key, named
/// for the first of them. Read by the image engine and the audio engine alone,
/// so a recovered job must resubmit exactly what it was submitted with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OcrOptions {
    pub(crate) language_correction: bool,
    pub(crate) custom_words: Vec<String>,
    pub(crate) speaker_count: Option<u32>,
    /// BCP 47, `None` for the Mac's own language.
    pub(crate) speech_locale: Option<String>,
}

impl Default for OcrOptions {
    /// The values the contract treats as absent.
    fn default() -> Self {
        Self {
            language_correction: true,
            custom_words: Vec::new(),
            speaker_count: None,
            speech_locale: None,
        }
    }
}

impl OcrOptions {
    /// One word per line: the contract's wire form, and what the ledger stores.
    pub(crate) fn custom_words_wire(&self) -> String {
        self.custom_words.join("\n")
    }

    /// Rebuild from the stored wire form.
    pub(crate) fn from_wire(
        language_correction: bool,
        custom_words: &str,
        speaker_count: Option<u32>,
        speech_locale: Option<String>,
    ) -> Self {
        Self {
            language_correction,
            custom_words: custom_words.lines().map(str::to_owned).collect(),
            speaker_count,
            speech_locale,
        }
    }
}

/// The webview's one question for the service: is it taking jobs.
#[tauri::command]
pub(crate) async fn conversion_capabilities(
    app: AppHandle,
) -> Result<ConversionCapabilities, String> {
    fetch_capabilities(&backend_host::backend_origin(&app)?).await
}

/// Fetch the live routing contract, unauthenticated. Callers read
/// `input_formats`, which is never duplicated in desktop code.
pub(crate) async fn fetch_capabilities(base_url: &str) -> Result<ConversionCapabilities, String> {
    let response = send_service_request(base_url, None, "/api/v1/capabilities").await?;
    let envelope: CapabilitiesEnvelope = parse_success(response, "capabilities")?;
    Ok(envelope.data.conversion)
}

/// Stream a source file as multipart, so the webview sees neither the bearer
/// token nor the source bytes.
pub(crate) async fn submit_conversion(
    base_url: &str,
    token: &str,
    source_path: &str,
    client_run_id: &str,
    profile: &str,
    ocr: &OcrOptions,
    idempotency_key: &str,
) -> Result<ConversionJob, String> {
    let custom_words = ocr.custom_words_wire();
    if custom_words.len() > MAX_METADATA_BYTES {
        return Err(format!(
            "Custom words are too long ({} bytes, {MAX_METADATA_BYTES} maximum). Shorten them in Settings.",
            custom_words.len()
        ));
    }
    validate_uuid(client_run_id, "clientRunId")?;
    validate_idempotency_key(idempotency_key)?;
    let endpoint = endpoint_url(base_url, "/api/v1/conversions")?;

    let source = Path::new(source_path);
    let metadata = tokio::fs::metadata(source)
        .await
        .map_err(|e| format!("Could not open conversion source: {e}"))?;
    if !metadata.is_file() {
        return Err("Conversion source must be a regular file".into());
    }
    let media_type = crate::media_type(source);
    let source_part = multipart::Part::file(source)
        .await
        .map_err(|e| format!("Could not open conversion source: {e}"))?
        .mime_str(&media_type)
        .map_err(|_| "Conversion source media type is invalid".to_string())?;
    let mut form = multipart::Form::new()
        .part("source", source_part)
        .text("clientRunId", client_run_id.to_owned())
        .text("profile", profile.to_owned());
    // Absent is the documented default for each, and an older service answers
    // an unknown field with 422. Only a changed OCR setting sends a part.
    if !ocr.language_correction {
        form = form.text("languageCorrection", "false");
    }
    if !custom_words.is_empty() {
        form = form.text("customWords", custom_words);
    }
    if let Some(speakers) = ocr.speaker_count {
        form = form.text("speakerCount", speakers.to_string());
    }
    if let Some(locale) = &ocr.speech_locale {
        form = form.text("speechLocale", locale.clone());
    }

    let response = http_client()?
        .post(endpoint)
        .header("idempotency-key", idempotency_key)
        .bearer_auth(valid_token(Some(token))?)
        .multipart(form)
        .timeout(STREAM_REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(transport_error)?;
    let response = response_payload(response, Some(token)).await?;
    let envelope: ConversionJobEnvelope = parse_success(response, "submission")?;
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
        &format!("/api/v1/conversions/{conversion_id}"),
    )
    .await?;
    let envelope: ConversionJobEnvelope = parse_success(response, "poll")?;
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

/// The body arrives already redacted from `response_payload`, which is the one
/// place that scrubs the token.
fn parse_success<T: for<'de> Deserialize<'de>>(
    response: ServiceResponse,
    operation: &str,
) -> Result<T, String> {
    if !(200..300).contains(&response.status) {
        // The service's own sentence, not its JSON: this lands in the job row.
        let message = serde_json::from_str::<serde_json::Value>(&response.body)
            .ok()
            .and_then(|body| body["error"]["message"].as_str().map(str::to_owned))
            .unwrap_or(response.body);
        return Err(format!(
            "Conversion service {operation} failed (HTTP {}): {message}",
            response.status
        ));
    }
    serde_json::from_str(&response.body)
        .map_err(|_| format!("Conversion service returned an invalid {operation} response"))
}

/// One bounded GET against an explicit service, so recovery can replay
/// against the server the in-flight row recorded. A token means bearer auth.
async fn send_service_request(
    base_url: &str,
    token: Option<&str>,
    path: &str,
) -> Result<ServiceResponse, String> {
    let mut builder = http_client()?
        .get(endpoint_url(base_url, path)?)
        .timeout(SMALL_REQUEST_TIMEOUT);
    if token.is_some() {
        builder = builder.bearer_auth(valid_token(token)?);
    }
    let response = builder.send().await.map_err(transport_error)?;
    response_payload(response, token).await
}

/// Stream a Markdown artifact from an explicit service directly to disk.
/// Only the final path is returned. Response bytes are never serialized.
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
    validate_output(output_dir, file_name).await?;
    let path = format!("/api/v1/conversions/{conversion_id}/artifacts/markdown");
    let endpoint = endpoint_url(base_url, &path)?;
    let client = http_client()?;
    let mut response = client
        .get(endpoint)
        .bearer_auth(valid_token(Some(token))?)
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

    let (file, path) = crate::jobs::claim_path(output_dir, file_name, "md")?;
    let mut file = tokio::fs::File::from_std(file);
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

/// One client for the process. A `Client` owns the connection pool, so one per
/// request opens roughly 120 on a ten-minute conversion.
pub(crate) fn http_client() -> Result<Client, String> {
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

fn valid_token(token: Option<&str>) -> Result<&str, String> {
    let trimmed = token
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| "Backend token is not configured".to_string())?;
    HeaderValue::from_str(&format!("Bearer {trimmed}"))
        .map_err(|_| "Backend token contains invalid header characters".to_string())?;
    Ok(trimmed)
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

pub(crate) fn validate_uuid(value: &str, label: &str) -> Result<(), String> {
    // Length 36 admits the hyphenated form alone.
    if value.len() == 36 && uuid::Uuid::try_parse(value).is_ok() {
        Ok(())
    } else {
        Err(format!("Invalid {label}"))
    }
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
) -> Result<ServiceResponse, String> {
    let status = response.status().as_u16();
    let bytes = read_bounded_body(&mut response, GENERIC_BODY_LIMIT).await?;
    let body = String::from_utf8(bytes)
        .map_err(|_| "Conversion service returned a non-text response".to_string())?;
    Ok(ServiceResponse {
        status,
        body: redact_token(&body, token),
    })
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
            "Conversion-service response exceeds the {limit}-byte host limit"
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(format!(
                "Conversion-service response exceeds the {limit}-byte host limit"
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn validate_output(output_dir: &str, file_name: &str) -> Result<(), String> {
    let metadata = tokio::fs::metadata(output_dir)
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
    Ok(())
}

fn transport_error(error: reqwest::Error) -> String {
    format!("Conversion-service HTTP failed: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::oneshot;

    const UUID: &str = "11111111-1111-4111-8111-111111111111";

    /// A refused upload reaches the job row as the service's sentence, not
    /// as its JSON envelope.
    #[test]
    fn a_refusal_reads_as_the_services_message() {
        let refused = ServiceResponse {
            status: 413,
            body: r#"{"error":{"code":"upload_too_large","message":"The upload exceeds the configured limit.","requestId":"r","details":[]}}"#.into(),
        };
        let error = parse_success::<ConversionJobEnvelope>(refused, "submission").unwrap_err();
        assert_eq!(
            error,
            "Conversion service submission failed (HTTP 413): The upload exceeds the configured limit."
        );
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

    /// A hand-edited settings.json can exceed the contract's text-part budget,
    /// which would 422 every job in the run.
    #[tokio::test]
    async fn custom_words_over_the_metadata_budget_are_refused_by_name() {
        let ocr = OcrOptions {
            language_correction: true,
            custom_words: vec!["Uniwise".into(); 40],
            speaker_count: None,
            speech_locale: None,
        };
        assert!(ocr.custom_words_wire().len() > MAX_METADATA_BYTES);
        let error = submit_conversion(
            "http://127.0.0.1:9",
            "token",
            "/tmp/does-not-matter.png",
            UUID,
            "standard",
            &ocr,
            "over-budget-key",
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("Custom words are too long"), "{error}");
    }

    #[test]
    fn a_uuid_must_be_hyphenated() {
        assert!(validate_uuid(UUID, "id").is_ok());
        assert!(validate_uuid("ABCDEF01-2345-4789-8abc-DEF012345678", "id").is_ok());
        assert!(validate_uuid(&UUID.replace('-', ""), "id").is_err());
        assert!(validate_uuid(&format!("{{{UUID}}}"), "id").is_err());
        assert!(validate_uuid(&format!("urn:uuid:{UUID}"), "id").is_err());
    }

    #[test]
    fn backend_url_is_an_origin_not_an_open_redirect_surface() {
        assert!(endpoint_url("https://converter.local", "/health/live").is_ok());
        assert!(endpoint_url("file:///tmp/service", "/health/live").is_err());
        assert!(endpoint_url("https://user:secret@converter.local", "/health/live").is_err());
        assert!(endpoint_url("https://converter.local/prefix", "/health/live").is_err());
    }

    /// A service that echoes the token cannot put it in a job row.
    #[tokio::test]
    async fn an_error_body_cannot_echo_the_token() {
        let server = mock_server(MockResponse {
            status: "400 Bad Request",
            content_type: "application/json",
            body: br#"{"error":"private-token"}"#.to_vec(),
            declared_length: None,
            split_body: false,
            chunked: false,
        })
        .await;
        let error = poll_conversion(&server.base_url, "private-token", UUID)
            .await
            .unwrap_err();
        assert!(error.contains("HTTP 400"), "{error}");
        assert!(!error.contains("private-token"));
        assert!(error.contains("[REDACTED]"));
        let _ = server.request.await.unwrap();
        server.task.await.unwrap();
    }

    #[tokio::test]
    async fn conversion_upload_streams_a_real_file_as_contract_cased_multipart() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.pdf");
        tokio::fs::write(&source, b"%PDF-real-file-bytes")
            .await
            .unwrap();
        let server = mock_server(MockResponse {
            status: "202 Accepted",
            content_type: "application/json",
            body: format!(r#"{{"data":{{"id":"{UUID}","status":"queued","warnings":[]}}}}"#)
                .into_bytes(),
            declared_length: None,
            split_body: false,
            chunked: false,
        })
        .await;
        let ocr = OcrOptions {
            language_correction: false,
            custom_words: vec!["Uniwise".into(), "Tool-Kit".into()],
            speaker_count: None,
            speech_locale: None,
        };
        let job = submit_conversion(
            &server.base_url,
            "upload-token",
            &source.to_string_lossy(),
            UUID,
            "standard",
            &ocr,
            "stable-key_1",
        )
        .await
        .unwrap();
        assert_eq!(job.id, UUID);

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
        assert!(request.contains("name=\"languageCorrection\""));
        assert!(request.contains("false"));
        assert!(request.contains("name=\"customWords\""));
        assert!(request.contains("Uniwise\nTool-Kit"));
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
        let request = String::from_utf8(capabilities.request.await.unwrap()).unwrap();
        assert!(request.starts_with("GET /api/v1/capabilities HTTP/1.1\r\n"));
        assert!(!request.to_ascii_lowercase().contains("authorization:"));
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
        let request = String::from_utf8(poll.request.await.unwrap()).unwrap();
        assert!(request.starts_with(&format!("GET /api/v1/conversions/{UUID} HTTP/1.1\r\n")));
        poll.task.await.unwrap();
    }

    #[tokio::test]
    async fn submit_rejects_an_invalid_response_conversion_id_without_leaking_token() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.pdf");
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
            &OcrOptions::default(),
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
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.pdf");
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
            &OcrOptions {
                language_correction: false,
                custom_words: vec!["Uniwise".into(), "Tool-Kit".into()],
                speaker_count: Some(2),
                speech_locale: None,
            },
            "stable-replay-key",
        )
        .await
        .unwrap();
        assert_eq!(job.id, UUID);
        let request = String::from_utf8_lossy(&server.request.await.unwrap()).into_owned();
        assert!(request
            .to_ascii_lowercase()
            .contains("idempotency-key: stable-replay-key\r\n"));
        // The list reaches the wire as one part, one word per line.
        assert!(request.contains("name=\"languageCorrection\"\r\n\r\nfalse"));
        assert!(request.contains("name=\"customWords\"\r\n\r\nUniwise\nTool-Kit"));
        assert!(request.contains("name=\"speakerCount\"\r\n\r\n2"));
        server.task.await.unwrap();
    }

    #[tokio::test]
    async fn submit_loss_can_replay_the_same_idempotency_key() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("replay.pdf");
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
        let ocr = OcrOptions::default();
        let submit = || {
            submit_conversion(
                &base_url,
                "submit-token",
                &source_path,
                UUID,
                "standard",
                &ocr,
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
            // Default OCR settings put nothing on the wire, so an older service
            // still accepts the submission.
            assert!(!request.contains("name=\"languageCorrection\""));
            assert!(!request.contains("name=\"customWords\""));
            assert!(!request.contains("name=\"speakerCount\""));
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn an_oversized_response_body_is_refused() {
        let server = mock_server(MockResponse {
            status: "200 OK",
            content_type: "application/json",
            body: vec![b'x'; GENERIC_BODY_LIMIT + 1],
            declared_length: None,
            split_body: false,
            chunked: false,
        })
        .await;
        let error = fetch_capabilities(&server.base_url).await.unwrap_err();
        assert!(error.contains("host limit"), "{error}");
        let _ = server.request.await.unwrap();
        server.task.await.unwrap();
    }

    #[tokio::test]
    async fn markdown_download_streams_to_the_next_collision_safe_name() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("report.md"), b"existing")
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
            dir.path().to_str().unwrap(),
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
        let non_success_dir = tempfile::tempdir().unwrap();
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
            non_success_dir.path().to_str().unwrap(),
            "report.pdf",
        )
        .await
        .unwrap_err();
        assert!(error.contains("conversion_not_ready"));
        assert_eq!(
            std::fs::read_dir(non_success_dir.path()).unwrap().count(),
            0
        );
        let _ = server.request.await.unwrap();
        server.task.await.unwrap();

        let partial_dir = tempfile::tempdir().unwrap();
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
            partial_dir.path().to_str().unwrap(),
            "report.pdf",
        )
        .await
        .unwrap_err();
        assert!(error.contains("HTTP failed"));
        assert_eq!(std::fs::read_dir(partial_dir.path()).unwrap().count(), 0);
        let _ = server.request.await.unwrap();
        server.task.await.unwrap();

        let overrun_dir = tempfile::tempdir().unwrap();
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
            overrun_dir.path().to_str().unwrap(),
            "report.pdf",
            4,
        )
        .await
        .unwrap_err();
        assert!(error.contains("host limit"));
        assert_eq!(std::fs::read_dir(overrun_dir.path()).unwrap().count(), 0);
        let _ = server.request.await.unwrap();
        server.task.await.unwrap();
    }

    #[tokio::test]
    async fn markdown_download_validates_id_directory_and_plain_filename() {
        let dir = tempfile::tempdir().unwrap();
        assert!(download_markdown(
            "http://127.0.0.1:9",
            "token",
            "bad-id",
            dir.path().to_str().unwrap(),
            "report.pdf"
        )
        .await
        .is_err());
        assert!(download_markdown(
            "http://127.0.0.1:9",
            "token",
            UUID,
            dir.path().to_str().unwrap(),
            "../report.pdf"
        )
        .await
        .is_err());
        let file = dir.path().join("not-a-directory");
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
