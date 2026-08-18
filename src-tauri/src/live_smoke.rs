//! Live smoke tests against the real Datalab and Rev.ai APIs.
//!
//! These hit the network and spend real API credits, so they are `#[ignore]`d.
//! Run explicitly with keys in the environment:
//!
//! ```sh
//! DATALAB_API_KEY=... REVAI_API_KEY=... \
//!   cargo test --manifest-path src-tauri/Cargo.toml --lib live_smoke -- --ignored --nocapture
//! ```
//!
//! `TEST_PDF` / `TEST_AUDIO` override the default input paths.

use crate::providers::{self, PollResult};
use std::time::Duration;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

#[tokio::test]
#[ignore = "hits the live Datalab API; run with DATALAB_API_KEY set"]
async fn datalab_convert_live() {
    let key = std::env::var("DATALAB_API_KEY").expect("set DATALAB_API_KEY");
    let path = env_or("TEST_PDF", "/tmp/toolkit-test/sample.pdf");
    let client = reqwest::Client::new();

    let submitted = providers::datalab_submit(&client, &key, &path, "markdown", true)
        .await
        .expect("datalab submit");
    let check_url = submitted.check_url.clone().unwrap_or_else(|| {
        format!(
            "https://www.datalab.to/api/v1/convert/{}",
            submitted.remote_id
        )
    });

    let mut markdown = None;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_secs(5)).await;
        match providers::datalab_poll(&client, &key, &check_url, "markdown")
            .await
            .expect("datalab poll")
        {
            PollResult::Done(t) => {
                markdown = Some(t);
                break;
            }
            PollResult::Failed(e) => panic!("datalab failed: {e}"),
            PollResult::Pending => {}
        }
    }

    let markdown = markdown.expect("datalab timed out");
    let preview: String = markdown.chars().take(900).collect();
    println!("\n===== DATALAB MARKDOWN ({} chars) =====\n{preview}\n=========================================\n", markdown.len());
    assert!(!markdown.trim().is_empty(), "markdown should not be empty");
}

#[tokio::test]
#[ignore = "hits the live Rev.ai API; run with REVAI_API_KEY set"]
async fn revai_transcribe_live() {
    let key = std::env::var("REVAI_API_KEY").expect("set REVAI_API_KEY");
    let path = env_or("TEST_AUDIO", "/tmp/toolkit-test/sample.wav");
    let client = reqwest::Client::new();

    let submitted = providers::revai_submit(&client, &key, &path)
        .await
        .expect("revai submit");

    let mut transcript = None;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_secs(5)).await;
        match providers::revai_poll(&client, &key, &submitted.remote_id)
            .await
            .expect("revai poll")
        {
            PollResult::Done(t) => {
                transcript = Some(t);
                break;
            }
            PollResult::Failed(e) => panic!("revai failed: {e}"),
            PollResult::Pending => {}
        }
    }

    let transcript = transcript.expect("revai timed out");
    println!("\n===== REV.AI TRANSCRIPT ({} chars) =====\n{transcript}\n=========================================\n", transcript.len());
    assert!(
        !transcript.trim().is_empty(),
        "transcript should not be empty"
    );
}
