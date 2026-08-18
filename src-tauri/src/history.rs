//! Persistent run history, in a SQLite file beside `settings.json`.
//!
//! Two jobs, and the second is why this is a database rather than a log file:
//!
//! 1. Remember where every result went, so finished work can be found again.
//! 2. Answer "has this file already been done?" on **every** input scan — every
//!    drop, every job switch. That is an indexed lookup by source path.
//!
//! **History is a convenience and must never break a job.** Every entry point
//! that a job touches swallows storage errors: a conversion that succeeded is
//! still a success even if we failed to write the row.
//!
//! All SQL lives here. Nothing else in the app opens the database.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, Manager};

/// Rows kept before the oldest are trimmed. Big enough to be a real archive,
/// small enough that the file can't grow without bound.
const MAX_ENTRIES: i64 = 5_000;

/// Bump when the schema changes, and add a matching `if version < N` block in
/// `migrate` — so a new column is a migration rather than a crash on startup.
const SCHEMA_VERSION: i64 = 1;

/// The open database, or `None` if it could not be opened. `None` makes every
/// operation a silent no-op, which is the whole failure policy in one word.
pub struct History {
    db: Mutex<Option<Connection>>,
}

/// One finished job, on its way into the log.
pub struct Finished<'a> {
    pub file_name: &'a str,
    pub source_path: &'a str,
    pub output_path: Option<&'a str>,
    pub job_type: &'a str,
    pub output_format: &'a str,
    /// "done" | "failed".
    pub status: &'a str,
    pub error: Option<&'a str>,
}

/// One row, as the UI sees it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: i64,
    pub file_name: String,
    pub source_path: String,
    pub output_path: Option<String>,
    pub job_type: String,
    pub output_format: String,
    pub status: String,
    pub error: Option<String>,
    /// Unix seconds.
    pub finished_at: i64,
}

// --- Setup -----------------------------------------------------------------

/// Open (creating if needed) the history database in the app config dir.
/// Never fails hard: a broken database degrades to "no history".
pub fn init(app: &AppHandle) -> History {
    let db = app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())
        .and_then(|dir| {
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            open(&dir.join("history.db")).map_err(|e| e.to_string())
        });
    match db {
        Ok(conn) => History {
            db: Mutex::new(Some(conn)),
        },
        Err(e) => {
            eprintln!("[tool-kit] history unavailable: {e}");
            History {
                db: Mutex::new(None),
            }
        }
    }
}

fn open(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    // WAL because up to four job tasks finish concurrently (the semaphore in
    // jobs.rs) while a scan is reading. busy_timeout covers the rest.
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA busy_timeout = 5000;",
    )?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 1 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS history (
                 id            INTEGER PRIMARY KEY AUTOINCREMENT,
                 file_name     TEXT    NOT NULL,
                 source_path   TEXT    NOT NULL,
                 output_path   TEXT,
                 job_type      TEXT    NOT NULL,
                 output_format TEXT    NOT NULL,
                 status        TEXT    NOT NULL,
                 error         TEXT,
                 finished_at   INTEGER NOT NULL,
                 source_mtime  INTEGER
             );
             -- The already-done lookup. This index is the reason for a database.
             CREATE INDEX IF NOT EXISTS history_source ON history(source_path);
             CREATE INDEX IF NOT EXISTS history_finished ON history(finished_at DESC);",
        )?;
    }
    // Add `if version < 2 { … }` above when a column is added, then bump
    // SCHEMA_VERSION. A file written by a *newer* build is left untouched:
    // extra columns are harmless to read, and rewriting it would lose history.
    if version < SCHEMA_VERSION {
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    Ok(())
}

// --- Identity ---------------------------------------------------------------

/// The key a source file is filed under. Canonicalised on both write and read
/// so the same file reached two ways (a symlink, `/tmp` vs `/private/tmp`)
/// still matches itself. Falls back to the literal path when the file is gone,
/// which cannot cause a wrong "already done" — a missing source fails the
/// mtime check anyway.
fn key(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string())
}

