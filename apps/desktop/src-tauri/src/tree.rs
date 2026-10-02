//! One directory level of the library, read straight from disk.
//!
//! Disk is authoritative and `.toolkit/index.db` is a rebuildable index, so the
//! tree never asks the database what a folder holds. No Tauri types here, so
//! the pairing rule, the sort and the escape check are unit-testable.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;

use crate::jobs::{self, JobType};
use crate::settings::Settings;

/// How many entries one listing carries before a "N more" row.
pub const MAX_ENTRIES: usize = 500;

/// The one file in a project folder the app wrote. Hidden, because a click on
/// it can only break the project. Only at a project root.
const PROJECT_MARKER: &str = "project.json";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRow {
    /// Absolute. The only path the webview hands back to open or reveal.
    pub path: String,
    /// Workspace-relative, so a rename in Finder strands no expanded row.
    pub rel: String,
    pub name: String,
    pub is_dir: bool,
    /// Lowercase, no dot. Empty when the name carries none.
    pub ext: String,
    pub media_type: String,
    pub size: u64,
    pub modified_ms: u64,
    /// From `JobType::accepts`, so the webview never mirrors the table.
    pub job: Option<String>,
    /// The sibling result this source already has. Absolute.
    pub result_path: Option<String>,
    /// That result's file name, for the row's trailing marker.
    pub result_name: Option<String>,
    /// True when this file opens in the document pane: a text extension under
    /// `MAX_PREVIEW_BYTES`. It cannot answer for the encoding, so the caller
    /// still falls back to the card when the read refuses.
    pub openable: bool,
    /// The same test on `result_path`: a click on a paired source opens its
    /// result, and an `html` result is not one this pane reads.
    pub result_openable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirListing {
    /// The directory's own mtime, so a focus reconcile can skip it.
    pub modified_ms: u64,
    pub entries: Vec<FileRow>,
    /// Entries past `MAX_ENTRIES`, dropped from `entries`.
    pub truncated: usize,
    /// Files with a job and no result. Counted before truncation, so it
    /// describes the folder and not the rows shown.
    pub pending: usize,
}

/// Why a listing failed, in the one distinction its caller acts on. Only a
/// folder that is gone loses its place in the persisted expansion. A sleeping
/// volume or a permission error is a folder that is still there.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListError {
    /// The folder is not there any more, or is not a folder.
    pub gone: bool,
    pub message: String,
}

impl ListError {
    pub fn gone(message: impl Into<String>) -> Self {
        Self {
            gone: true,
            message: message.into(),
        }
    }

    pub fn transient(message: impl Into<String>) -> Self {
        Self {
            gone: false,
            message: message.into(),
        }
    }

    /// Only the two kinds that prove the folder is gone. A permission error or
    /// a dropped volume leaves the row alone.
    fn from_io(error: &std::io::Error) -> Self {
        use std::io::ErrorKind::{NotADirectory, NotFound};
        if matches!(error.kind(), NotFound | NotADirectory) {
            Self::gone(error.to_string())
        } else {
            Self::transient(error.to_string())
        }
    }
}

/// Resolve a workspace-relative path, refusing anything that leaves the
/// workspace. `..` is refused outright, because a symlinked folder makes
/// lexical normalization lie about where the path lands.
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

/// The mtime one directory carries now, in the units `DirListing` reports. The
/// cheap half of the focus reconcile: one `stat`, against a `read_dir` plus a
/// `stat` per entry. `None` reads as "changed".
pub fn modified(workspace: &Path, rel: &str) -> Option<u64> {
    let dir = resolve(workspace, rel).ok()?;
    let meta = std::fs::metadata(dir).ok()?;
    meta.is_dir().then(|| modified_ms(&meta))
}

