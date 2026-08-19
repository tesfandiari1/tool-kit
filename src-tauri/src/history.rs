//! Persistent run history, in a SQLite file beside `settings.json`.
//!
//! Three jobs, and the second is why this is a database rather than a log file:
//!
//! 1. Remember where every result went, so finished work can be found again.
//! 2. Answer "has this file already been done?" on **every** input scan — every
//!    drop, every job switch. That is an indexed lookup by source path.
//! 3. Remember accepted backend work across an app restart, without storing
//!    credentials or document content.
//!
//! **History is a convenience and must never break a job.** Every entry point
//! that a job touches swallows storage errors: a conversion that succeeded is
//! still a success even if we failed to write the row.
//!
//! All SQL lives here. Nothing else in the app opens the database.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension as _};
use serde::Serialize;
use tauri::{AppHandle, Manager};

/// Rows kept before the oldest are trimmed. Big enough to be a real archive,
/// small enough that the file can't grow without bound.
const MAX_ENTRIES: i64 = 5_000;

/// Bump when the schema changes, and add a matching `if version < N` block in
/// `migrate` — so a new column is a migration rather than a crash on startup.
const SCHEMA_VERSION: i64 = 4;

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

/// One backend conversion to remember before its submit request is sent.
///
/// The source identity and mtime are derived here rather than accepted from a
/// caller, so recovery never trusts stale metadata supplied over another API.
pub struct NewInFlight<'a> {
    pub source_path: &'a str,
    pub file_name: &'a str,
    pub output_dir: &'a str,
    /// Exact service origin used for submit and every recovery request.
    pub backend_url: &'a str,
    pub client_run_id: &'a str,
    pub idempotency_key: &'a str,
    pub conversion_profile: &'a str,
}

/// A backend conversion that can be recovered after an app restart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InFlightEntry {
    /// Canonical source path captured immediately before submit.
    pub source_path: String,
    pub file_name: String,
    pub output_dir: String,
    /// Exact service origin this idempotency key belongs to.
    pub backend_url: String,
    pub client_run_id: String,
    /// The stable per-file key and primary identity of this record.
    pub idempotency_key: String,
    /// `None` until the backend accepts or replays the submission.
    pub backend_job_id: Option<String>,
    /// `Some` once a remote fallback has been started for this file. Written
    /// before the submit request, so an unknown outcome is still visible.
    pub fallback_provider: Option<String>,
    /// `Some` only once the provider accepted. Provider set with this `None`
    /// means the submission may already have been billed.
    pub fallback_request_id: Option<String>,
    pub fallback_check_url: Option<String>,
    pub conversion_profile: String,
    /// Source modification time in Unix milliseconds at submit time.
    pub source_mtime: i64,
    /// Unix seconds.
    pub created_at: i64,
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
    if version < 2 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS inflight_conversions (
                 idempotency_key    TEXT    PRIMARY KEY NOT NULL,
                 source_path       TEXT    NOT NULL,
                 file_name         TEXT    NOT NULL,
                 output_dir        TEXT    NOT NULL,
                 client_run_id     TEXT    NOT NULL,
                 backend_job_id    TEXT,
                 conversion_profile TEXT   NOT NULL,
                 source_mtime      INTEGER NOT NULL,
                 created_at        INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS inflight_created
                 ON inflight_conversions(created_at, idempotency_key);",
        )?;
    }
    if version < 3 {
        // Version 2 was never released. Its rows have no trustworthy service
        // origin, so the empty default makes them non-recoverable rather than
        // silently binding them to whichever URL is configured on next start.
        conn.execute_batch(
            "ALTER TABLE inflight_conversions
                 ADD COLUMN backend_url TEXT NOT NULL DEFAULT '';",
        )?;
    }
    if version < 4 {
        // Remember the remote fallback before it is paid for. Without these,
        // a restart mid-fallback resubmitted the file and billed it twice.
        conn.execute_batch(
            "ALTER TABLE inflight_conversions ADD COLUMN fallback_provider TEXT;
             ALTER TABLE inflight_conversions ADD COLUMN fallback_request_id TEXT;
             ALTER TABLE inflight_conversions ADD COLUMN fallback_check_url TEXT;",
        )?;
    }
    // Add `if version < 5 { … }` above when the schema changes again, then bump
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

