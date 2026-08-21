//! One directory level of the library, read straight from disk.
//!
//! Disk is authoritative and `.toolkit/index.db` is a rebuildable index over
//! it, so the tree never asks the database what a folder holds. `workspace.rs`
//! owns project identity and that database. This module owns the listing rules
//! and nothing else, which is why it carries no Tauri types: the pairing rule,
//! the sort, and the escape check are all unit-testable on their own.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;

use crate::jobs::{self, JobType};
use crate::settings::Settings;

/// How many entries one listing may carry. A folder past this renders a
/// "N more" row rather than N more rows.
pub const MAX_ENTRIES: usize = 500;

/// The one file in a project folder the app wrote rather than the user. A
/// click on it can only break the project, so it is hidden the way Finder
/// hides a dotfile. Only at a project root, which is the only place the app
/// puts one.
const PROJECT_MARKER: &str = "project.json";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRow {
    /// Absolute. The only path the webview hands back to a command that opens
    /// or reveals a file.
    pub path: String,
    /// Workspace-relative. What expansion and selection are keyed on, so a
    /// workspace renamed in Finder does not strand every entry.
    pub rel: String,
    pub name: String,
    pub is_dir: bool,
    /// Lowercase, no dot. Empty when the name carries none.
    pub ext: String,
    pub media_type: String,
    pub size: u64,
    pub modified_ms: u64,
    /// "convert" | "transcribe", from `JobType::accepts`, so the webview never
    /// mirrors the extension table. None means neither job takes this file.
    pub job: Option<String>,
    /// The sibling result this source already has. Absolute.
    pub result_path: Option<String>,
    /// That result's file name, for the row's trailing marker.
    pub result_name: Option<String>,
    /// True when this file opens in the document pane: a text extension under
    /// `MAX_PREVIEW_BYTES`. Decided here so a click can never round-trip into
    /// a `read_document` failure toast.
    pub openable: bool,
    /// The same test, applied to `result_path`. A click on a paired source
    /// opens its result, and an `html` result is not a document this pane
    /// reads, so the answer cannot be inferred from the source row.
    pub result_openable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirListing {
    /// The directory's own mtime, so a focus reconcile can skip a folder that
    /// did not change.
    pub modified_ms: u64,
    pub entries: Vec<FileRow>,
    /// Entries past `MAX_ENTRIES`, dropped from `entries`.
    pub truncated: usize,
    /// Files with a job and no result. Fills the project row's reserved count
    /// slot. Counted before truncation, because the count describes the folder
    /// rather than the rows that fit.
    pub pending: usize,
}

/// Resolve a workspace-relative path, refusing anything that leaves the
/// workspace.
///
/// `..` is refused outright rather than normalized away. A symlinked folder
/// makes lexical normalization lie about where the path lands, and there is
/// nothing under the workspace a user needs `..` to reach.
pub fn resolve(workspace: &Path, rel: &str) -> Result<PathBuf, String> {
    let candidate = Path::new(rel);
    for part in candidate.components() {
        match part {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("That path is outside the workspace".into())
            }
        }
    }
    Ok(workspace.join(candidate))
}

/// The mtime one directory carries right now, in the same units `DirListing`
/// reports.
///
/// The cheap half of the focus reconcile: one `stat`, where `list` costs a
/// `read_dir` plus a `stat` per entry. `None` for a path that escapes, is gone,
/// or is no longer a folder, which the caller reads as "changed" and answers
/// with a real listing.
pub fn modified(workspace: &Path, rel: &str) -> Option<u64> {
    let dir = resolve(workspace, rel).ok()?;
    let meta = std::fs::metadata(dir).ok()?;
    meta.is_dir().then(|| modified_ms(&meta))
}

