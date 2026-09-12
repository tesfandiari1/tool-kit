//! Workspace foundation: the one user-chosen folder the library lives in.
//!
//! A project's identity is its `project.json`, and `.toolkit/index.db` is only
//! the fast lookup over those files, rebuilt by `rebuild_index` when gone.
//!
//! `.toolkit/workspace.json` is the one file that is not derived. Its absence
//! means the folder moved or unmounted, and every entry point here errors.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::settings;

/// The catch-all project. A result with no project of its own lands here.
pub const CATCH_ALL_TITLE: &str = "Drop Box";

/// The catch-all's former name, renamed on the next setup ahead of the mint
/// below: the mint keys on the folder name, so it would make a second one.
const LEGACY_CATCH_ALL_TITLE: &str = "Inbox";

/// What `setup_workspace` knows after creating or adopting a folder.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    pub workspace_path: String,
    pub workspace_id: String,
    pub catch_all_project_id: String,
    /// The catch-all's folder, workspace-relative. Where a drop is filed.
    pub catch_all_path: String,
    /// Set only when this call wrote the welcome file, which the host knows
    /// and the caller opens as a document tab.
    pub welcome_path: Option<String>,
}

/// One project, as the UI lists it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub id: String,
    pub title: String,
    /// Relative to the workspace root, so moving it does not stale the index.
    pub path: String,
    pub created_at: String,
}

/// `.toolkit/workspace.json`
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceMeta {
    schema_version: u32,
    id: String,
    created_at: String,
    /// Whether the welcome file was written. `default` so an older marker reads
    /// as "not yet" and gets the file once.
    #[serde(default)]
    welcome_seeded: bool,
}

/// `{Project}/project.json`
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectMeta {
    schema_version: u32,
    id: String,
    title: String,
    created_at: String,
}

/// The default offered by the first-launch picker: ~/Documents/Tool-Kit.
pub fn suggested_workspace_path() -> String {
    std::env::home_dir()
        .map(|home| home.join("Documents").join("Tool-Kit"))
        .unwrap_or_else(|| PathBuf::from("~/Documents/Tool-Kit"))
        .to_string_lossy()
        .into_owned()
}

/// Whether `path` already looks like a workspace. `.toolkit/workspace.json` is
/// the identity marker, and everything else is rebuildable.
pub fn inspect_workspace_path(path: &str) -> bool {
    Path::new(path)
        .join(".toolkit")
        .join("workspace.json")
        .is_file()
}

/// Create a workspace at `path`, or adopt one. A second call returns the same
/// ids.
pub fn setup_workspace(path: &str) -> Result<WorkspaceInfo, String> {
    let root = Path::new(path);
    let toolkit = root.join(".toolkit");
    std::fs::create_dir_all(&toolkit).map_err(|e| e.to_string())?;

    let workspace_file = toolkit.join("workspace.json");
    let mut workspace = if workspace_file.is_file() {
        read_json::<WorkspaceMeta>(&workspace_file)?
    } else {
        let meta = WorkspaceMeta {
            schema_version: 1,
            id: format!("w_{}", uuid::Uuid::new_v4()),
            created_at: iso_utc_now(),
            welcome_seeded: false,
        };
        write_json(&workspace_file, &meta)?;
        meta
    };

    let index = open_index(&toolkit.join("index.db"))?;
    // Before the mint below, never after. See LEGACY_CATCH_ALL_TITLE.
    migrate_catch_all(root, &index)?;

    // Created at first run. An existing one is adopted by its project.json.
    let catch_all_dir = root.join(CATCH_ALL_TITLE);
    std::fs::create_dir_all(&catch_all_dir).map_err(|e| e.to_string())?;
    let catch_all_file = catch_all_dir.join("project.json");
    let catch_all = if catch_all_file.is_file() {
        read_json::<ProjectMeta>(&catch_all_file)?
    } else {
        let meta = ProjectMeta {
            schema_version: 1,
            id: format!("p_{}", uuid::Uuid::new_v4()),
            title: CATCH_ALL_TITLE.into(),
            created_at: iso_utc_now(),
        };
        write_json(&catch_all_file, &meta)?;
        meta
    };

    // The marker gates this, not the catch-all mint: a folder renamed in Finder
    // mints a fresh one, and gating there resurrects a file the user deleted.
    // A write failure leaves the marker false, so the next launch retries.
    let welcome = catch_all_dir.join("welcome.md");
    let welcome_path = if workspace.welcome_seeded || welcome.exists() {
        None
    } else {
        std::fs::write(&welcome, include_str!("welcome.md"))
            .ok()
            .map(|_| welcome.to_string_lossy().into_owned())
    };

    // Record the seed once the file is there, so deleting it later is final.
    if !workspace.welcome_seeded && (welcome_path.is_some() || welcome.exists()) {
        workspace.welcome_seeded = true;
        let _ = write_json(&workspace_file, &workspace);
    }

    index
        .execute(
            "INSERT OR IGNORE INTO projects (id, title, path, created_at) \
             VALUES (?1, ?2, ?3, ?4)",
            (
                &catch_all.id,
                &catch_all.title,
                CATCH_ALL_TITLE,
                &catch_all.created_at,
            ),
        )
        .map_err(|e| e.to_string())?;

    Ok(WorkspaceInfo {
        workspace_path: path.to_string(),
        workspace_id: workspace.id,
        catch_all_project_id: catch_all.id,
        catch_all_path: CATCH_ALL_TITLE.to_string(),
        welcome_path,
    })
}