/// Resolve the exact source identity and mtime that a restart must verify.
/// If either lookup fails, the caller skips persistence rather than recording
/// a recovery row that could not be validated safely later.
fn source_identity(path: &str) -> Option<(String, i64)> {
    let canonical = std::fs::canonicalize(path).ok()?;
    let source_path = canonical.to_string_lossy().into_owned();
    let source_mtime = mtime_ms(&source_path)?;
    Some((source_path, source_mtime))
}

// --- In-flight backend conversions -----------------------------------------

/// Best-effort insert before submit. Reusing a stable idempotency key preserves
/// the original recovery identity, attached backend job, and creation time.
/// Returns `false` when persistence is unavailable or the source cannot be
/// identified safely; that must never stop the conversion itself.
#[allow(dead_code, reason = "the M6 jobs integration lands in a later patch")]
pub fn upsert_in_flight(app: &AppHandle, pending: &NewInFlight<'_>) -> bool {
    if pending.idempotency_key.trim().is_empty()
        || pending.backend_url.trim().is_empty()
        || pending.client_run_id.trim().is_empty()
        || pending.conversion_profile.trim().is_empty()
    {
        return false;
    }
    with_db(app, |conn| upsert_in_flight_row(conn, pending)).unwrap_or(false)
}

fn upsert_in_flight_row(conn: &Connection, pending: &NewInFlight<'_>) -> rusqlite::Result<bool> {
    let Some((source_path, source_mtime)) = source_identity(pending.source_path) else {
        return Ok(false);
    };
    let changed = conn.execute(
        "INSERT INTO inflight_conversions
           (idempotency_key, source_path, file_name, output_dir, backend_url,
            client_run_id, backend_job_id, conversion_profile, source_mtime,
            created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, ?9)
         ON CONFLICT(idempotency_key) DO NOTHING",
        rusqlite::params![
            pending.idempotency_key,
            source_path,
            pending.file_name,
            pending.output_dir,
            pending.backend_url,
            pending.client_run_id,
            pending.conversion_profile,
            source_mtime,
            now_secs(),
        ],
    )?;
    Ok(changed == 1 || in_flight_matches(conn, pending, &source_path, source_mtime)?)
}

fn in_flight_matches(
    conn: &Connection,
    pending: &NewInFlight<'_>,
    source_path: &str,
    source_mtime: i64,
) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM inflight_conversions
              WHERE idempotency_key = ?1
                AND source_path = ?2
                AND file_name = ?3
                AND output_dir = ?4
                AND backend_url = ?5
                AND client_run_id = ?6
                AND conversion_profile = ?7
                AND source_mtime = ?8
         )",
        rusqlite::params![
            pending.idempotency_key,
            source_path,
            pending.file_name,
            pending.output_dir,
            pending.backend_url,
            pending.client_run_id,
            pending.conversion_profile,
            source_mtime,
        ],
        |row| row.get(0),
    )
}

/// Attach the backend UUID returned by either an accepted or replayed submit.
/// The same UUID may be attached repeatedly; a conflicting UUID is rejected so
/// a replay cannot silently replace the job that the durable key identifies.
#[allow(dead_code, reason = "the M6 jobs integration lands in a later patch")]
pub fn attach_backend_job(app: &AppHandle, idempotency_key: &str, backend_job_id: &str) -> bool {
    if idempotency_key.trim().is_empty() || backend_job_id.trim().is_empty() {
        return false;
    }
    with_db(app, |conn| {
        attach_backend_job_row(conn, idempotency_key, backend_job_id)
    })
    .unwrap_or(false)
}

fn attach_backend_job_row(
    conn: &Connection,
    idempotency_key: &str,
    backend_job_id: &str,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE inflight_conversions
            SET backend_job_id = ?2
          WHERE idempotency_key = ?1
            AND (backend_job_id IS NULL OR backend_job_id = ?2)",
        rusqlite::params![idempotency_key, backend_job_id],
    )?;
    Ok(changed == 1)
}

/// List restart-recoverable records in stable creation order.
///
/// `None` means storage was unavailable or unreadable; `Some([])` honestly
/// means the store was read and contains no active backend conversions.
#[allow(dead_code, reason = "the M6 jobs integration lands in a later patch")]
pub fn list_in_flight(app: &AppHandle) -> Option<Vec<InFlightEntry>> {
    with_db(app, select_in_flight)
}