/// Modification time in Unix milliseconds, or `None` if it cannot be read.
fn mtime_ms(path: &str) -> Option<i64> {
    let m = std::fs::metadata(path).ok()?.modified().ok()?;
    m.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// --- Writing ----------------------------------------------------------------

/// Log a finished job. Errors are swallowed on purpose — see the module note.
pub fn record(app: &AppHandle, f: &Finished) {
    with_db(app, |conn| {
        insert(conn, f)?;
        trim(conn)
    });
}

fn insert(conn: &Connection, f: &Finished) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO history
           (file_name, source_path, output_path, job_type, output_format,
            status, error, finished_at, source_mtime)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            f.file_name,
            key(f.source_path),
            f.output_path,
            f.job_type,
            f.output_format,
            f.status,
            f.error,
            now_secs(),
            mtime_ms(f.source_path),
        ],
    )?;
    Ok(())
}

fn trim(conn: &Connection) -> rusqlite::Result<()> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0))?;
    if n > MAX_ENTRIES {
        conn.execute(
            "DELETE FROM history WHERE id IN
               (SELECT id FROM history ORDER BY finished_at ASC, id ASC LIMIT ?1)",
            [n - MAX_ENTRIES],
        )?;
    }
    Ok(())
}

// --- The "already done" question ---------------------------------------------

/// Which of `sources` already have a usable result, and **where that result
/// is**, given the job and the output format that would be produced now.
/// Keyed by the source path exactly as passed in; the value is the output file.
///
/// A result is reusable when **all** of these hold:
///
/// - the same source path was processed before, **and**
/// - by the same job, **and**
/// - to the same output format, **and**
/// - that run succeeded and its output file **still exists on disk**, **and**
/// - the source's mtime is **unchanged** since it was processed.
///
/// Any one failing means it genuinely needs redoing: deleted the output → redo,
/// edited the source → redo, switched markdown → html → redo.
///
/// Note this says nothing about *which folder* the result is in — that is the
/// caller's business. A result in the folder the user picked means there is
/// nothing to do; one somewhere else can be copied instead of paid for again.
///
/// Deliberately **path-keyed, not content-hashed**. Hashing would survive
/// renames, but it means reading every byte of every file on every scan, which
/// is brutal for the video files this app is pointed at.
pub fn reusable(
    app: &AppHandle,
    sources: &[std::path::PathBuf],
    job_type: &str,
    output_format: &str,
) -> HashMap<String, String> {
    with_db(app, |conn| {
        Ok(reuse_map(conn, sources, job_type, output_format))
    })
    .unwrap_or_default()
}

fn reuse_map(
    conn: &Connection,
    sources: &[std::path::PathBuf],
    job_type: &str,
    output_format: &str,
) -> HashMap<String, String> {
    let mut stmt = match conn.prepare(
        "SELECT output_path, source_mtime FROM history
          WHERE source_path = ?1 AND job_type = ?2 AND output_format = ?3
            AND status = 'done' AND output_path IS NOT NULL
          ORDER BY finished_at DESC LIMIT 8",
    ) {
        Ok(s) => s,
        Err(_) => return HashMap::new(),
    };

    let mut found = HashMap::new();
    for source in sources {
        let raw = source.to_string_lossy().into_owned();
        // Read the source's mtime once. If we can't (the file vanished between
        // the scan and now), there is nothing to reuse.
        let Some(current) = mtime_ms(&raw) else {
            continue;
        };

        let rows = stmt.query_map(rusqlite::params![key(&raw), job_type, output_format], |r| {
            Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<i64>>(1)?))
        });
        let Ok(rows) = rows else { continue };

        // Any past run whose output survives and whose source is untouched
        // counts. Checking every candidate, not just the newest, means a
        // deleted newer output falls back to an older one that is still there.
        for row in rows.flatten() {
            let (Some(out), Some(then)) = row else {
                continue;
            };
            if then == current && Path::new(&out).is_file() {
                found.insert(raw.clone(), out);
                break;
            }
        }
    }
    found
}

/// Do these two paths name the same folder? Canonicalised, so a trailing
/// slash or a symlinked parent doesn't read as a different destination and
/// trigger a pointless copy.
pub fn same_dir(a: &Path, b: &Path) -> bool {
    let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    real(a) == real(b)
}

/// Is this result already sitting in the folder the user picked?
pub fn is_in_dir(output_path: &str, dir: &str) -> bool {
    Path::new(output_path)
        .parent()
        .is_some_and(|parent| same_dir(parent, Path::new(dir)))
}

// --- Reading ----------------------------------------------------------------

/// Newest first, optionally filtered by a substring of the file name or its
/// folder. An unreadable database reads as an empty history.
pub fn list(app: &AppHandle, query: &str, limit: u32) -> Vec<Entry> {
    with_db(app, |conn| select(conn, query, limit)).unwrap_or_default()
}

