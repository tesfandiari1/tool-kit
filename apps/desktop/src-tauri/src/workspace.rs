//! Workspace foundation: the one user-chosen folder the library lives in.
//!
//! On disk (docs/north-star.md): `.toolkit/workspace.json` and
//! `.toolkit/index.db` are derived state, safe to delete and rebuild; a
//! project's identity is its `project.json`, so renaming a folder in Finder
//! changes nothing. `index.db` is only the fast lookup over the projects.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::settings;

const INBOX_TITLE: &str = "Inbox";

/// What `setup_workspace` knows after creating or adopting a folder.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    pub workspace_path: String,
    pub workspace_id: String,
    pub inbox_project_id: String,
    /// True when the folder already held a `.toolkit/workspace.json` and we
    /// took it over rather than minting a new identity.
    pub adopted: bool,
}

/// One project, as the UI lists it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub id: String,
    pub title: String,
    /// Folder relative to the workspace root ("Inbox"), so moving the
    /// workspace doesn't stale the index.
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

/// Whether `path` already looks like a workspace: a `.toolkit/workspace.json`
/// is the identity marker, everything else is rebuildable.
pub fn inspect_workspace_path(path: &str) -> bool {
    Path::new(path)
        .join(".toolkit")
        .join("workspace.json")
        .is_file()
}