fn select_in_flight(conn: &Connection) -> rusqlite::Result<Vec<InFlightEntry>> {
    let mut stmt = conn.prepare(
        "SELECT source_path, file_name, output_dir, backend_url,
                client_run_id, idempotency_key, backend_job_id,
                fallback_provider, fallback_request_id, fallback_check_url,
                conversion_profile, source_mtime, created_at
           FROM inflight_conversions
          ORDER BY created_at ASC, idempotency_key ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(InFlightEntry {
            source_path: row.get(0)?,
            file_name: row.get(1)?,
            output_dir: row.get(2)?,
            backend_url: row.get(3)?,
            client_run_id: row.get(4)?,
            idempotency_key: row.get(5)?,
            backend_job_id: row.get(6)?,
            fallback_provider: row.get(7)?,
            fallback_request_id: row.get(8)?,
            fallback_check_url: row.get(9)?,
            conversion_profile: row.get(10)?,
            source_mtime: row.get(11)?,
            created_at: row.get(12)?,
        })
    })?;
    rows.collect()
}

/// Record that a remote fallback is about to start, before the request goes
/// out. A row with a provider and no request id means the outcome is unknown
/// and may already have been billed, so recovery must never resubmit it.
pub fn begin_fallback(app: &AppHandle, idempotency_key: &str, provider: &str) -> bool {
    if idempotency_key.trim().is_empty() || provider.trim().is_empty() {
        return false;
    }
    with_db(app, |conn| {
        Ok(conn.execute(
            "UPDATE inflight_conversions
                SET fallback_provider = ?2
              WHERE idempotency_key = ?1",
            rusqlite::params![idempotency_key, provider],
        )? == 1)
    })
    .unwrap_or(false)
}

/// Attach the accepted remote request so a restart resumes polling it.
pub fn attach_fallback_request(
    app: &AppHandle,
    idempotency_key: &str,
    request_id: &str,
    check_url: &str,
) -> bool {
    if idempotency_key.trim().is_empty() || request_id.trim().is_empty() {
        return false;
    }
    with_db(app, |conn| {
        Ok(conn.execute(
            "UPDATE inflight_conversions
                SET fallback_request_id = ?2, fallback_check_url = ?3
              WHERE idempotency_key = ?1
                AND (fallback_request_id IS NULL OR fallback_request_id = ?2)",
            rusqlite::params![idempotency_key, request_id, check_url],
        )? == 1)
    })
    .unwrap_or(false)
}

/// Delete one terminal or explicitly stopped conversion by its stable key.
/// Unknown keys and unavailable storage both return `false`; no other row is
/// cleaned up as a side effect.
#[allow(dead_code, reason = "the M6 jobs integration lands in a later patch")]
pub fn delete_in_flight(app: &AppHandle, idempotency_key: &str) -> bool {
    if idempotency_key.trim().is_empty() {
        return false;
    }
    with_db(app, |conn| delete_in_flight_row(conn, idempotency_key)).unwrap_or(false)
}

/// Recovery is safe only while the canonical source and its modification time
/// still identify the exact file that was submitted originally.
pub fn in_flight_source_is_current(entry: &InFlightEntry) -> bool {
    source_identity(&entry.source_path)
        .is_some_and(|identity| identity == (entry.source_path.clone(), entry.source_mtime))
}