fn select(conn: &Connection, query: &str, limit: u32) -> rusqlite::Result<Vec<Entry>> {
    let limit = limit.clamp(1, 2_000) as i64;
    let read = |r: &rusqlite::Row| -> rusqlite::Result<Entry> {
        Ok(Entry {
            id: r.get(0)?,
            file_name: r.get(1)?,
            source_path: r.get(2)?,
            output_path: r.get(3)?,
            job_type: r.get(4)?,
            output_format: r.get(5)?,
            status: r.get(6)?,
            error: r.get(7)?,
            finished_at: r.get(8)?,
        })
    };
    const COLS: &str = "id, file_name, source_path, output_path, job_type,
                        output_format, status, error, finished_at";

    // One statement shape for both cases: an empty query becomes `%%`, which
    // matches every row, so listing and searching can't drift apart.
    let pattern = format!("%{}%", escape_like(query.trim()));
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM history
          WHERE file_name LIKE ?1 ESCAPE '\\' OR source_path LIKE ?1 ESCAPE '\\'
          ORDER BY finished_at DESC, id DESC LIMIT ?2"
    ))?;
    let rows = stmt
        .query_map(rusqlite::params![pattern, limit], read)?
        .collect::<rusqlite::Result<Vec<Entry>>>()?;
    Ok(rows)
}

/// `%` and `_` are wildcards in LIKE, so a search for "report_v2" must not
/// quietly match "reportXv2".
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Forget everything. The output files themselves are never touched.
pub fn clear(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<History>();
    let guard = state
        .db
        .lock()
        .map_err(|_| "History is locked".to_string())?;
    let conn = guard.as_ref().ok_or("History is unavailable")?;
    conn.execute("DELETE FROM history", [])
        .map(|_| ())
        .map_err(|e| e.to_string())?;
    // Give the space back rather than leaving a 5,000-row file behind.
    let _ = conn.execute_batch("VACUUM");
    Ok(())
}

