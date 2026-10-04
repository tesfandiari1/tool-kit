//! Apple's on-device speech model, checked and fetched ahead of a Transcribe
//! job. The audio worker does both, because only Swift reaches
//! `AssetInventory`. It runs with a cleared environment, as the converter runs
//! it, so it resolves the same locale a job will.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

const AUDIO_WORKER_BIN: &str = "tool-kit-audio-worker";
const STATUS_TIMEOUT: Duration = Duration::from_secs(20);
/// Offline, the OS waits for the network rather than failing, so the install
/// needs an end of its own.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum SpeechModelState {
    Installed,
    Downloading,
    Supported,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SpeechModel {
    state: SpeechModelState,
    /// BCP 47.
    locale: String,
    /// The locale's English name, for the Settings copy.
    language: String,
    /// What "Same as this Mac" resolves to.
    system_locale: String,
    system_language: String,
    /// Every language Apple offers, the ones on this Mac first.
    choices: Vec<SpeechChoice>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SpeechChoice {
    locale: String,
    language: String,
    installed: bool,
}

/// It lands in the worker's environment, so only BCP 47's own characters pass,
/// the rule the converter applies to `speechLocale`.
fn valid_locale(locale: &str) -> bool {
    (2..=35).contains(&locale.len())
        && locale
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
}

#[derive(Debug, PartialEq)]
enum Line {
    Progress(f64),
    Model(SpeechModel),
    Other,
}

fn parse_line(line: &str) -> Line {
    if let Some(done) = line.strip_prefix("progress ") {
        return done.parse().map_or(Line::Other, Line::Progress);
    }
    serde_json::from_str(line).map_or(Line::Other, Line::Model)
}

/// `None` on a build with no audio worker, which offers no Transcribe either.
fn worker() -> Option<PathBuf> {
    let path = std::env::current_exe()
        .ok()?
        .with_file_name(AUDIO_WORKER_BIN);
    path.is_file().then_some(path)
}

async fn run_worker(
    app: Option<&AppHandle>,
    verb: &str,
    locale: Option<&str>,
    limit: Duration,
) -> Result<Option<SpeechModel>, String> {
    let Some(worker) = worker() else {
        return Ok(None);
    };
    if locale.is_some_and(|locale| !valid_locale(locale)) {
        return Err("That language is not one Transcribe knows.".to_string());
    }
    let mut child = Command::new(worker)
        .args(["--speech", verb])
        .env_clear()
        // Empty for the Mac's own language, as a job sends it.
        .env("TOOLKIT_AUDIO_WORKER_LOCALE", locale.unwrap_or_default())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Could not start the audio worker: {e}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or("The audio worker has no output to read.")?;

    let read = async {
        let mut lines = BufReader::new(stdout).lines();
        let mut model = None;
        while let Ok(Some(line)) = lines.next_line().await {
            match parse_line(&line) {
                Line::Progress(done) => {
                    if let Some(app) = app {
                        let _ = app.emit("speech-model-progress", done);
                    }
                }
                Line::Model(found) => model = Some(found),
                Line::Other => {}
            }
        }
        model
    };
    let Ok(model) = tokio::time::timeout(limit, read).await else {
        return Err(
            "The speech model did not finish downloading. Check the internet connection and try again."
                .to_string(),
        );
    };
    let output = child
        .wait_with_output()
        .await
        .map_err(|e| format!("The audio worker did not exit: {e}"))?;
    match model {
        Some(model) if output.status.success() => Ok(Some(model)),
        _ => {
            // A clean exit with no report is a worker older than this host.
            eprintln!(
                "speech model {verb} failed ({}, report {}): {}",
                output.status,
                if model.is_some() { "read" } else { "missing" },
                String::from_utf8_lossy(&output.stderr).trim()
            );
            Err(if verb == "install" {
                "The speech model did not download. Check the internet connection and try again."
            } else {
                "Could not check the speech model."
            }
            .to_string())
        }
    }
}

/// `locale` is the Settings value, `None` for the Mac's own language.
#[tauri::command]
pub(crate) async fn speech_model_status(
    locale: Option<String>,
) -> Result<Option<SpeechModel>, String> {
    run_worker(None, "status", locale.as_deref(), STATUS_TIMEOUT).await
}

/// Progress goes out as `speech-model-progress`, 0 to 1. The OS merges
/// duplicate requests, so a second call while one runs downloads nothing twice.
#[tauri::command]
pub(crate) async fn download_speech_model(
    app: AppHandle,
    locale: Option<String>,
) -> Result<Option<SpeechModel>, String> {
    run_worker(Some(&app), "install", locale.as_deref(), INSTALL_TIMEOUT).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_worker_lines() {
        assert_eq!(parse_line("progress 0.5"), Line::Progress(0.5));
        assert_eq!(parse_line("progress soon"), Line::Other);
        assert_eq!(
            parse_line(
                r#"{"state":"supported","locale":"fr-FR","language":"French (France)","systemLocale":"en-US","systemLanguage":"English (United States)","choices":[{"locale":"fr-FR","language":"French (France)","installed":false}]}"#
            ),
            Line::Model(SpeechModel {
                state: SpeechModelState::Supported,
                locale: "fr-FR".to_string(),
                language: "French (France)".to_string(),
                system_locale: "en-US".to_string(),
                system_language: "English (United States)".to_string(),
                choices: vec![SpeechChoice {
                    locale: "fr-FR".to_string(),
                    language: "French (France)".to_string(),
                    installed: false,
                }],
            })
        );
        assert_eq!(parse_line(r#"{"state":"later"}"#), Line::Other);
    }

    #[test]
    fn only_a_language_tag_reaches_the_worker() {
        assert!(valid_locale("de-DE"));
        assert!(valid_locale("zh-Hant-TW"));
        assert!(!valid_locale("d"));
        assert!(!valid_locale("de DE"));
        assert!(!valid_locale("de\0DE"));
    }
}
