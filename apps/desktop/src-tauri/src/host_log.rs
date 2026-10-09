//! `tool-kit.log` beside `converter.log`: the host's own lines. A Finder launch
//! has no terminal, so `eprintln!` reached no one and a quit or a panic left
//! no trace.

use std::{fs::OpenOptions, io::Write as _, path::PathBuf, sync::OnceLock};

use time::{format_description::well_known::Rfc3339, OffsetDateTime};

const CAP_BYTES: u64 = 2 * 1024 * 1024;

static PATH: OnceLock<PathBuf> = OnceLock::new();

/// Points the log into `dir` and routes panics to it. Past the cap the file
/// starts over at launch, so it never grows without bound.
pub(crate) fn init(dir: PathBuf) {
    let path = dir.join("tool-kit.log");
    if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > CAP_BYTES) {
        let _ = std::fs::remove_file(&path);
    }
    let _ = PATH.set(path);
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        line(&format!("panic: {info}"));
        default_hook(info);
    }));
}

/// One timestamped line to the log, and to stderr for `pnpm tauri dev`.
pub(crate) fn line(message: &str) {
    eprintln!("[tool-kit] {message}");
    let Some(path) = PATH.get() else {
        return;
    };
    let stamp = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_default();
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        // A log write must never take the app down with it.
        let _ = writeln!(file, "{stamp} {message}");
    }
}
