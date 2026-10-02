//! Provider integrations. Both providers follow the same shape:
//! submit -> poll -> fetch.

use serde_json::Value;
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderKind {
    Datalab,
    RevAi,
}

impl ProviderKind {
    pub fn label(&self) -> &'static str {
        match self {
            ProviderKind::Datalab => "Datalab",
            ProviderKind::RevAi => "Rev.ai",
        }
    }
    /// Keychain account name for this provider's API key.
    pub fn key_name(&self) -> &'static str {
        match self {
            ProviderKind::Datalab => "datalab",
            ProviderKind::RevAi => "revai",
        }
    }
}

pub struct Submitted {
    pub remote_id: String,
    pub check_url: Option<String>,
}

impl Submitted {
    /// Datalab's own status URL when it sent one, else the documented default.
    pub fn datalab_check_url(&self) -> String {
        self.check_url
            .clone()
            .unwrap_or_else(|| format!("{DATALAB_BASE}/api/v1/convert/{}", self.remote_id))
    }
}

pub enum PollResult {
    Pending,
    Done(String),
    Failed(String),
}

/// Whether sending this submit again can bill the same file twice.
#[derive(Debug)]
pub enum SubmitError {
    /// The provider answered and refused, so a retry buys the first one.
    Refused(String),
    /// No outcome came back: the upload may have landed with no id to poll.
    Uncertain(String),
}

impl SubmitError {
    pub fn message(&self) -> &str {
        match self {
            Self::Refused(message) | Self::Uncertain(message) => message,
        }
    }
}

/// Everything that fails before the body is sent is a refusal.
impl From<String> for SubmitError {
    fn from(message: String) -> Self {
        Self::Refused(message)
    }
}

const DATALAB_BASE: &str = "https://www.datalab.to";
const REVAI_JOBS: &str = "https://api.rev.ai/speechtotext/v1/jobs";

/// Pull the first present, non-null value from `keys`, as text.
fn pick_string(body: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        match body.get(*k) {
            Some(Value::String(s)) if !s.is_empty() => return Some(s.clone()),
            Some(v) if !v.is_null() && !matches!(v, Value::String(_)) => {
                return Some(serde_json::to_string_pretty(v).unwrap_or_default());
            }
            _ => {}
        }
    }
    None
}

/// Read a submit response: the body on success, the server's message on
/// failure. All three submits answer the same shape.
async fn submit_body(resp: reqwest::Response, who: &str) -> Result<Value, SubmitError> {
    let status = resp.status();
    let body: Value = match resp.json().await {
        Ok(body) => body,
        // An unreadable body under a success status is an accepted upload whose
        // id we lost, not a refusal.
        Err(e) => {
            let message = format!("Bad response from {who}: {e}");
            return Err(if status.is_success() {
                SubmitError::Uncertain(message)
            } else {
                SubmitError::Refused(message)
            });
        }
    };
    if !status.is_success() {
        return Err(SubmitError::Refused(
            pick_string(&body, &["error", "detail", "message", "title"])
                .unwrap_or_else(|| format!("{who} error ({status})")),
        ));
    }
    Ok(body)
}

/// Read a file once, so a multipart body can be rebuilt cheaply per attempt.
/// `Bytes`, not `Vec<u8>`: `send_retrying` takes `Fn`, so a `Vec` clone per
/// attempt holds two copies of the file at once.
async fn read_file_bytes(
    path: &str,
    default_name: &str,
) -> Result<(bytes::Bytes, String, String), String> {
    let p = Path::new(path);
    let name = p
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(default_name)
        .to_string();
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| format!("Could not read file: {e}"))?;
    let mime = mime_guess::from_path(p).first_or_octet_stream().to_string();
    Ok((bytes::Bytes::from(bytes), name, mime))
}