/// Create a workspace at `path`, or adopt one that's already there. Idempotent:
/// a second call returns the same ids with `adopted: true`.
pub fn setup_workspace(path: &str) -> Result<WorkspaceInfo, String> {
    let root = Path::new(path);
    let toolkit = root.join(".toolkit");
    std::fs::create_dir_all(&toolkit).map_err(|e| e.to_string())?;

    let workspace_file = toolkit.join("workspace.json");
    let adopted = workspace_file.is_file();
    let workspace = if adopted {
        read_json::<WorkspaceMeta>(&workspace_file)?
    } else {
        let meta = WorkspaceMeta {
            schema_version: 1,
            id: format!("w_{}", uuid::Uuid::new_v4()),
            created_at: iso_utc_now(),
        };
        write_json(&workspace_file, &meta)?;
        meta
    };

    // Inbox is created automatically at first run; an existing one is adopted
    // by its project.json, never re-minted.
    let inbox_dir = root.join(INBOX_TITLE);
    std::fs::create_dir_all(&inbox_dir).map_err(|e| e.to_string())?;
    let inbox_file = inbox_dir.join("project.json");
    let inbox = if inbox_file.is_file() {
        read_json::<ProjectMeta>(&inbox_file)?
    } else {
        let meta = ProjectMeta {
            schema_version: 1,
            id: format!("p_{}", uuid::Uuid::new_v4()),
            title: INBOX_TITLE.into(),
            created_at: iso_utc_now(),
        };
        write_json(&inbox_file, &meta)?;
        meta
    };

    let index = open_index(&toolkit.join("index.db"))?;
    index
        .execute(
            "INSERT OR IGNORE INTO projects (id, title, path, created_at) \
             VALUES (?1, ?2, ?3, ?4)",
            (&inbox.id, &inbox.title, INBOX_TITLE, &inbox.created_at),
        )
        .map_err(|e| e.to_string())?;

    Ok(WorkspaceInfo {
        workspace_path: path.to_string(),
        workspace_id: workspace.id,
        inbox_project_id: inbox.id,
        adopted,
    })
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
    let project_dir = workspace.join(&folder);
    if project_dir.exists() {
        return Err(format!("A project named “{folder}” already exists"));
    }
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

    let index = open_index(&workspace.join(".toolkit").join("index.db"))?;
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
    if trimmed.eq_ignore_ascii_case(INBOX_TITLE) {
        return Err(format!("{INBOX_TITLE} is reserved"));
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

/// Every project in the configured workspace's index. `index.db` is derived,
/// so a missing file reads as an empty list rather than an error.
pub fn list_projects(app: &AppHandle) -> Result<Vec<ProjectSummary>, String> {
    let workspace = settings::load(app)
        .workspace_path
        .ok_or_else(|| "No workspace configured".to_string())?;
    list_projects_at(Path::new(&workspace))
}

fn list_projects_at(workspace: &Path) -> Result<Vec<ProjectSummary>, String> {
    let db_path = workspace.join(".toolkit").join("index.db");
    if !db_path.is_file() {
        return Ok(Vec::new());
    }
    let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;
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

fn open_index(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS projects (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            path TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS documents (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
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
    fn setup_creates_the_marker_inbox_and_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = workspace_path(&dir);

        let info = setup_workspace(&path).unwrap();
        assert!(!info.adopted);
        assert!(info.workspace_id.starts_with("w_"));
        assert!(info.inbox_project_id.starts_with("p_"));
        assert_eq!(info.workspace_path, path);

        let root = Path::new(&path);
        let marker: serde_json::Value = read_json(&root.join(".toolkit/workspace.json")).unwrap();
        assert_eq!(marker["schemaVersion"], 1);
        assert_eq!(marker["id"].as_str().unwrap(), info.workspace_id);
        let inbox: serde_json::Value = read_json(&root.join("Inbox/project.json")).unwrap();
        assert_eq!(inbox["id"].as_str().unwrap(), info.inbox_project_id);
        assert_eq!(inbox["title"], "Inbox");
        assert!(inbox["createdAt"].as_str().unwrap().ends_with('Z'));
        assert!(root.join(".toolkit/index.db").is_file());

        let projects = list_projects_at(root).unwrap();
        assert_eq!(
            projects,
            vec![ProjectSummary {
                id: info.inbox_project_id,
                title: "Inbox".into(),
                path: "Inbox".into(),
                created_at: inbox["createdAt"].as_str().unwrap().to_string(),
            }]
        );
    }

    #[test]
    fn a_second_setup_adopts_instead_of_re_minting() {
        let dir = tempfile::tempdir().unwrap();
        let path = workspace_path(&dir);

        let first = setup_workspace(&path).unwrap();
        let second = setup_workspace(&path).unwrap();
        assert!(second.adopted);
        assert_eq!(second.workspace_id, first.workspace_id);
        assert_eq!(second.inbox_project_id, first.inbox_project_id);
        // The Inbox row is inserted once, not duplicated.
        assert_eq!(list_projects_at(Path::new(&path)).unwrap().len(), 1);
    }

    #[test]
    fn setup_adopts_an_inbox_renamed_in_finder() {
        let dir = tempfile::tempdir().unwrap();
        let path = workspace_path(&dir);
        let first = setup_workspace(&path).unwrap();

        // Identity comes from project.json, so a rename keeps the project.
        std::fs::rename(
            Path::new(&path).join("Inbox"),
            Path::new(&path).join("Unsorted"),
        )
        .unwrap();
        let info = setup_workspace(&path).unwrap();
        assert!(info.adopted);
        assert_eq!(info.workspace_id, first.workspace_id);
        // A fresh Inbox is created because the old one is no longer at
        // "Inbox"; the renamed folder is adopted by a scan, not by setup.
        assert_ne!(info.inbox_project_id, first.inbox_project_id);
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
    fn a_workspace_without_an_index_lists_no_projects() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(list_projects_at(dir.path()).unwrap(), Vec::new());
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
        assert!(Path::new(&path).join("Acme Acquisition/project.json").is_file());
        assert_eq!(list_projects_at(Path::new(&path)).unwrap().len(), 2);
    }

    #[test]
    fn create_project_rejects_inbox_and_empty_names() {
        let dir = tempfile::tempdir().unwrap();
        let path = workspace_path(&dir);
        setup_workspace(&path).unwrap();
        let root = Path::new(&path);

        assert!(create_project_at(root, "").is_err());
        assert!(create_project_at(root, "Inbox").is_err());
        assert!(create_project_at(root, "inbox").is_err());
        assert!(create_project_at(root, ".").is_err());
        assert!(create_project_at(root, "..").is_err());
        assert!(create_project_at(root, ".toolkit").is_err());
        assert!(create_project_at(root, ".hidden").is_err());
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