/// List one directory level under the workspace, refusing a `rel` that escapes.
pub fn list(workspace: &Path, rel: &str, cfg: &Settings) -> Result<DirListing, ListError> {
    // A path that escapes names no folder here, so it is as gone as a deleted.
    let dir = resolve(workspace, rel).map_err(ListError::gone)?;
    let dir_meta = std::fs::metadata(&dir).map_err(|e| ListError::from_io(&e))?;
    if !dir_meta.is_dir() {
        return Err(ListError::gone("Not a folder"));
    }
    // Only a project root holds a project.json the app wrote. Deeper down, that
    // name belongs to the user.
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

/// The result sitting beside the file at `rel`. Runs the listing and the
/// pairing the tree runs, so a move cannot carry a different set and split the
/// pair the user is looking at.
pub fn paired_result(workspace: &Path, rel: &str, cfg: &Settings) -> Option<PathBuf> {
    let source = resolve(workspace, rel).ok()?;
    let dir = source.parent()?;
    let name = source.file_name()?.to_str()?;
    // The same test `list` makes: the parent is a project root when the file
    // sits directly inside it.
    let at_project_root = Path::new(rel).components().count() == 2;
    let raws = read_level(dir, at_project_root).ok()?;
    let index = raws.iter().position(|raw| raw.name == name)?;
    pair_results(&raws, cfg).of[index].map(|hit| raws[hit].path.clone())
}

/// Whether the document pane can read this file: a text extension under the
/// preview cap. One rule for a source and its result.
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

fn read_level(dir: &Path, at_project_root: bool) -> Result<Vec<Raw>, ListError> {
    // Finder hides dotfiles, and .git and .toolkit are not what the user came
    // to read. Same predicate the input walk uses.
    let hidden = |name: &str| name.starts_with('.');

    let mut raws = Vec::new();
    for entry in std::fs::read_dir(dir)
        .map_err(|e| ListError::from_io(&e))?
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        if hidden(&name) || (at_project_root && name == PROJECT_MARKER) {
            continue;
        }
        // Neither call follows a symlink, so one lists as a file, never
        // descended.
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

/// Which source rows carry a result, and which results were folded into one.
/// `deck.pdf` and `deck.md` are one line that names both.
struct Pairing {
    of: Vec<Option<usize>>,
    claimed: Vec<bool>,
}

/// How much older than its source a result may be and still be its result. A
/// file that predates its source did not come from it, and without this rule a
/// hand-written `deck.md` folds `deck.pdf` in and that file never converts.
///
/// The grace absorbs one bulk write pass: a checkout lays a directory down in
/// name order, so a few milliseconds is ordering, not evidence. Erring this way
/// costs a visible Convert button rather than a silent skip.
const PAIR_GRACE_MS: u64 = 2_000;

fn pair_results(raws: &[Raw], cfg: &Settings) -> Pairing {
    // Every file that could be a result, keyed by the stem and extension a
    // conversion would produce. `numbered` re-keys `deck (1).md` under `deck`.
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
        exact
            .entry((stem, raw.ext.clone()))
            .or_default()
            .push(index);
    }

    let mut pairing = Pairing {
        of: vec![None; raws.len()],
        claimed: vec![false; raws.len()],
    };
    // Documents claim first. `lecture.mp4` sorts ahead of `lecture.pdf`, and
    // both pair with `lecture.md`, so name order hands the recording the
    // document's result and the tree offers a paid Convert again. A recording
    // left unpaired still meets the history check in `convert_one`.
    let mut sources: Vec<(usize, JobType)> = raws
        .iter()
        .enumerate()
        .filter(|(_, raw)| !raw.is_dir)
        .filter_map(|(index, raw)| Some((index, job_for(&raw.ext)?)))
        .collect();
    // Stable, so name order still decides within each job.
    sources.sort_by_key(|&(_, jt)| jt == JobType::Transcribe);
    for (index, jt) in sources {
        let raw = &raws[index];
        let stem = stem_of(&raw.name).to_lowercase();
        // Every extension this route can write: the service writes `.md`
        // whatever the format says, so the format alone reads as unconverted.
        let keys: Vec<(String, String)> = jobs::result_extensions_for(jt, cfg)
            .into_iter()
            .map(|ext| (stem.clone(), ext.to_string()))
            .collect();
        // The plain name a first conversion writes beats a numbered one.
        let hit = keys
            .iter()
            .flat_map(|key| exact.get(key))
            .chain(keys.iter().flat_map(|key| numbered.get(key)))
            .flat_map(|candidates| candidates.iter().copied())
            .find(|&candidate| {
                candidate != index
                    && !pairing.claimed[candidate]
                    && raws[candidate].modified_ms + PAIR_GRACE_MS >= raw.modified_ms
            });
        if let Some(hit) = hit {
            pairing.claimed[hit] = true;
            pairing.of[index] = Some(hit);
        }
    }
    pairing
}

pub(crate) fn job_for(ext: &str) -> Option<JobType> {
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

/// `write_output` numbers a collision, so a second conversion of `deck.pdf`
/// lands as `deck (1).md`. Strip the suffix, or the pairing rule misses it.
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
/// runs of digits compared as numbers. `img10` before `img2` reads as broken.
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
            // Leading zeros carry no value, so compare length then digits.
            // That reads any width without an integer parse.
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

    /// The shipped route: the conversion service, which writes Markdown.
    fn settings(format: &str) -> Settings {
        Settings {
            datalab_format: format.into(),
            ..Settings::default()
        }
    }

    /// Datalab over the network, which writes the chosen format alone.
    fn direct(format: &str) -> Settings {
        Settings {
            conversion_route: crate::settings::ConversionRoute::Direct,
            ..settings(format)
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

    /// `paired_result` is what a move carries, so it answers with the file the
    /// row draws or the move splits the pair.
    #[test]
    fn the_result_a_move_carries_is_the_one_the_row_draws() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["deck.pdf", "deck.md", "lonely.pdf"]);
        let cfg = settings("markdown");

        assert_eq!(
            paired_result(&root, "Inbox/deck.pdf", &cfg),
            Some(root.join("Inbox/deck.md")),
        );
        assert_eq!(paired_result(&root, "Inbox/lonely.pdf", &cfg), None);
        assert_eq!(paired_result(&root, "Inbox/gone.pdf", &cfg), None);
        // Same answer the listing gives, which is the whole point of reusing it.
        let listing = list(&root, "Inbox", &cfg).unwrap();
        assert_eq!(
            row(&listing, "deck.pdf").result_path.as_deref(),
            Some(root.join("Inbox/deck.md").to_string_lossy().as_ref()),
        );
    }

    /// A result the pairing rule refuses is not one a move may carry either.
    #[test]
    fn a_result_too_old_to_pair_is_not_carried() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["deck.pdf", "deck.md"]);
        age(&root.join("Inbox/deck.md"), 7 * 24 * 60 * 60);

        assert_eq!(paired_result(&root, "Inbox/deck.pdf", &settings("markdown")), None);
    }

    #[test]
    fn digits_sort_by_value_so_img2_comes_before_img10() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["img10.pdf", "img2.pdf", "IMG1.pdf"]);

        let listing = list(&root, "Inbox", &settings("markdown")).unwrap();

        assert_eq!(names(&listing), ["IMG1.pdf", "img2.pdf", "img10.pdf"]);
    }

    /// Backdate a file, so a test can say "this existed before that".
    fn age(path: &Path, seconds: u64) {
        let when = std::time::SystemTime::now() - std::time::Duration::from_secs(seconds);
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(when)
            .unwrap();
    }

    #[test]
    fn a_result_older_than_its_source_is_not_its_result() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["deck.md", "deck.pdf"]);
        // A note the user wrote last week, and a deck dropped in today.
        age(&root.join("Inbox/deck.md"), 7 * 24 * 60 * 60);

        let listing = list(&root, "Inbox", &settings("markdown")).unwrap();

        // Two rows: the note is its own file, the deck still needs converting.
        assert_eq!(names(&listing), ["deck.md", "deck.pdf"]);
        assert_eq!(row(&listing, "deck.pdf").result_name, None);
        assert_eq!(listing.pending, 1);
    }

    #[test]
    fn a_source_edited_after_its_result_needs_converting_again() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["deck.pdf", "deck.md"]);
        // Same shape by another route: the source changed after the run, and
        // Settings promises this runs again.
        age(&root.join("Inbox/deck.md"), 60);

        let listing = list(&root, "Inbox", &settings("markdown")).unwrap();

        assert_eq!(row(&listing, "deck.pdf").result_name, None);
    }

    #[test]
    fn one_write_pass_still_pairs_whatever_order_it_landed_in() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["deck.pdf", "deck.md"]);
        // A checkout lays a directory down in name order, so the result lands
        // a moment early. The grace absorbs that.
        age(&root.join("Inbox/deck.md"), 1);

        let listing = list(&root, "Inbox", &settings("markdown")).unwrap();

        assert_eq!(names(&listing), ["deck.pdf"]);
        assert_eq!(
            row(&listing, "deck.pdf").result_name.as_deref(),
            Some("deck.md")
        );
    }

    #[test]
    fn a_source_and_its_sibling_result_are_one_row() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["deck.pdf", "deck.md"]);

        let listing = list(&root, "Inbox", &settings("markdown")).unwrap();

        assert_eq!(names(&listing), ["deck.pdf"]);
        assert_eq!(
            row(&listing, "deck.pdf").result_name.as_deref(),
            Some("deck.md")
        );
        assert_eq!(listing.pending, 0);
    }

    /// `lecture.mp4` sorts first, and both sources pair with `lecture.md`. The
    /// recording must not take it, or the PDF is offered a paid Convert again.
    #[test]
    fn a_document_claims_a_shared_stem_result_before_a_recording() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["lecture.mp4", "lecture.pdf", "lecture.md"]);

        let listing = list(&root, "Inbox", &settings("markdown")).unwrap();

        assert_eq!(
            row(&listing, "lecture.pdf").result_name.as_deref(),
            Some("lecture.md")
        );
        assert_eq!(row(&listing, "lecture.mp4").result_name, None);
    }

    /// The output format decides the extension, so one folder pairs differently
    /// under `html`. Reading only `.md` offers to convert a finished file.
    #[test]
    fn the_output_format_decides_which_sibling_counts_as_the_result() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["deck.pdf", "deck.html"]);

        let markdown = list(&root, "Inbox", &settings("markdown")).unwrap();
        assert_eq!(row(&markdown, "deck.pdf").result_name, None);
        assert_eq!(markdown.pending, 2);

        let html = list(&root, "Inbox", &settings("html")).unwrap();
        assert_eq!(names(&html), ["deck.pdf"]);
        assert_eq!(
            row(&html, "deck.pdf").result_name.as_deref(),
            Some("deck.html")
        );
    }

    /// The service writes Markdown whatever the format says, so a file it
    /// converted reads as converted under every format. Pairing on the format
    /// alone leaves the row offering Convert, and every press spends.
    #[test]
    fn a_service_result_pairs_under_a_format_the_service_never_writes() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["deck.pdf", "deck.md"]);

        let listing = list(&root, "Inbox", &settings("html")).unwrap();

        assert_eq!(names(&listing), ["deck.pdf"]);
        assert_eq!(
            row(&listing, "deck.pdf").result_name.as_deref(),
            Some("deck.md")
        );
        assert_eq!(listing.pending, 0);
    }

    /// The Datalab fallback under the same route writes the chosen format, so
    /// both answers pair.
    #[test]
    fn a_fallback_result_in_the_chosen_format_pairs_on_the_same_route() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["deck.pdf", "deck.html"]);

        let listing = list(&root, "Inbox", &settings("html")).unwrap();

        assert_eq!(names(&listing), ["deck.pdf"]);
        assert_eq!(
            row(&listing, "deck.pdf").result_name.as_deref(),
            Some("deck.html")
        );
    }

    /// Direct is one writer and it writes the chosen format, so a stray `.md`
    /// beside the source is not this run's result.
    #[test]
    fn the_direct_route_pairs_on_the_chosen_format_alone() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["deck.pdf", "deck.md"]);

        let listing = list(&root, "Inbox", &direct("html")).unwrap();

        assert_eq!(names(&listing), ["deck.md", "deck.pdf"]);
        assert_eq!(row(&listing, "deck.pdf").result_name, None);
        assert_eq!(listing.pending, 1);
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

    /// Two sources want one `deck.md`. The first in sort order takes it.
    #[test]
    fn the_first_source_in_sort_order_claims_a_shared_result() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["deck.pdf", "deck.docx", "deck.md"]);

        let listing = list(&root, "Inbox", &settings("markdown")).unwrap();

        assert_eq!(names(&listing), ["deck.docx", "deck.pdf"]);
        assert_eq!(
            row(&listing, "deck.docx").result_name.as_deref(),
            Some("deck.md")
        );
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

        assert!(
            err.message.contains("outside the workspace"),
            "unexpected: {err:?}"
        );
        assert!(err.gone);
        assert!(resolve(&root, "/etc").is_err());
    }

    /// Only a folder that is really gone loses its persisted expansion.
    #[test]
    fn only_a_missing_folder_reports_itself_as_gone() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["deck.pdf"]);

        assert!(
            list(&root, "Inbox/gone", &settings("markdown"))
                .unwrap_err()
                .gone
        );
        assert!(
            list(&root, "Inbox/deck.pdf", &settings("markdown"))
                .unwrap_err()
                .gone
        );

        let denied = root.join("Inbox/locked");
        std::fs::create_dir_all(&denied).unwrap();
        std::fs::set_permissions(&denied, std::os::unix::fs::PermissionsExt::from_mode(0o000))
            .unwrap();
        let err = list(&root, "Inbox/locked", &settings("markdown")).unwrap_err();
        std::fs::set_permissions(&denied, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        assert!(!err.gone, "a folder we cannot read is still there: {err:?}");
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
        assert_eq!(
            row(&listing, "call.m4a").result_name.as_deref(),
            Some("call.txt")
        );
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

    /// The cheap question answers with the number the listing carries, or every
    /// focus looks like a change.
    #[test]
    fn the_folder_mtime_matches_the_one_its_listing_reports() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["deck.pdf"]);

        let listing = list(&root, "Inbox", &settings("markdown")).unwrap();

        assert_eq!(modified(&root, "Inbox"), Some(listing.modified_ms));
    }

    /// A gone folder, a file, and an escaping path all defer to the listing.
    #[test]
    fn nothing_that_is_not_a_folder_here_reports_an_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let root = project(&dir, &["deck.pdf"]);

        assert_eq!(modified(&root, "Inbox/gone"), None);
        assert_eq!(modified(&root, "Inbox/deck.pdf"), None);
        assert_eq!(modified(&root, "../elsewhere"), None);
    }
}