/// Run `f` against the database, or return `None` if there isn't one. Every
/// error becomes `None`, which is the "history never breaks a job" rule.
fn with_db<T>(app: &AppHandle, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Option<T> {
    let state = app.state::<History>();
    let guard = state.db.lock().ok()?;
    let conn = guard.as_ref()?;
    match f(conn) {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("[tool-kit] history write failed: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn
    }

    /// A source file and the output it produced, both real on disk.
    fn pair(name: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("toolkit-hist-{name}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let src = root.join("report.pdf");
        let out = root.join("report.md");
        fs::write(&src, b"pdf").unwrap();
        fs::write(&out, b"# md").unwrap();
        (src, out)
    }

    fn log_done(conn: &Connection, src: &Path, out: &Path) {
        insert(
            conn,
            &Finished {
                file_name: "report.pdf",
                source_path: &src.to_string_lossy(),
                output_path: Some(&out.to_string_lossy()),
                job_type: "convert",
                output_format: "markdown",
                status: "done",
                error: None,
            },
        )
        .unwrap();
    }

    #[test]
    fn an_entry_round_trips() {
        let conn = db();
        let (src, out) = pair("roundtrip");
        log_done(&conn, &src, &out);

        let rows = select(&conn, "", 50).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].file_name, "report.pdf");
        assert_eq!(
            rows[0].output_path.as_deref(),
            Some(out.to_string_lossy().as_ref())
        );
        assert_eq!(rows[0].status, "done");
        assert!(rows[0].finished_at > 0);

        // ...and is findable by name and by folder.
        assert_eq!(select(&conn, "report", 50).unwrap().len(), 1);
        assert_eq!(
            select(&conn, "toolkit-hist-roundtrip", 50).unwrap().len(),
            1
        );
        assert_eq!(select(&conn, "nothing-like-this", 50).unwrap().len(), 0);
    }

    #[test]
    fn already_done_accepts_an_untouched_pair() {
        let conn = db();
        let (src, out) = pair("hit");
        log_done(&conn, &src, &out);
        let done = reuse_map(&conn, std::slice::from_ref(&src), "convert", "markdown");
        assert_eq!(
            done.len(),
            1,
            "an unchanged source with its output still there is done"
        );
    }

    #[test]
    fn already_done_rejects_a_deleted_output() {
        let conn = db();
        let (src, out) = pair("no-output");
        log_done(&conn, &src, &out);
        fs::remove_file(&out).unwrap();
        assert!(
            reuse_map(&conn, &[src], "convert", "markdown").is_empty(),
            "the result is gone, so the file needs redoing"
        );
    }

    #[test]
    fn already_done_rejects_an_edited_source() {
        let conn = db();
        let (src, out) = pair("edited");
        log_done(&conn, &src, &out);
        // Pretend the source was touched after it was processed. Doctoring the
        // stored mtime is deterministic; rewriting the file could land in the
        // same millisecond and flake.
        conn.execute("UPDATE history SET source_mtime = source_mtime - 1000", [])
            .unwrap();
        assert!(
            reuse_map(&conn, &[src], "convert", "markdown").is_empty(),
            "an edited source needs redoing"
        );
    }

    #[test]
    fn already_done_is_scoped_to_the_job_and_format() {
        let conn = db();
        let (src, out) = pair("scoped");
        log_done(&conn, &src, &out);
        assert!(
            reuse_map(&conn, std::slice::from_ref(&src), "convert", "html").is_empty(),
            "format switch"
        );
        assert!(
            reuse_map(&conn, std::slice::from_ref(&src), "transcribe", "markdown").is_empty(),
            "job switch"
        );
        assert_eq!(reuse_map(&conn, &[src], "convert", "markdown").len(), 1);
    }

    #[test]
    fn a_failed_run_is_never_already_done() {
        let conn = db();
        let (src, _out) = pair("failed");
        insert(
            &conn,
            &Finished {
                file_name: "report.pdf",
                source_path: &src.to_string_lossy(),
                output_path: None,
                job_type: "convert",
                output_format: "markdown",
                status: "failed",
                error: Some("upstream said no"),
            },
        )
        .unwrap();
        assert!(reuse_map(&conn, &[src], "convert", "markdown").is_empty());
        assert_eq!(
            select(&conn, "", 50).unwrap()[0].error.as_deref(),
            Some("upstream said no")
        );
    }

    /// The classic way to lose a user's history is a `migrate` that recreates
    /// the table on every open. Reopening must find the rows still there.
    #[test]
    fn a_database_on_disk_survives_a_reopen() {
        let dir = std::env::temp_dir().join("toolkit-hist-reopen");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.db");
        let (src, out) = pair("reopen-src");

        {
            let conn = open(&path).unwrap();
            log_done(&conn, &src, &out);
        }
        let conn = open(&path).unwrap();
        assert_eq!(
            select(&conn, "", 50).unwrap().len(),
            1,
            "reopening wiped the history"
        );

        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn retention_trims_the_oldest_first() {
        let conn = db();
        // Inserted raw, not through `insert`: 5,000 canonicalize+stat syscalls
        // would make this a slow test for no extra coverage.
        conn.execute_batch("BEGIN").unwrap();
        for i in 0..MAX_ENTRIES + 10 {
            conn.execute(
                "INSERT INTO history
                   (file_name, source_path, job_type, output_format, status, finished_at)
                 VALUES ('f', 'p', 'convert', 'markdown', 'done', ?1)",
                [i],
            )
            .unwrap();
        }
        conn.execute_batch("COMMIT").unwrap();

        trim(&conn).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, MAX_ENTRIES);
        let oldest: i64 = conn
            .query_row("SELECT MIN(finished_at) FROM history", [], |r| r.get(0))
            .unwrap();
        assert_eq!(oldest, 10, "the ten oldest should be the ones dropped");
    }

    /// The reuse decision splits on *where* the result is: in the chosen
    /// folder means nothing to do, anywhere else means copy it rather than pay
    /// for it twice.
    #[test]
    fn a_result_counts_as_here_only_in_its_own_folder() {
        let (_src, out) = pair("folder-match");
        let out_s = out.to_string_lossy().to_string();
        let dir = out.parent().unwrap().to_string_lossy().to_string();

        assert!(is_in_dir(&out_s, &dir));
        // A trailing slash names the same folder and must not force a copy.
        assert!(is_in_dir(&out_s, &format!("{dir}/")));
        assert!(!is_in_dir(&out_s, &std::env::temp_dir().to_string_lossy()));
        // A subfolder of the result's folder is a different destination.
        assert!(!is_in_dir(&out_s, &format!("{dir}/nested")));
    }

    #[test]
    fn search_wildcards_are_literal() {
        let conn = db();
        let (src, out) = pair("wildcard");
        log_done(&conn, &src, &out);
        // "%" must not match everything.
        assert_eq!(select(&conn, "%", 50).unwrap().len(), 0);
    }
}