fn bytes_part(bytes: bytes::Bytes, name: &str, mime: &str) -> reqwest::multipart::Part {
    // `stream_with_length`, not `Part::bytes`, which copies the buffer back
    // out. The length is required, or the body is chunked and both refuse.
    let len = bytes.len() as u64;
    let part = |bytes: bytes::Bytes| {
        reqwest::multipart::Part::stream_with_length(reqwest::Body::from(bytes), len)
            .file_name(name.to_string())
    };
    // A panic aborts the job task and strands the row on "Uploading…". A bad
    // type sends the same bytes untyped, never an empty file.
    part(bytes.clone())
        .mime_str(mime)
        .unwrap_or_else(|_| part(bytes))
}

/// Seconds per `retry-after`, capped so a large one cannot pin a permit for
/// hours. Only the integer form is sent, and a date falls back to our backoff.
fn retry_after_secs(resp: &reqwest::Response) -> Option<u64> {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|s| s.min(60))
}

/// Upload timeout. A submit carries the whole file, so it needs far longer
/// than a poll.
pub const UPLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Poll timeout. A poll is a tiny GET, and the next tick retries a hang.
pub const POLL_TIMEOUT: Duration = Duration::from_secs(60);

/// Terminal where the status says retrying cannot help. 5xx and 429 stay
/// transient. The body is optional: a gateway's HTML 401 is still a 401.
fn terminal_poll_error(
    status: reqwest::StatusCode,
    body: Option<&Value>,
    who: &str,
) -> Option<PollResult> {
    if status.is_success() || status.is_server_error() || status.as_u16() == 429 {
        return None;
    }
    let detail = body
        .and_then(|body| pick_string(body, &["error", "detail", "message", "title"]))
        .unwrap_or_else(|| status.to_string());
    Some(PollResult::Failed(format!(
        "{who} returned {status}: {detail}"
    )))
}