/// `None` means the store is unavailable or the key was not durably written;
/// callers preserve history's best-effort policy in either case. `Some(false)`
/// is authoritative and must stop a replay from submitting changed bytes under
/// the original idempotency key.
pub fn in_flight_source_for_key_is_current(app: &AppHandle, idempotency_key: &str) -> Option<bool> {
    if idempotency_key.trim().is_empty() {
        return None;
    }
    with_db(app, |conn| {
        conn.query_row(
            "SELECT source_path, source_mtime
               FROM inflight_conversions
              WHERE idempotency_key = ?1",
            [idempotency_key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
    })
    .flatten()
    .map(|(source_path, source_mtime): (String, i64)| {
        source_identity(&source_path)
            .is_some_and(|identity| identity == (source_path, source_mtime))
    })
}

fn delete_in_flight_row(conn: &Connection, idempotency_key: &str) -> rusqlite::Result<bool> {
    conn.execute(
        "DELETE FROM inflight_conversions WHERE idempotency_key = ?1",
        [idempotency_key],
    )
    .map(|changed| changed == 1)
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

    const BACKEND_URL: &str = "http://127.0.0.1:8473";

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

    fn create_v1_schema(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE history (
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
             CREATE INDEX history_source ON history(source_path);
             CREATE INDEX history_finished ON history(finished_at DESC);
             PRAGMA user_version = 1;",
        )
        .unwrap();
    }

    fn create_v2_schema(conn: &Connection) {
        create_v1_schema(conn);
        conn.execute_batch(
            "CREATE TABLE inflight_conversions (
                 idempotency_key    TEXT    PRIMARY KEY NOT NULL,
                 source_path       TEXT    NOT NULL,
                 file_name         TEXT    NOT NULL,
                 output_dir        TEXT    NOT NULL,
                 client_run_id     TEXT    NOT NULL,
                 backend_job_id    TEXT,
                 conversion_profile TEXT   NOT NULL,
                 source_mtime      INTEGER NOT NULL,
                 created_at        INTEGER NOT NULL
             );
             CREATE INDEX inflight_created
                 ON inflight_conversions(created_at, idempotency_key);
             PRAGMA user_version = 2;",
        )
        .unwrap();
    }

    #[test]
    fn v1_migrates_without_losing_history() {
        let conn = Connection::open_in_memory().unwrap();
        create_v1_schema(&conn);
        conn.execute(
            "INSERT INTO history
               (file_name, source_path, job_type, output_format, status, finished_at)
             VALUES ('kept.pdf', '/kept.pdf', 'convert', 'markdown', 'done', 42)",
            [],
        )
        .unwrap();

        migrate(&conn).unwrap();

        let rows = select(&conn, "", 50).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].file_name, "kept.pdf");
        let active_table: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                  WHERE type = 'table' AND name = 'inflight_conversions'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active_table, 1);
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn v2_migrates_without_rebinding_existing_rows() {
        let conn = Connection::open_in_memory().unwrap();
        create_v2_schema(&conn);
        let (src, out) = pair("v2-migration");
        log_done(&conn, &src, &out);
        let source_path = fs::canonicalize(&src)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let output_dir = out.parent().unwrap().to_string_lossy().into_owned();
        let source_mtime = mtime_ms(&source_path).unwrap();
        conn.execute(
            "INSERT INTO inflight_conversions
               (idempotency_key, source_path, file_name, output_dir,
                client_run_id, backend_job_id, conversion_profile,
                source_mtime, created_at)
             VALUES (?1, ?2, 'report.pdf', ?3, ?4, ?5, 'standard', ?6, 42)",
            rusqlite::params![
                "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
                source_path,
                output_dir,
                "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
                "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
                source_mtime,
            ],
        )
        .unwrap();

        migrate(&conn).unwrap();

        assert_eq!(select(&conn, "", 50).unwrap().len(), 1);
        let rows = select_in_flight(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source_path, source_path);
        assert_eq!(rows[0].output_dir, output_dir);
        assert_eq!(rows[0].backend_url, "");
        assert_eq!(
            rows[0].backend_job_id.as_deref(),
            Some("cccccccc-cccc-4ccc-8ccc-cccccccccccc")
        );
        let pending = NewInFlight {
            source_path: &source_path,
            file_name: "report.pdf",
            output_dir: &output_dir,
            backend_url: BACKEND_URL,
            client_run_id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
            idempotency_key: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
            conversion_profile: "standard",
        };
        assert!(
            !upsert_in_flight_row(&conn, &pending).unwrap(),
            "an origin-unknown v2 row must not bind to the current service"
        );
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn in_flight_record_attach_list_and_delete_round_trip() {
        let conn = db();
        let (src, out) = pair("inflight-roundtrip");
        let source_path = src.to_string_lossy().into_owned();
        let output_dir = out.parent().unwrap().to_string_lossy().into_owned();
        let pending = NewInFlight {
            source_path: &source_path,
            file_name: "report.pdf",
            output_dir: &output_dir,
            backend_url: BACKEND_URL,
            client_run_id: "11111111-1111-4111-8111-111111111111",
            idempotency_key: "22222222-2222-4222-8222-222222222222",
            conversion_profile: "standard",
        };

        assert!(upsert_in_flight_row(&conn, &pending).unwrap());
        // Retrying the pre-submit write keeps the one durable identity.
        assert!(upsert_in_flight_row(&conn, &pending).unwrap());
        let before_attach = select_in_flight(&conn).unwrap();
        assert_eq!(before_attach.len(), 1);
        assert_eq!(before_attach[0].backend_job_id, None);
        assert_eq!(before_attach[0].backend_url, BACKEND_URL);
        assert_eq!(before_attach[0].client_run_id, pending.client_run_id);
        assert_eq!(before_attach[0].idempotency_key, pending.idempotency_key);
        assert_eq!(before_attach[0].conversion_profile, "standard");
        assert!(before_attach[0].created_at > 0);

        let backend_job_id = "33333333-3333-4333-8333-333333333333";
        assert!(attach_backend_job_row(&conn, pending.idempotency_key, backend_job_id).unwrap());
        assert!(attach_backend_job_row(&conn, pending.idempotency_key, backend_job_id).unwrap());
        assert!(!attach_backend_job_row(
            &conn,
            pending.idempotency_key,
            "44444444-4444-4444-8444-444444444444"
        )
        .unwrap());
        assert_eq!(
            select_in_flight(&conn).unwrap()[0]
                .backend_job_id
                .as_deref(),
            Some(backend_job_id)
        );

        assert!(delete_in_flight_row(&conn, pending.idempotency_key).unwrap());
        assert!(select_in_flight(&conn).unwrap().is_empty());
        assert!(!delete_in_flight_row(&conn, pending.idempotency_key).unwrap());
    }

    #[test]
    fn an_idempotency_key_cannot_move_to_another_backend() {
        let conn = db();
        let (src, out) = pair("inflight-backend-binding");
        let source_path = src.to_string_lossy().into_owned();
        let output_dir = out.parent().unwrap().to_string_lossy().into_owned();
        let original = NewInFlight {
            source_path: &source_path,
            file_name: "report.pdf",
            output_dir: &output_dir,
            backend_url: BACKEND_URL,
            client_run_id: "12121212-1212-4212-8212-121212121212",
            idempotency_key: "34343434-3434-4434-8434-343434343434",
            conversion_profile: "standard",
        };
        assert!(upsert_in_flight_row(&conn, &original).unwrap());

        let changed_service = NewInFlight {
            backend_url: "http://127.0.0.1:9473",
            ..original
        };
        assert!(!upsert_in_flight_row(&conn, &changed_service).unwrap());
        let rows = select_in_flight(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].backend_url, BACKEND_URL);
    }

    #[test]
    fn in_flight_identity_is_canonical_and_captures_source_mtime() {
        let conn = db();
        let root = std::env::temp_dir().join("toolkit-hist-inflight-identity");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("nested")).unwrap();
        let src = root.join("report.pdf");
        fs::write(&src, b"pdf").unwrap();
        let alias = root.join("nested").join("..").join("report.pdf");
        let alias_path = alias.to_string_lossy().into_owned();
        let output_dir = root.to_string_lossy().into_owned();
        let pending = NewInFlight {
            source_path: &alias_path,
            file_name: "report.pdf",
            output_dir: &output_dir,
            backend_url: BACKEND_URL,
            client_run_id: "55555555-5555-4555-8555-555555555555",
            idempotency_key: "66666666-6666-4666-8666-666666666666",
            conversion_profile: "local_only",
        };

        assert!(upsert_in_flight_row(&conn, &pending).unwrap());
        let row = select_in_flight(&conn).unwrap().remove(0);
        assert_eq!(
            row.source_path,
            fs::canonicalize(&src).unwrap().to_string_lossy()
        );
        assert_eq!(row.source_mtime, mtime_ms(&row.source_path).unwrap());
        assert!(in_flight_source_is_current(&row));
        fs::remove_file(&src).unwrap();
        assert!(!in_flight_source_is_current(&row));
    }

    #[test]
    fn in_flight_records_survive_reopen() {
        let dir = std::env::temp_dir().join("toolkit-hist-inflight-reopen");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.db");
        let src = dir.join("report.pdf");
        fs::write(&src, b"pdf").unwrap();
        let source_path = src.to_string_lossy().into_owned();
        let output_dir = dir.to_string_lossy().into_owned();
        let pending = NewInFlight {
            source_path: &source_path,
            file_name: "report.pdf",
            output_dir: &output_dir,
            backend_url: BACKEND_URL,
            client_run_id: "77777777-7777-4777-8777-777777777777",
            idempotency_key: "88888888-8888-4888-8888-888888888888",
            conversion_profile: "standard",
        };

        {
            let conn = open(&path).unwrap();
            assert!(upsert_in_flight_row(&conn, &pending).unwrap());
            assert!(attach_backend_job_row(
                &conn,
                pending.idempotency_key,
                "99999999-9999-4999-8999-999999999999"
            )
            .unwrap());
        }

        let conn = open(&path).unwrap();
        let rows = select_in_flight(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].idempotency_key, pending.idempotency_key);
        assert_eq!(rows[0].backend_url, BACKEND_URL);
        assert_eq!(
            rows[0].backend_job_id.as_deref(),
            Some("99999999-9999-4999-8999-999999999999")
        );
    }

    #[test]
    fn newer_schema_version_is_not_rewritten() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE future_data (value TEXT NOT NULL);
             INSERT INTO future_data (value) VALUES ('keep me');
             PRAGMA user_version = 99;",
        )
        .unwrap();

        migrate(&conn).unwrap();

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        let value: String = conn
            .query_row("SELECT value FROM future_data", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 99);
        assert_eq!(value, "keep me");
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

    /// Written before the request goes out, so the outcome of an interrupted
    /// submit is recoverable rather than invisible. Without this a restart
    /// resubmitted the file and billed it twice.
    #[test]
    fn a_fallback_is_recorded_before_its_request_is_accepted() {
        let conn = db();
        let (src, _out) = pair("fallback-ledger");
        let source_path = fs::canonicalize(&src)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let output_dir = src.parent().unwrap().to_string_lossy().into_owned();
        let key = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
        upsert_in_flight_row(
            &conn,
            &NewInFlight {
                source_path: &source_path,
                file_name: "report.pdf",
                output_dir: &output_dir,
                backend_url: "http://127.0.0.1:8080",
                client_run_id: "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee",
                idempotency_key: key,
                conversion_profile: "standard",
            },
        )
        .unwrap();

        let row = |conn: &Connection| select_in_flight(conn).unwrap().remove(0);
        assert_eq!(row(&conn).fallback_provider, None);

        conn.execute(
            "UPDATE inflight_conversions SET fallback_provider = 'datalab'
              WHERE idempotency_key = ?1",
            [key],
        )
        .unwrap();
        let started = row(&conn);
        assert_eq!(started.fallback_provider.as_deref(), Some("datalab"));
        assert_eq!(
            started.fallback_request_id, None,
            "a provider with no request id is the uncertain state recovery must refuse to resubmit"
        );

        conn.execute(
            "UPDATE inflight_conversions
                SET fallback_request_id = 'req-1', fallback_check_url = 'https://example.test/1'
              WHERE idempotency_key = ?1",
            [key],
        )
        .unwrap();
        let accepted = row(&conn);
        assert_eq!(accepted.fallback_request_id.as_deref(), Some("req-1"));
        assert_eq!(
            accepted.fallback_check_url.as_deref(),
            Some("https://example.test/1")
        );
    }

    #[test]
    fn v3_rows_gain_empty_fallback_columns() {
        let conn = Connection::open_in_memory().unwrap();
        create_v2_schema(&conn);
        conn.execute_batch(
            "ALTER TABLE inflight_conversions
                 ADD COLUMN backend_url TEXT NOT NULL DEFAULT '';
             PRAGMA user_version = 3;",
        )
        .unwrap();
        let (src, _out) = pair("v3-migration");
        let source_path = fs::canonicalize(&src)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        conn.execute(
            "INSERT INTO inflight_conversions
               (idempotency_key, source_path, file_name, output_dir, backend_url,
                client_run_id, conversion_profile, source_mtime, created_at)
             VALUES (?1, ?2, 'report.pdf', ?3, 'http://127.0.0.1:8080', ?4, 'standard', 7, 42)",
            rusqlite::params![
                "ffffffff-ffff-4fff-8fff-ffffffffffff",
                source_path,
                src.parent().unwrap().to_string_lossy(),
                "99999999-9999-4999-8999-999999999999",
            ],
        )
        .unwrap();

        migrate(&conn).unwrap();

        let rows = select_in_flight(&conn).unwrap();
        assert_eq!(rows.len(), 1, "an existing recovery row must survive");
        assert_eq!(rows[0].fallback_provider, None);
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
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