/// Rename a pre-existing `Inbox` to the catch-all's current name, once.
/// Identity is `project.json`, so the id, the date and every file survive.
/// A folder the user retitled by hand is theirs, and is left alone.
fn migrate_catch_all(root: &Path, index: &Connection) -> Result<(), String> {
    let legacy = root.join(LEGACY_CATCH_ALL_TITLE);
    let current = root.join(CATCH_ALL_TITLE);
    if current.exists() || !legacy.join("project.json").is_file() {
        return Ok(());
    }
    let mut meta = read_json::<ProjectMeta>(&legacy.join("project.json"))?;
    if !meta.title.eq_ignore_ascii_case(LEGACY_CATCH_ALL_TITLE) {
        return Ok(());
    }
    std::fs::rename(&legacy, &current).map_err(|e| e.to_string())?;
    meta.title = CATCH_ALL_TITLE.to_string();
    // The folder is already moved, and a failed write leaves a `Drop Box`
    // titled "Inbox" that the next launch skips.
    write_json(&current.join("project.json"), &meta)?;
    index
        .execute(
            "UPDATE projects SET title = ?1, path = ?2 WHERE id = ?3",
            (CATCH_ALL_TITLE, CATCH_ALL_TITLE, &meta.id),
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Follow the folder rename through the two settings that name it.
/// `active_project_path` is where a run writes, so a stale one costs the
/// destination silently. `expanded_paths` is the rows left open.
pub fn migrate_settings(app: &AppHandle) {
    let mut cfg = settings::load(app);
    let legacy_prefix = format!("{LEGACY_CATCH_ALL_TITLE}/");
    let renamed = |rel: &str| -> Option<String> {
        if rel == LEGACY_CATCH_ALL_TITLE {
            return Some(CATCH_ALL_TITLE.to_string());
        }
        rel.strip_prefix(&legacy_prefix)
            .map(|rest| format!("{CATCH_ALL_TITLE}/{rest}"))
    };

    let mut changed = false;
    if let Some(next) = cfg.active_project_path.as_deref().and_then(renamed) {
        cfg.active_project_path = Some(next);
        changed = true;
    }
    for path in &mut cfg.expanded_paths {
        if let Some(next) = renamed(path) {
            *path = next;
            changed = true;
        }
    }
    if changed {
        let _ = settings::save(app, &cfg);
    }
}

/// Creates a project folder under the workspace and registers it in the index.
pub fn create_project(app: &AppHandle, title: &str) -> Result<ProjectSummary, String> {
    let workspace = settings::load(app)
        .workspace_path
        .ok_or_else(|| "No workspace configured".to_string())?;
    create_project_at(Path::new(&workspace), title)
}

fn create_project_at(workspace: &Path, title: &str) -> Result<ProjectSummary, String> {
    let folder = folder_name_for_title(title)?;
    let toolkit = require_workspace(workspace)?;
    let project_dir = workspace.join(&folder);
    if project_dir.exists() {
        return Err(format!("A project named “{folder}” already exists"));
    }
    // Opened before anything lands on disk, or an index failure returns past
    // the rollbacks below and the orphan folder blocks that name for good.
    let index = open_index(&toolkit.join("index.db"))?;
    std::fs::create_dir_all(&project_dir).map_err(|e| e.to_string())?;

    let meta = ProjectMeta {
        schema_version: 1,
        id: format!("p_{}", uuid::Uuid::new_v4()),
        title: title.trim().to_string(),
        created_at: iso_utc_now(),
    };
    if let Err(e) = write_json(&project_dir.join("project.json"), &meta) {
        let _ = std::fs::remove_dir_all(&project_dir);
        return Err(e);
    }

    if let Err(e) = index.execute(
        "INSERT INTO projects (id, title, path, created_at) VALUES (?1, ?2, ?3, ?4)",
        (&meta.id, &meta.title, &folder, &meta.created_at),
    ) {
        let _ = std::fs::remove_dir_all(&project_dir);
        return Err(e.to_string());
    }

    Ok(ProjectSummary {
        id: meta.id,
        title: meta.title,
        path: folder,
        created_at: meta.created_at,
    })
}

fn folder_name_for_title(title: &str) -> Result<String, String> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err("Project name cannot be empty".into());
    }
    if trimmed.eq_ignore_ascii_case(CATCH_ALL_TITLE) {
        return Err(format!("{CATCH_ALL_TITLE} is reserved"));
    }
    // Still reserved: the migration above would rename an Inbox out from
    // under the user.
    if trimmed.eq_ignore_ascii_case(LEGACY_CATCH_ALL_TITLE) {
        return Err(format!("{LEGACY_CATCH_ALL_TITLE} is reserved"));
    }
    if trimmed == "." || trimmed == ".." {
        return Err("Project name cannot be . or ..".into());
    }
    if trimmed.starts_with('.') {
        return Err("Project name cannot start with .".into());
    }
    if trimmed.eq_ignore_ascii_case(".toolkit") {
        return Err(".toolkit is reserved".into());
    }
    if trimmed.chars().any(|c| matches!(c, '/' | ':' | '\0')) {
        return Err("Project name cannot contain / or :".into());
    }
    Ok(trimmed.to_string())
}