/// Send a request with up to 3 attempts, retrying only failures that prove the
/// server never started work (429/502/503/504/529, and connect errors) so a
/// retry can't double-bill. `make` rebuilds the request each attempt because
/// multipart bodies are single-use. Honors `retry-after`.
async fn send_retrying<F, Fut>(make: F) -> Result<reqwest::Response, String>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<reqwest::Response, reqwest::Error>>,
{
    const MAX_TRIES: u32 = 3;
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        match make().await {
            Ok(resp) => {
                let code = resp.status().as_u16();
                // Only statuses meaning the request was refused before any work
                // happened. A 500 may mean the job was created, then errored.
                let transient = matches!(code, 429 | 502 | 503 | 504 | 529);
                if transient && attempt < MAX_TRIES {
                    let wait = retry_after_secs(&resp).unwrap_or(2u64.pow(attempt));
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                    continue;
                }
                return Ok(resp);
            }
            Err(e) => {
                // A connect failure means the body never went. A timeout or a
                // mid-body error may have reached the server.
                if e.is_connect() && attempt < MAX_TRIES {
                    tokio::time::sleep(Duration::from_secs(2u64.pow(attempt))).await;
                    continue;
                }
                return Err(e.to_string());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Datalab — standard /convert endpoint
// ---------------------------------------------------------------------------

pub async fn datalab_submit(
    client: &reqwest::Client,
    api_key: &str,
    path: &str,
    output_format: &str,
    high_accuracy: bool,
) -> Result<Submitted, SubmitError> {
    let (bytes, name, mime) = read_file_bytes(path, "file").await?;
    let resp = send_retrying(|| {
        // Page delimiters are always on so any figure can be cited to a page.
        let mut form = reqwest::multipart::Form::new()
            .part("file", bytes_part(bytes.clone(), &name, &mime))
            .text("output_format", output_format.to_string())
            .text("paginate", "true");
        // High-accuracy profile for scanned, table-heavy documents: an LLM
        // pass over tables and layout, a re-OCR of every page ignoring embedded
        // text, and clean line reconstruction. Wasted on a digital PDF.
        if high_accuracy {
            form = form
                .text("use_llm", "true")
                .text("force_ocr", "true")
                .text("format_lines", "true");
        }
        client
            .post(format!("{DATALAB_BASE}/api/v1/convert"))
            .header("X-API-Key", api_key)
            .timeout(UPLOAD_TIMEOUT)
            .multipart(form)
            .send()
    })
    .await
    // No answer, so the request id is unknowable either way.
    .map_err(SubmitError::Uncertain)?;
    let body = submit_body(resp, "Datalab").await?;
    if !body
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Err(SubmitError::Refused(
            pick_string(&body, &["error", "detail"])
                .unwrap_or_else(|| "Datalab rejected the request".into()),
        ));
    }
    let remote_id = body
        .get("request_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let check_url = body
        .get("request_check_url")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    // Accepted and maybe billed, but nothing to poll.
    if remote_id.is_empty() && check_url.is_none() {
        return Err(SubmitError::Uncertain(
            "Datalab did not return a request id".into(),
        ));
    }
    Ok(Submitted {
        remote_id,
        check_url,
    })
}

pub async fn datalab_poll(
    client: &reqwest::Client,
    api_key: &str,
    check_url: &str,
    output_format: &str,
) -> Result<PollResult, String> {
    let resp = client
        .get(check_url)
        .header("X-API-Key", api_key)
        .timeout(POLL_TIMEOUT)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let http = resp.status();
    let body = resp.json::<Value>().await;
    if let Some(failed) = terminal_poll_error(http, body.as_ref().ok(), "Datalab") {
        return Ok(failed);
    }
    let body = body.map_err(|e| format!("Bad poll response from Datalab: {e}"))?;
    let status = body.get("status").and_then(|v| v.as_str()).unwrap_or("");
    if status == "complete" {
        if !body
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
        {
            return Ok(PollResult::Failed(
                pick_string(&body, &["error", "detail"])
                    .unwrap_or_else(|| "Conversion failed".into()),
            ));
        }
        let keys: &[&str] = match output_format {
            "html" => &["html", "output"],
            "json" => &["json", "output"],
            "chunks" => &["chunks", "json", "output"],
            _ => &["markdown", "output", "content"],
        };
        // An empty result field would otherwise be a 0-byte file called done.
        return Ok(match pick_string(&body, keys) {
            Some(text) if !text.trim().is_empty() => PollResult::Done(text),
            _ => PollResult::Failed(
                "Datalab reported success but returned no content for this file.".into(),
            ),
        });
    }
    if let Some(err) = body.get("error").and_then(|v| v.as_str()) {
        if !err.is_empty() {
            return Ok(PollResult::Failed(err.to_string()));
        }
    }
    Ok(PollResult::Pending)
}

// ---------------------------------------------------------------------------
// Datalab — named pipelines (pl_...): run -> poll execution -> step result
// ---------------------------------------------------------------------------

pub async fn datalab_pipeline_submit(
    client: &reqwest::Client,
    api_key: &str,
    pipeline_id: &str,
    path: &str,
    output_format: &str,
) -> Result<Submitted, SubmitError> {
    let url = format!("{DATALAB_BASE}/api/v1/pipelines/{pipeline_id}/run");
    let (bytes, name, mime) = read_file_bytes(path, "file").await?;
    let resp = send_retrying(|| {
        let form = reqwest::multipart::Form::new()
            .part("file", bytes_part(bytes.clone(), &name, &mime))
            .text("output_format", output_format.to_string());
        client
            .post(&url)
            .header("X-API-Key", api_key)
            .timeout(UPLOAD_TIMEOUT)
            .multipart(form)
            .send()
    })
    .await
    .map_err(SubmitError::Uncertain)?;
    let body = submit_body(resp, "Datalab pipeline").await?;
    // An accepted run with no id in the answer: started, and unpollable.
    let exec_id = body
        .get("execution_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            SubmitError::Uncertain("Datalab pipeline did not return an execution id".into())
        })?;
    Ok(Submitted {
        remote_id: exec_id.to_string(),
        check_url: None,
    })
}

pub async fn datalab_pipeline_poll(
    client: &reqwest::Client,
    api_key: &str,
    execution_id: &str,
) -> Result<PollResult, String> {
    let url = format!("{DATALAB_BASE}/api/v1/pipelines/executions/{execution_id}");
    let resp = client
        .get(&url)
        .header("X-API-Key", api_key)
        .timeout(POLL_TIMEOUT)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let http = resp.status();
    let body = resp.json::<Value>().await;
    if let Some(failed) = terminal_poll_error(http, body.as_ref().ok(), "Datalab pipeline") {
        return Ok(failed);
    }
    let body = body.map_err(|e| format!("Bad poll response from Datalab pipeline: {e}"))?;
    match body.get("status").and_then(|v| v.as_str()).unwrap_or("") {
        "completed" | "completed_with_errors" => {
            // The last *completed* step. Step 0 fetches a failed step's error
            // payload and writes it to disk as the document.
            let step_index = body
                .get("steps")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter(|s| s.get("status").and_then(|v| v.as_str()) == Some("completed"))
                .filter_map(|s| s.get("step_index").and_then(|v| v.as_i64()))
                .max();
            let Some(step_index) = step_index else {
                return Ok(PollResult::Failed(
                    pick_string(&body, &["error", "error_message", "detail"])
                        .unwrap_or_else(|| "Pipeline finished with no completed step".into()),
                ));
            };
            let rurl = format!(
                "{DATALAB_BASE}/api/v1/pipelines/executions/{execution_id}/steps/{step_index}/result"
            );
            let rresp = client
                .get(&rurl)
                .header("X-API-Key", api_key)
                .timeout(POLL_TIMEOUT)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            let rhttp = rresp.status();
            // The run is already paid for, so a 5xx here is worth another tick.
            if rhttp.is_server_error() || rhttp.as_u16() == 429 {
                return Err(format!("Datalab step result returned {rhttp}"));
            }
            // Status before parse, so a 404 with an HTML body is terminal.
            if !rhttp.is_success() {
                return Ok(PollResult::Failed(format!(
                    "Datalab step result returned {rhttp}"
                )));
            }
            let rbody: Value = rresp
                .json()
                .await
                .map_err(|e| format!("Bad step result from Datalab: {e}"))?;
            match pick_string(
                &rbody,
                &["markdown", "html", "json", "output", "content", "text"],
            ) {
                Some(text) if !text.trim().is_empty() => Ok(PollResult::Done(text)),
                _ => Ok(PollResult::Failed(
                    "Datalab pipeline returned no content for this file.".into(),
                )),
            }
        }
        "failed" => Ok(PollResult::Failed(
            pick_string(&body, &["error", "error_message", "detail"])
                .unwrap_or_else(|| "Pipeline failed".into()),
        )),
        _ => Ok(PollResult::Pending),
    }
}

/// A pipeline id switches Convert from /convert to a pipeline run. Both the
/// fallback and the direct path come here, so neither picks its own endpoint.
pub async fn datalab_submit_any(
    client: &reqwest::Client,
    api_key: &str,
    pipeline: Option<&str>,
    path: &str,
    output_format: &str,
    high_accuracy: bool,
) -> Result<Submitted, SubmitError> {
    match pipeline {
        Some(id) => datalab_pipeline_submit(client, api_key, id, path, output_format).await,
        None => datalab_submit(client, api_key, path, output_format, high_accuracy).await,
    }
}

pub async fn datalab_poll_any(
    client: &reqwest::Client,
    api_key: &str,
    pipeline: bool,
    remote_id: &str,
    check_url: &str,
    output_format: &str,
) -> Result<PollResult, String> {
    if pipeline {
        datalab_pipeline_poll(client, api_key, remote_id).await
    } else {
        datalab_poll(client, api_key, check_url, output_format).await
    }
}

// ---------------------------------------------------------------------------
// Rev.ai — speech to text
// ---------------------------------------------------------------------------

pub async fn revai_submit(
    client: &reqwest::Client,
    api_key: &str,
    path: &str,
) -> Result<Submitted, String> {
    let (bytes, name, mime) = read_file_bytes(path, "media").await?;
    let resp = send_retrying(|| {
        let form =
            reqwest::multipart::Form::new().part("media", bytes_part(bytes.clone(), &name, &mime));
        client
            .post(REVAI_JOBS)
            .bearer_auth(api_key)
            .timeout(UPLOAD_TIMEOUT)
            .multipart(form)
            .send()
    })
    .await?;
    // Transcribe has no uncertainty guard, so its job row carries the failure.
    let body = submit_body(resp, "Rev.ai")
        .await
        .map_err(|e| e.message().to_string())?;
    let id = body
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("Rev.ai did not return a job id")?;
    Ok(Submitted {
        remote_id: id.to_string(),
        check_url: None,
    })
}

pub async fn revai_poll(
    client: &reqwest::Client,
    api_key: &str,
    job_id: &str,
) -> Result<PollResult, String> {
    let resp = client
        .get(format!("{REVAI_JOBS}/{job_id}"))
        .bearer_auth(api_key)
        .timeout(POLL_TIMEOUT)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let http = resp.status();
    let body = resp.json::<Value>().await;
    if let Some(failed) = terminal_poll_error(http, body.as_ref().ok(), "Rev.ai") {
        return Ok(failed);
    }
    let body = body.map_err(|e| format!("Bad poll response from Rev.ai: {e}"))?;
    match body.get("status").and_then(|v| v.as_str()).unwrap_or("") {
        "transcribed" => {
            let tresp = client
                .get(format!("{REVAI_JOBS}/{job_id}/transcript"))
                .bearer_auth(api_key)
                .header(reqwest::header::ACCEPT, "text/plain")
                .timeout(POLL_TIMEOUT)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            // The transcription is already billed by the time we ask, so a blip
            // must not discard it. `Err` polls again, a client error ends it.
            let ts = tresp.status();
            if ts.is_server_error() || ts.as_u16() == 429 {
                return Err(format!("Transcript fetch returned {ts}"));
            }
            if !ts.is_success() {
                return Ok(PollResult::Failed(format!(
                    "Could not fetch transcript ({ts})"
                )));
            }
            let text = tresp.text().await.map_err(|e| e.to_string())?;
            // Rev.ai returns 200 with an empty body for silent media.
            if text.trim().is_empty() {
                return Ok(PollResult::Failed(
                    "Rev.ai returned an empty transcript — no speech was detected.".into(),
                ));
            }
            Ok(PollResult::Done(text))
        }
        "failed" => Ok(PollResult::Failed(
            pick_string(&body, &["failure_detail", "failure"])
                .unwrap_or_else(|| "Transcription failed".into()),
        )),
        _ => Ok(PollResult::Pending),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Answer one request with `response`, verbatim, and hand back its URL.
    async fn serve_once(response: String) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/check", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0u8; 4096];
            let _ = socket.read(&mut buffer).await;
            let _ = socket.write_all(response.as_bytes()).await;
        });
        url
    }

    /// A gateway's HTML 401 is still a 401. Polling it for a minute fixes
    /// nothing, while an HTML 503 stays transient.
    #[tokio::test]
    async fn the_status_decides_a_poll_before_the_body_is_parsed() {
        let client = reqwest::Client::new();
        let html = |status| {
            format!("HTTP/1.1 {status}\r\nContent-Type: text/html\r\nContent-Length: 13\r\nConnection: close\r\n\r\n<html></html>")
        };

        let url = serve_once(html("401 Unauthorized")).await;
        match datalab_poll(&client, "key", &url, "markdown").await {
            Ok(PollResult::Failed(message)) => assert!(message.contains("401"), "{message}"),
            Ok(_) => panic!("a 401 read as pending or done"),
            Err(error) => panic!("a 401 read as transient: {error}"),
        }

        let url = serve_once(html("503 Service Unavailable")).await;
        assert!(datalab_poll(&client, "key", &url, "markdown")
            .await
            .is_err());
    }
}