/// List one directory level under the workspace. `rel` is workspace-relative
/// and is refused if it escapes.
pub fn list(workspace: &Path, rel: &str, cfg: &Settings) -> Result<DirListing, String> {
    let dir = resolve(workspace, rel)?;
    let dir_meta = std::fs::metadata(&dir).map_err(|e| e.to_string())?;
    if !dir_meta.is_dir() {
        return Err("Not a folder".into());
    }
    // Only a project root holds a project.json the app wrote. A folder deeper
    // in the tree carrying that name belongs to the user.
    let at_project_root = Path::new(rel).components().count() == 1;

    let mut raws = read_level(&dir, at_project_root)?;
    raws.sort_by(|a, b| natural_cmp(&a.name, &b.name).then_with(|| a.name.cmp(&b.name)));

    let paired = pair_results(&raws, cfg);
    let mut entries = Vec::with_capacity(raws.len());
    for (index, raw) in raws.iter().enumerate() {
        if paired.claimed[index] {
            continue;
        }
        let result = paired.of[index].map(|hit| &raws[hit]);
        entries.push(FileRow {
            rel: join_rel(rel, &raw.name),
            path: raw.path.to_string_lossy().into_owned(),
            is_dir: raw.is_dir,
            media_type: crate::media_type(&raw.path),
            job: (!raw.is_dir)
                .then(|| job_for(&raw.ext))
                .flatten()
                .map(|jt| jt.id().to_string()),
            result_path: result.map(|hit| hit.path.to_string_lossy().into_owned()),
            result_name: result.map(|hit| hit.name.clone()),
            openable: opens_in_pane(raw),
            result_openable: result.is_some_and(opens_in_pane),
            name: raw.name.clone(),
            ext: raw.ext.clone(),
            size: raw.size,
            modified_ms: raw.modified_ms,
        });
    }

    let pending = entries
        .iter()
        .filter(|row| row.job.is_some() && row.result_name.is_none())
        .count();
    let truncated = entries.len().saturating_sub(MAX_ENTRIES);
    entries.truncate(MAX_ENTRIES);
    Ok(DirListing {
        modified_ms: modified_ms(&dir_meta),
        entries,
        truncated,
        pending,
    })
}

/// Whether the document pane can read this file: a text extension under the
/// preview cap. One rule, so a source row and the result it pairs with answer
/// it the same way.
fn opens_in_pane(raw: &Raw) -> bool {
    !raw.is_dir
        && raw.size <= crate::MAX_PREVIEW_BYTES
        && crate::ALREADY_TEXT.contains(&raw.ext.as_str())
}

/// One directory entry, before sorting and pairing decide what it becomes.
struct Raw {
    name: String,
    path: PathBuf,
    is_dir: bool,
    ext: String,
    size: u64,
    modified_ms: u64,
}

fn read_level(dir: &Path, at_project_root: bool) -> Result<Vec<Raw>, String> {
    // Finder hides dotfiles, and .git, .DS_Store and .toolkit are never what
    // the user came here to read. Same predicate the input walk uses.
    let hidden = |name: &str| name.starts_with('.');

    let mut raws = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if hidden(&name) || (at_project_root && name == PROJECT_MARKER) {
            continue;
        }
        // Neither call follows a symlink, so a symlinked folder lists as a
        // file and is never descended. That is also what the escape check
        // above exists for.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let path = entry.path();
        raws.push(Raw {
            ext: extension_of(&name),
            is_dir: file_type.is_dir(),
            size: meta.len(),
            modified_ms: modified_ms(&meta),
            name,
            path,
        });
    }
    Ok(raws)
}

/// Which source rows carry a result, and which result rows were folded into
/// one. A folded result is not a row of its own: `deck.pdf` and `deck.md` are
/// one line that names both.
struct Pairing {
    of: Vec<Option<usize>>,
    claimed: Vec<bool>,
}

fn pair_results(raws: &[Raw], cfg: &Settings) -> Pairing {
    // Every file that could be a result, keyed by the stem and extension a
    // conversion would have produced. `numbered` holds the same file under the
    // stem it came from: `write_output` answers a collision with
    // `deck (1).md`, and a plain stem match would never find it.
    let mut exact: HashMap<(String, String), Vec<usize>> = HashMap::new();
    let mut numbered: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (index, raw) in raws.iter().enumerate() {
        if raw.is_dir || raw.ext.is_empty() {
            continue;
        }
        let stem = stem_of(&raw.name).to_lowercase();
        let base = result_stem(&stem);
        if base != stem {
            numbered
                .entry((base.to_string(), raw.ext.clone()))
                .or_default()
                .push(index);
        }
        exact.entry((stem, raw.ext.clone())).or_default().push(index);
    }

    let mut pairing = Pairing {
        of: vec![None; raws.len()],
        claimed: vec![false; raws.len()],
    };
    for (index, raw) in raws.iter().enumerate() {
        if raw.is_dir {
            continue;
        }
        let Some(jt) = job_for(&raw.ext) else {
            continue;
        };
        let key = (
            stem_of(&raw.name).to_lowercase(),
            jobs::output_extension_for(jt, cfg).to_string(),
        );
        // The plain name a first conversion writes wins over a numbered one,
        // whatever the sort order says.
        let hit = [exact.get(&key), numbered.get(&key)]
            .into_iter()
            .flatten()
            .flat_map(|candidates| candidates.iter().copied())
            .find(|&candidate| candidate != index && !pairing.claimed[candidate]);
        if let Some(hit) = hit {
            pairing.claimed[hit] = true;
            pairing.of[index] = Some(hit);
        }
    }
    pairing
}

fn job_for(ext: &str) -> Option<JobType> {
    [JobType::Convert, JobType::Transcribe]
        .into_iter()
        .find(|jt| jt.accepts(ext))
}

