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

pub enum PollResult {
    Pending,
    Done(String),
    Failed(String),
}

const DATALAB_BASE: &str = "https://www.datalab.to";
const DATALAB_CONVERT: &str = "https://www.datalab.to/api/v1/convert";
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

/// Read a file once into memory, returning (bytes, filename, mime) so a
/// multipart body can be rebuilt cheaply on each retry attempt.
async fn read_file_bytes(
    path: &str,
    default_name: &str,
) -> Result<(Vec<u8>, String, String), String> {
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
    Ok((bytes, name, mime))
}

fn bytes_part(bytes: Vec<u8>, name: &str, mime: &str) -> reqwest::multipart::Part {
    let part = reqwest::multipart::Part::bytes(bytes).file_name(name.to_string());
    // A panic here would abort the spawned job task, leaving the row stuck on
    // "Uploading…" forever, so fall back instead of unwrapping.
    part.mime_str(mime)
        .unwrap_or_else(|_| reqwest::multipart::Part::bytes(Vec::new()).file_name(name.to_string()))
}

/// Seconds to wait per `retry-after`, capped so a large server-supplied delay
/// can't pin a concurrency permit for hours. The integer form is the only one
/// these APIs send; an HTTP-date falls back to our own backoff.
fn retry_after_secs(resp: &reqwest::Response) -> Option<u64> {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|s| s.min(60))
}

/// Upload timeout. Submits carry the whole file, so they need far longer than a
/// poll — a 300s cap made large Transcribe files impossible to submit at all.
pub const UPLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Poll timeout. Polls are tiny GETs; if one hangs, the next tick retries.
pub const POLL_TIMEOUT: Duration = Duration::from_secs(60);

/// Turn a non-success poll response into a terminal failure where the status
/// says retrying can't help. 5xx and 429 stay transient (caller keeps polling).
fn terminal_poll_error(status: reqwest::StatusCode, body: &Value, who: &str) -> Option<PollResult> {
    if status.is_success() || status.is_server_error() || status.as_u16() == 429 {
        return None;
    }
    let detail = pick_string(body, &["error", "detail", "message", "title"])
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
                // Only retry statuses that mean the request was refused before
                // any work happened. A 500 may mean the job was created and
                // then errored — retrying it would bill the user twice.
                let transient = matches!(code, 429 | 502 | 503 | 504 | 529);
                if transient && attempt < MAX_TRIES {
                    let wait = retry_after_secs(&resp).unwrap_or(2u64.pow(attempt));
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                    continue;
                }
                return Ok(resp);
            }
            Err(e) => {
                // Same reasoning: a failure while connecting means the body was
                // never sent. A timeout or a mid-body error might have reached
                // the server, so don't resend and risk a duplicate charge.
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
) -> Result<Submitted, String> {
    let (bytes, name, mime) = read_file_bytes(path, "file").await?;
    let resp = send_retrying(|| {
        // Page delimiters are always on so any figure can be cited to a page.
        let mut form = reqwest::multipart::Form::new()
            .part("file", bytes_part(bytes.clone(), &name, &mime))
            .text("output_format", output_format.to_string())
            .text("paginate", "true");
        // High-accuracy profile, tuned for SIM/CIM source docs (tax returns,
        // P&Ls, balance sheets — frequently scanned and table-heavy). It trades
        // credits and latency for fidelity, which is the right default when
        // every extracted figure ends up cited in a buyer-facing memorandum,
        // and wasted effort on a clean digital PDF:
        //   use_llm      — LLM pass that markedly improves tables/forms/layout
        //   force_ocr    — re-OCR every page, ignoring unreliable embedded text
        //   format_lines — reconstruct lines cleanly (keeps financial rows intact)
        if high_accuracy {
            form = form
                .text("use_llm", "true")
                .text("force_ocr", "true")
                .text("format_lines", "true");
        }
        client
            .post(DATALAB_CONVERT)
            .header("X-API-Key", api_key)
            .timeout(UPLOAD_TIMEOUT)
            .multipart(form)
            .send()
    })
    .await?;
    parse_datalab_submit(resp).await
}

async fn parse_datalab_submit(resp: reqwest::Response) -> Result<Submitted, String> {
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Bad response from Datalab: {e}"))?;
    if !status.is_success() {
        return Err(pick_string(&body, &["error", "detail", "message"])
            .unwrap_or_else(|| format!("Datalab error ({status})")));
    }
    if !body
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Err(pick_string(&body, &["error", "detail"])
            .unwrap_or_else(|| "Datalab rejected the request".into()));
    }
    Ok(Submitted {
        remote_id: body
            .get("request_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        check_url: body
            .get("request_check_url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
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
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Bad poll response from Datalab: {e}"))?;
    if let Some(failed) = terminal_poll_error(http, &body, "Datalab") {
        return Ok(failed);
    }
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
        // An absent/empty result field used to be written out as a 0-byte file
        // and reported as a success. Fail the job instead.
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
) -> Result<Submitted, String> {
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
    .await?;
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Bad response from Datalab pipeline: {e}"))?;
    if !status.is_success() {
        return Err(pick_string(&body, &["error", "detail", "message"])
            .unwrap_or_else(|| format!("Datalab pipeline error ({status})")));
    }
    let exec_id = body
        .get("execution_id")
        .and_then(|v| v.as_str())
        .ok_or("Datalab pipeline did not return an execution id")?;
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
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Bad poll response from Datalab pipeline: {e}"))?;
    if let Some(failed) = terminal_poll_error(http, &body, "Datalab pipeline") {
        return Ok(failed);
    }
    match body.get("status").and_then(|v| v.as_str()).unwrap_or("") {
        "completed" | "completed_with_errors" => {
            let steps = body
                .get("steps")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            // Take the last *completed* step. Defaulting to step 0 when nothing
            // completed fetched a failed step's error payload and wrote it to
            // disk as if it were the converted document.
            let step_index = steps
                .iter()
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
            // Same rule as the transcript fetch: the pipeline run is already
            // paid for, so a 5xx on collecting the result is worth another tick.
            if rhttp.is_server_error() || rhttp.as_u16() == 429 {
                return Err(format!("Datalab step result returned {rhttp}"));
            }
            let rbody: Value = rresp
                .json()
                .await
                .map_err(|e| format!("Bad step result from Datalab: {e}"))?;
            if !rhttp.is_success() {
                return Ok(PollResult::Failed(format!(
                    "Datalab step result returned {rhttp}"
                )));
            }
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
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Bad response from Rev.ai: {e}"))?;
    if !status.is_success() {
        return Err(pick_string(&body, &["detail", "message", "title", "error"])
            .unwrap_or_else(|| format!("Rev.ai error ({status})")));
    }
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
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Bad poll response from Rev.ai: {e}"))?;
    if let Some(failed) = terminal_poll_error(http, &body, "Rev.ai") {
        return Ok(failed);
    }
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
            // The transcription is finished and already billed by the time we
            // ask for it, so a transient blip here must not discard it. `Err`
            // sends the caller's poll loop round again (it tolerates ~1 minute
            // of unbroken failure); only a client error is worth giving up on.
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
