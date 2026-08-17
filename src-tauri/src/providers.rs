//! Provider integrations. Document/audio providers follow submit -> poll ->
//! fetch; the LLM provider (Claude) is a single synchronous request.

use serde_json::Value;
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderKind {
    Datalab,
    RevAi,
    Anthropic,
}

impl ProviderKind {
    pub fn label(&self) -> &'static str {
        match self {
            ProviderKind::Datalab => "Datalab",
            ProviderKind::RevAi => "Rev.ai",
            ProviderKind::Anthropic => "Claude",
        }
    }
    /// Keychain account name for this provider's API key.
    pub fn key_name(&self) -> &'static str {
        match self {
            ProviderKind::Datalab => "datalab",
            ProviderKind::RevAi => "revai",
            ProviderKind::Anthropic => "anthropic",
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
const ANTHROPIC_MESSAGES: &str = "https://api.anthropic.com/v1/messages";

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
async fn read_file_bytes(path: &str, default_name: &str) -> Result<(Vec<u8>, String, String), String> {
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
    reqwest::multipart::Part::bytes(bytes)
        .file_name(name.to_string())
        .mime_str(mime)
        .expect("mime_guess produces a valid MIME type")
}

fn retry_after_secs(resp: &reqwest::Response) -> Option<u64> {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
}

/// Send a request with up to 3 attempts, retrying transient failures (HTTP 429,
/// 529, 5xx, and network errors) with backoff. `make` rebuilds the request each
/// attempt because multipart bodies are single-use. Honors `retry-after`.
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
                let transient = code == 429 || code == 529 || resp.status().is_server_error();
                if transient && attempt < MAX_TRIES {
                    let wait = retry_after_secs(&resp).unwrap_or(2u64.pow(attempt));
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                    continue;
                }
                return Ok(resp);
            }
            Err(e) => {
                if attempt < MAX_TRIES {
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
) -> Result<Submitted, String> {
    let (bytes, name, mime) = read_file_bytes(path, "file").await?;
    let resp = send_retrying(|| {
        // High-accuracy profile for SIM/CIM source docs (tax returns, P&Ls,
        // balance sheets — frequently scanned, table-heavy). These flags trade
        // credits + latency for fidelity, which is the right call when every
        // extracted figure ends up cited in a buyer-facing memorandum:
        //   use_llm     — LLM pass that markedly improves tables/forms/layout
        //   force_ocr   — re-OCR every page, ignoring unreliable embedded text
        //   format_lines— reconstruct lines cleanly (keeps financial rows intact)
        //   paginate    — emit page delimiters so figures can be cited to a page
        let form = reqwest::multipart::Form::new()
            .part("file", bytes_part(bytes.clone(), &name, &mime))
            .text("output_format", output_format.to_string())
            .text("use_llm", "true")
            .text("force_ocr", "true")
            .text("format_lines", "true")
            .text("paginate", "true");
        client
            .post(DATALAB_CONVERT)
            .header("X-API-Key", api_key)
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
    if !body.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
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
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Bad poll response from Datalab: {e}"))?;
    let status = body.get("status").and_then(|v| v.as_str()).unwrap_or("");
    if status == "complete" {
        if !body.get("success").and_then(|v| v.as_bool()).unwrap_or(true) {
            return Ok(PollResult::Failed(
                pick_string(&body, &["error", "detail"]).unwrap_or_else(|| "Conversion failed".into()),
            ));
        }
        let keys: &[&str] = match output_format {
            "html" => &["html", "output"],
            "json" => &["json", "output"],
            "chunks" => &["chunks", "json", "output"],
            _ => &["markdown", "output", "content"],
        };
        return Ok(PollResult::Done(pick_string(&body, keys).unwrap_or_default()));
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
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Bad poll response from Datalab pipeline: {e}"))?;
    match body.get("status").and_then(|v| v.as_str()).unwrap_or("") {
        "completed" | "completed_with_errors" => {
            let steps = body
                .get("steps")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let mut step_index: i64 = 0;
            for s in &steps {
                let ok = s.get("status").and_then(|v| v.as_str()) == Some("completed");
                let idx = s.get("step_index").and_then(|v| v.as_i64()).unwrap_or(0);
                if ok && idx >= step_index {
                    step_index = idx;
                }
            }
            let rurl = format!(
                "{DATALAB_BASE}/api/v1/pipelines/executions/{execution_id}/steps/{step_index}/result"
            );
            let rbody: Value = client
                .get(&rurl)
                .header("X-API-Key", api_key)
                .send()
                .await
                .map_err(|e| e.to_string())?
                .json()
                .await
                .map_err(|e| format!("Bad step result from Datalab: {e}"))?;
            let text = pick_string(
                &rbody,
                &["markdown", "html", "json", "output", "content", "text"],
            )
            .unwrap_or_else(|| serde_json::to_string_pretty(&rbody).unwrap_or_default());
            Ok(PollResult::Done(text))
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
        let form = reqwest::multipart::Form::new().part("media", bytes_part(bytes.clone(), &name, &mime));
        client.post(REVAI_JOBS).bearer_auth(api_key).multipart(form).send()
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
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Bad poll response from Rev.ai: {e}"))?;
    match body.get("status").and_then(|v| v.as_str()).unwrap_or("") {
        "transcribed" => {
            let tresp = client
                .get(format!("{REVAI_JOBS}/{job_id}/transcript"))
                .bearer_auth(api_key)
                .header(reqwest::header::ACCEPT, "text/plain")
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !tresp.status().is_success() {
                return Err(format!("Could not fetch transcript ({})", tresp.status()));
            }
            Ok(PollResult::Done(tresp.text().await.map_err(|e| e.to_string())?))
        }
        "failed" => Ok(PollResult::Failed(
            pick_string(&body, &["failure_detail", "failure"])
                .unwrap_or_else(|| "Transcription failed".into()),
        )),
        _ => Ok(PollResult::Pending),
    }
}

// ---------------------------------------------------------------------------
// Claude — summarize text into AI-ready markdown notes
// ---------------------------------------------------------------------------

pub async fn anthropic_summarize(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    text: &str,
) -> Result<String, String> {
    let prompt = format!(
        "Summarize the document below into clear, well-structured Markdown notes. \
         Capture the key points, decisions, figures, and action items. Use headings and \
         bullet lists. Output only the Markdown, no preamble.\n\n---\n\n{text}"
    );
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 8192,
        "messages": [{ "role": "user", "content": prompt }],
    });
    let resp = send_retrying(|| {
        client
            .post(ANTHROPIC_MESSAGES)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
    })
    .await?;
    let status = resp.status();
    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("Bad response from Claude: {e}"))?;
    if !status.is_success() {
        let msg = json
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("Claude error ({status})"));
        return Err(msg);
    }
    // Check stop_reason before trusting the content: a refusal can carry an
    // empty body, and a max_tokens stop means the summary is truncated.
    let stop = json.get("stop_reason").and_then(|v| v.as_str()).unwrap_or("");
    if stop == "refusal" {
        return Err("Claude declined to summarize this content.".into());
    }
    let mut out = json
        .get("content")
        .and_then(|c| c.as_array())
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    if out.trim().is_empty() {
        return Err("Claude returned an empty summary".into());
    }
    if stop == "max_tokens" {
        out.push_str(
            "\n\n<!-- Summary truncated: hit the model's output limit. \
             Raise the summarize output cap if you need the full text. -->",
        );
    }
    Ok(out)
}