fn join_rel(rel: &str, name: &str) -> String {
    if rel.is_empty() {
        name.to_string()
    } else {
        format!("{rel}/{name}")
    }
}

fn stem_of(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_string())
}

fn extension_of(name: &str) -> String {
    Path::new(name)
        .extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// `write_output` numbers a collision rather than clobbering it, so a second
/// conversion of `deck.pdf` lands as `deck (1).md`. Strip that suffix so the
/// pairing rule still recognizes the file as `deck`'s result.
fn result_stem(stem: &str) -> &str {
    let Some(rest) = stem.strip_suffix(')') else {
        return stem;
    };
    let Some((head, digits)) = rest.rsplit_once(" (") else {
        return stem;
    };
    if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) {
        head
    } else {
        stem
    }
}

fn modified_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|since| u64::try_from(since.as_millis()).ok())
        .unwrap_or(0)
}

/// Finder's `localizedStandardCompare` in the small: case-insensitive, with
/// runs of digits compared as numbers. `img2` before `img10` is the kind of
/// thing a Mac user reads as a broken list.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (start_a, start_b) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            // Leading zeros carry no value, so compare length first and then
            // digit by digit. That reads a run of any width without parsing it
            // into an integer that could overflow.
            let da = without_leading_zeros(&a[start_a..i]);
            let db = without_leading_zeros(&b[start_b..j]);
            let ord = da.len().cmp(&db.len()).then_with(|| da.cmp(db));
            if ord != Ordering::Equal {
                return ord;
            }
        } else {
            let ca = lower(a[i]);
            let cb = lower(b[j]);
            if ca != cb {
                return ca.cmp(&cb);
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

fn without_leading_zeros(digits: &[char]) -> &[char] {
    let mut start = 0;
    while start + 1 < digits.len() && digits[start] == '0' {
        start += 1;
    }
    &digits[start..]
}

fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(format: &str) -> Settings {
        Settings {
            datalab_format: format.into(),
            ..Settings::default()
        }
    }

    /// Build a workspace holding one project folder with `files` in it.
    fn project(dir: &tempfile::TempDir, files: &[&str]) -> PathBuf {
        let root = dir.path().join("ws");
        let inbox = root.join("Inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        for name in files {
            std::fs::write(inbox.join(name), b"x").unwrap();
        }
        root
    }

    fn names(listing: &DirListing) -> Vec<&str> {
        listing.entries.iter().map(|e| e.name.as_str()).collect()
    }

    fn row<'a>(listing: &'a DirListing, name: &str) -> &'a FileRow {
        listing
            .entries
            .iter()
            .find(|e| e.name == name)
            .unwrap_or_else(|| panic!("{name} is not in the listing"))
    }

    #[test]
    fn digits_sort_by_value_so_img2_comes_before_img10() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["img10.pdf", "img2.pdf", "IMG1.pdf"]);

        let listing = list(&root, "Inbox", &settings("markdown")).unwrap();

        assert_eq!(names(&listing), ["IMG1.pdf", "img2.pdf", "img10.pdf"]);
    }

    #[test]
    fn a_source_and_its_sibling_result_are_one_row() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["deck.pdf", "deck.md"]);

        let listing = list(&root, "Inbox", &settings("markdown")).unwrap();

        assert_eq!(names(&listing), ["deck.pdf"]);
        assert_eq!(row(&listing, "deck.pdf").result_name.as_deref(), Some("deck.md"));
        assert_eq!(listing.pending, 0);
    }

    /// The output format decides the extension, so the same folder pairs
    /// differently under `html`. A tree that read only `.md` would offer to
    /// convert a file that already has its result.
    #[test]
    fn the_output_format_decides_which_sibling_counts_as_the_result() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["deck.pdf", "deck.html"]);

        let markdown = list(&root, "Inbox", &settings("markdown")).unwrap();
        assert_eq!(row(&markdown, "deck.pdf").result_name, None);
        assert_eq!(markdown.pending, 2);

        let html = list(&root, "Inbox", &settings("html")).unwrap();
        assert_eq!(names(&html), ["deck.pdf"]);
        assert_eq!(row(&html, "deck.pdf").result_name.as_deref(), Some("deck.html"));
    }

    #[test]
    fn a_numbered_result_still_pairs_with_its_source() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["deck.pdf", "deck (1).md"]);

        let listing = list(&root, "Inbox", &settings("markdown")).unwrap();

        assert_eq!(names(&listing), ["deck.pdf"]);
        assert_eq!(
            row(&listing, "deck.pdf").result_name.as_deref(),
            Some("deck (1).md")
        );
    }

    /// Two sources want one `deck.md`. The first in sort order takes it and
    /// the other still offers to convert, which is deterministic rather than
    /// silent.
    #[test]
    fn the_first_source_in_sort_order_claims_a_shared_result() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["deck.pdf", "deck.docx", "deck.md"]);

        let listing = list(&root, "Inbox", &settings("markdown")).unwrap();

        assert_eq!(names(&listing), ["deck.docx", "deck.pdf"]);
        assert_eq!(row(&listing, "deck.docx").result_name.as_deref(), Some("deck.md"));
        assert_eq!(row(&listing, "deck.pdf").result_name, None);
        assert_eq!(listing.pending, 1);
    }

    #[test]
    fn dotfiles_and_the_project_marker_are_absent() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &[".DS_Store", "project.json", "notes.md"]);

        let listing = list(&root, "Inbox", &settings("markdown")).unwrap();

        assert_eq!(names(&listing), ["notes.md"]);
    }

    #[test]
    fn a_relative_path_that_climbs_out_of_the_workspace_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["notes.md"]);

        let err = list(&root, "Inbox/../..", &settings("markdown")).unwrap_err();

        assert!(err.contains("outside the workspace"), "unexpected: {err}");
        assert!(resolve(&root, "/etc").is_err());
    }

    #[test]
    fn a_symlinked_directory_lists_as_a_file_and_is_never_descended() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["notes.md"]);
        let target = dir.path().join("elsewhere");
        std::fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, root.join("Inbox/link")).unwrap();

        let listing = list(&root, "Inbox", &settings("markdown")).unwrap();

        assert!(!row(&listing, "link").is_dir);
    }

    #[test]
    fn a_folder_past_the_cap_reports_what_it_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let names: Vec<String> = (0..600).map(|n| format!("note{n:04}.md")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let root = project(&dir, &refs);

        let listing = list(&root, "Inbox", &settings("markdown")).unwrap();

        assert_eq!(listing.entries.len(), MAX_ENTRIES);
        assert_eq!(listing.truncated, 100);
        assert_eq!(listing.entries[0].name, "note0000.md");
    }

    #[test]
    fn a_markdown_file_opens_and_a_pdf_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["notes.md", "deck.pdf"]);

        let listing = list(&root, "Inbox", &settings("markdown")).unwrap();

        assert!(row(&listing, "notes.md").openable);
        assert!(!row(&listing, "deck.pdf").openable);
        assert_eq!(row(&listing, "deck.pdf").job.as_deref(), Some("convert"));
        assert_eq!(row(&listing, "notes.md").job, None);
        assert_eq!(row(&listing, "notes.md").rel, "Inbox/notes.md");
    }

    #[test]
    fn a_markdown_result_opens_and_an_html_one_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["deck.pdf", "deck.md", "slides.pdf", "slides.html"]);

        let markdown = list(&root, "Inbox", &settings("markdown")).unwrap();
        assert!(row(&markdown, "deck.pdf").result_openable);

        let html = list(&root, "Inbox", &settings("html")).unwrap();
        assert!(!row(&html, "slides.pdf").result_openable);
        assert_eq!(
            row(&html, "slides.pdf").result_name.as_deref(),
            Some("slides.html")
        );
    }

    #[test]
    fn audio_is_a_transcribe_source() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["call.m4a", "call.txt"]);

        let listing = list(&root, "Inbox", &settings("markdown")).unwrap();

        assert_eq!(names(&listing), ["call.m4a"]);
        assert_eq!(row(&listing, "call.m4a").job.as_deref(), Some("transcribe"));
        assert_eq!(row(&listing, "call.m4a").result_name.as_deref(), Some("call.txt"));
    }

    #[test]
    fn a_project_marker_deeper_in_the_tree_belongs_to_the_user() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &[]);
        let nested = root.join("Inbox/data");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("project.json"), b"{}").unwrap();

        let listing = list(&root, "Inbox/data", &settings("markdown")).unwrap();

        assert_eq!(names(&listing), ["project.json"]);
    }

    /// The reconcile's cheap question has to answer with the same number the
    /// listing carries, or every focus would look like a change and the stat
    /// would buy nothing.
    #[test]
    fn the_folder_mtime_matches_the_one_its_listing_reports() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["deck.pdf"]);

        let listing = list(&root, "Inbox", &settings("markdown")).unwrap();

        assert_eq!(modified(&root, "Inbox"), Some(listing.modified_ms));
    }

    /// A folder that is gone, a file, and a path out of the workspace all
    /// answer the same way: ask the listing, which owns what those mean.
    #[test]
    fn nothing_that_is_not_a_folder_here_reports_an_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["deck.pdf"]);

        assert_eq!(modified(&root, "Inbox/gone"), None);
        assert_eq!(modified(&root, "Inbox/deck.pdf"), None);
        assert_eq!(modified(&root, "../elsewhere"), None);
    }
}