/// Every project in the workspace index, rebuilt first when the index is gone.
/// A missing workspace folder is an error.
pub fn list_projects(app: &AppHandle) -> Result<Vec<ProjectSummary>, String> {
    let workspace = settings::load(app)
        .workspace_path
        .ok_or_else(|| "No workspace configured".to_string())?;
    list_projects_at(Path::new(&workspace))
}

fn list_projects_at(workspace: &Path) -> Result<Vec<ProjectSummary>, String> {
    let toolkit = require_workspace(workspace)?;
    let db_path = toolkit.join("index.db");
    let missing = !db_path.is_file();
    let conn = open_index(&db_path)?;
    if missing {
        rebuild_index(workspace, &conn)?;
    }
    let mut stmt = conn
        .prepare("SELECT id, title, path, created_at FROM projects ORDER BY created_at, id")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ProjectSummary {
                id: row.get(0)?,
                title: row.get(1)?,
                path: row.get(2)?,
                created_at: row.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut projects = Vec::new();
    for row in rows {
        projects.push(row.map_err(|e| e.to_string())?);
    }
    Ok(projects)
}

/// The workspace's `.toolkit` directory, or an error naming the missing folder.
/// Without it `create_dir_all` resurrects an empty decoy at the old path.
fn require_workspace(workspace: &Path) -> Result<PathBuf, String> {
    let toolkit = workspace.join(".toolkit");
    if !toolkit.join("workspace.json").is_file() {
        return Err(format!(
            "Workspace not found at {}. Move it back or pick it again.",
            workspace.display()
        ));
    }
    Ok(toolkit)
}

/// Rebuild the index from the project folders themselves. A project's identity
/// is its `project.json`, so every row is recoverable from disk.
fn rebuild_index(workspace: &Path, index: &Connection) -> Result<(), String> {
    for entry in std::fs::read_dir(workspace).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.path().is_dir() {
            continue;
        }
        // A hand-made folder carries no project.json, so it is skipped rather
        // than failing the rebuild.
        let Ok(meta) = read_json::<ProjectMeta>(&entry.path().join("project.json")) else {
            continue;
        };
        index
            .execute(
                "INSERT OR IGNORE INTO projects (id, title, path, created_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                (
                    &meta.id,
                    &meta.title,
                    &entry.file_name().to_string_lossy().into_owned(),
                    &meta.created_at,
                ),
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn open_index(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS projects (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            path TEXT NOT NULL,
            created_at TEXT NOT NULL
        );",
    )
    .map_err(|e| e.to_string())?;
    Ok(conn)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes)
        .map_err(|e| format!("{} could not be parsed: {e}", path.display()))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

/// RFC 3339 UTC, without taking a chrono dependency for one call site.
fn iso_utc_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hour, min, sec) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Civil date from a day count (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_path(dir: &tempfile::TempDir) -> String {
        dir.path().join("ws").to_string_lossy().into_owned()
    }

    #[test]
    fn the_suggestion_is_tool_kit_under_documents() {
        assert!(suggested_workspace_path().ends_with("Documents/Tool-Kit"));
    }

    #[test]
    fn inspect_only_accepts_a_folder_with_a_workspace_marker() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ws");
        assert!(!inspect_workspace_path(&root.to_string_lossy()));
        std::fs::create_dir_all(root.join(".toolkit")).unwrap();
        assert!(!inspect_workspace_path(&root.to_string_lossy()));
        std::fs::write(root.join(".toolkit/workspace.json"), b"{}").unwrap();
        assert!(inspect_workspace_path(&root.to_string_lossy()));
    }

    #[test]
    fn setup_creates_the_marker_catch_all_and_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = workspace_path(&dir);

        let info = setup_workspace(&path).unwrap();
        assert!(info.workspace_id.starts_with("w_"));
        assert!(info.catch_all_project_id.starts_with("p_"));
        assert_eq!(info.catch_all_path, CATCH_ALL_TITLE);
        assert_eq!(info.workspace_path, path);

        let root = Path::new(&path);
        let marker: serde_json::Value = read_json(&root.join(".toolkit/workspace.json")).unwrap();
        assert_eq!(marker["schemaVersion"], 1);
        assert_eq!(marker["id"].as_str().unwrap(), info.workspace_id);
        let catch_all: serde_json::Value =
            read_json(&root.join("Drop Box/project.json")).unwrap();
        assert_eq!(catch_all["id"].as_str().unwrap(), info.catch_all_project_id);
        assert_eq!(catch_all["title"], "Drop Box");
        assert!(catch_all["createdAt"].as_str().unwrap().ends_with('Z'));
        assert!(root.join(".toolkit/index.db").is_file());

        let projects = list_projects_at(root).unwrap();
        assert_eq!(
            projects,
            vec![ProjectSummary {
                id: info.catch_all_project_id,
                title: "Drop Box".into(),
                path: "Drop Box".into(),
                created_at: catch_all["createdAt"].as_str().unwrap().to_string(),
            }]
        );
    }

    #[test]
    fn a_second_setup_adopts_instead_of_re_minting() {
        let dir = tempfile::tempdir().unwrap();
        let path = workspace_path(&dir);

        let first = setup_workspace(&path).unwrap();
        let second = setup_workspace(&path).unwrap();
        assert_eq!(second.workspace_id, first.workspace_id);
        assert_eq!(second.catch_all_project_id, first.catch_all_project_id);
        // The catch-all row is inserted once, not duplicated.
        assert_eq!(list_projects_at(Path::new(&path)).unwrap().len(), 1);
    }

    #[test]
    fn setup_adopts_a_catch_all_renamed_in_finder() {
        let dir = tempfile::tempdir().unwrap();
        let path = workspace_path(&dir);
        let first = setup_workspace(&path).unwrap();

        // Identity comes from project.json, so a rename keeps the project.
        std::fs::rename(
            Path::new(&path).join(CATCH_ALL_TITLE),
            Path::new(&path).join("Unsorted"),
        )
        .unwrap();
        let info = setup_workspace(&path).unwrap();
        assert_eq!(info.workspace_id, first.workspace_id);
        // A fresh catch-all, because the old one no longer sits under its name.
        assert_ne!(info.catch_all_project_id, first.catch_all_project_id);
    }

    #[test]
    fn a_new_workspace_gets_a_welcome_file_and_a_second_setup_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = workspace_path(&dir);

        let first = setup_workspace(&path).unwrap();
        let welcome = Path::new(&path).join("Drop Box/welcome.md");
        assert_eq!(
            first.welcome_path.as_deref(),
            Some(welcome.to_string_lossy().as_ref())
        );
        assert!(welcome.is_file());

        let second = setup_workspace(&path).unwrap();
        assert_eq!(second.welcome_path, None);
    }

    /// The file belongs to the user, so deleting it is their decision.
    #[test]
    fn a_deleted_welcome_file_is_not_written_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = workspace_path(&dir);
        setup_workspace(&path).unwrap();

        let welcome = Path::new(&path).join("Drop Box/welcome.md");
        std::fs::remove_file(&welcome).unwrap();
        let second = setup_workspace(&path).unwrap();

        assert_eq!(second.welcome_path, None);
        assert!(!welcome.exists());
    }

    /// A renamed Inbox makes setup mint a fresh folder, not a new workspace,
    /// so it carries no welcome file.
    #[test]
    fn the_fresh_catch_all_a_finder_rename_produces_carries_no_welcome_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = workspace_path(&dir);
        setup_workspace(&path).unwrap();

        std::fs::rename(
            Path::new(&path).join(CATCH_ALL_TITLE),
            Path::new(&path).join("Unsorted"),
        )
        .unwrap();
        let info = setup_workspace(&path).unwrap();
        assert_eq!(info.welcome_path, None);
        assert!(!Path::new(&path).join("Drop Box/welcome.md").exists());
    }

    #[test]
    fn a_corrupt_workspace_marker_is_an_error_not_a_clobber() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ws");
        std::fs::create_dir_all(root.join(".toolkit")).unwrap();
        std::fs::write(root.join(".toolkit/workspace.json"), b"not json").unwrap();

        let err = setup_workspace(&root.to_string_lossy()).unwrap_err();
        assert!(err.contains("could not be parsed"), "unexpected: {err}");
        assert_eq!(
            std::fs::read(root.join(".toolkit/workspace.json")).unwrap(),
            b"not json"
        );
    }

    #[test]
    fn a_workspace_that_is_gone_is_an_error_not_an_empty_library() {
        let dir = tempfile::tempdir().unwrap();
        let err = list_projects_at(&dir.path().join("moved")).unwrap_err();
        assert!(err.contains("Workspace not found"), "unexpected: {err}");
    }

    #[test]
    fn a_deleted_index_is_rebuilt_from_the_project_folders() {
        let dir = tempfile::tempdir().unwrap();
        let path = workspace_path(&dir);
        let info = setup_workspace(&path).unwrap();
        let root = Path::new(&path);
        let acme = create_project_at(root, "Acme").unwrap();

        // index.db is derived, so deleting it costs only the re-walk.
        std::fs::remove_file(root.join(".toolkit/index.db")).unwrap();
        let projects = list_projects_at(root).unwrap();
        let mut ids: Vec<&str> = projects.iter().map(|p| p.id.as_str()).collect();
        ids.sort_unstable();
        let mut want = vec![info.catch_all_project_id.as_str(), acme.id.as_str()];
        want.sort_unstable();
        assert_eq!(ids, want);
        assert!(projects
            .iter()
            .any(|p| p.path == "Acme" && p.title == "Acme"));
    }

    #[test]
    fn create_project_leaves_no_folder_behind_when_the_index_will_not_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = workspace_path(&dir);
        setup_workspace(&path).unwrap();
        let root = Path::new(&path);

        // A directory where the file belongs makes SQLite refuse to open it.
        std::fs::remove_file(root.join(".toolkit/index.db")).unwrap();
        std::fs::create_dir(root.join(".toolkit/index.db")).unwrap();

        assert!(create_project_at(root, "Acme").is_err());
        assert!(
            !root.join("Acme").exists(),
            "an orphan folder was left behind"
        );
        // So the same name is still free once the index is reachable again.
        std::fs::remove_dir(root.join(".toolkit/index.db")).unwrap();
        assert!(create_project_at(root, "Acme").is_ok());
    }

    #[test]
    fn create_project_refuses_a_workspace_that_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let moved = dir.path().join("moved");

        let err = create_project_at(&moved, "Acme").unwrap_err();
        assert!(err.contains("Workspace not found"), "unexpected: {err}");
        assert!(!moved.exists(), "a decoy workspace was created");
    }

    #[test]
    fn create_project_adds_a_folder_and_index_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = workspace_path(&dir);
        setup_workspace(&path).unwrap();

        let summary = create_project_at(Path::new(&path), "Acme Acquisition").unwrap();
        assert_eq!(summary.title, "Acme Acquisition");
        assert_eq!(summary.path, "Acme Acquisition");
        assert!(summary.id.starts_with("p_"));
        assert!(Path::new(&path)
            .join("Acme Acquisition/project.json")
            .is_file());
        assert_eq!(list_projects_at(Path::new(&path)).unwrap().len(), 2);
    }

    #[test]
    fn create_project_rejects_the_catch_all_and_empty_names() {
        let dir = tempfile::tempdir().unwrap();
        let path = workspace_path(&dir);
        setup_workspace(&path).unwrap();
        let root = Path::new(&path);

        assert!(create_project_at(root, "").is_err());
        assert!(create_project_at(root, "Drop Box").is_err());
        assert!(create_project_at(root, "drop box").is_err());
        // Still refused after the rename: the migration would take it.
        assert!(create_project_at(root, "Inbox").is_err());
        assert!(create_project_at(root, "inbox").is_err());
        assert!(create_project_at(root, ".").is_err());
        assert!(create_project_at(root, "..").is_err());
        assert!(create_project_at(root, ".toolkit").is_err());
        assert!(create_project_at(root, ".hidden").is_err());
    }

    /// The rename keeps the project. A second folder strands every file.
    #[test]
    fn a_legacy_inbox_is_renamed_rather_than_left_beside_a_new_catch_all() {
        let dir = tempfile::tempdir().unwrap();
        let path = workspace_path(&dir);
        let root = Path::new(&path);

        // A workspace as it was written before the rename.
        let first = setup_workspace(&path).unwrap();
        std::fs::rename(root.join(CATCH_ALL_TITLE), root.join("Inbox")).unwrap();
        let legacy_meta = root.join("Inbox/project.json");
        let mut meta: serde_json::Value = read_json(&legacy_meta).unwrap();
        meta["title"] = serde_json::Value::String("Inbox".into());
        write_json(&legacy_meta, &meta).unwrap();
        std::fs::write(root.join("Inbox/deck.md"), b"kept").unwrap();

        let after = setup_workspace(&path).unwrap();

        assert_eq!(after.catch_all_project_id, first.catch_all_project_id);
        assert!(!root.join("Inbox").exists(), "the legacy folder survived");
        assert_eq!(
            std::fs::read(root.join("Drop Box/deck.md")).unwrap(),
            b"kept",
            "the files did not come across"
        );
        let projects = list_projects_at(root).unwrap();
        assert_eq!(projects.len(), 1, "a second catch-all was minted");
        assert_eq!(projects[0].path, CATCH_ALL_TITLE);
        assert_eq!(projects[0].title, CATCH_ALL_TITLE);
    }

    /// Only a project still calling itself Inbox is the catch-all.
    #[test]
    fn a_retitled_inbox_folder_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = workspace_path(&dir);
        let root = Path::new(&path);
        setup_workspace(&path).unwrap();

        // Their own project, sitting at the folder name the migration looks at.
        std::fs::rename(root.join(CATCH_ALL_TITLE), root.join("Inbox")).unwrap();
        let their_meta = root.join("Inbox/project.json");
        let mut meta: serde_json::Value = read_json(&their_meta).unwrap();
        meta["title"] = serde_json::Value::String("Mail archive".into());
        write_json(&their_meta, &meta).unwrap();

        setup_workspace(&path).unwrap();

        assert!(root.join("Inbox/project.json").is_file(), "it was moved");
        let kept: serde_json::Value = read_json(&their_meta).unwrap();
        assert_eq!(kept["title"], "Mail archive");
        // Setup still owes the workspace a catch-all, so it mints a fresh one.
        assert!(root.join("Drop Box/project.json").is_file());
    }

    #[test]
    fn the_timestamp_is_a_real_rfc3339_utc_instant() {
        let now = iso_utc_now();
        assert_eq!(now.len(), 20);
        assert_eq!(now.chars().nth(4), Some('-'));
        assert_eq!(now.chars().nth(10), Some('T'));
        assert!(now.ends_with('Z'));
    }
}
