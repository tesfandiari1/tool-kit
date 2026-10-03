# North star

**Last updated:** 2026-10-03

This document is the product target for Tool-Kit after the local-first pivot.
It was produced from a greenfield debate (workspace editor, filesystem truth,
composable tools pipeline) and deliberately ignores current implementation
details. Use it to decide what to build next and what to stop building.

---

## One sentence

Tool-Kit is a **local markdown workspace**: projects are folders, documents are
plain `.md` files you can open anywhere, and conversion/redaction/enrichment are
tools that bring content *into* the workspace — not a batch job that writes to
a folder you pick each run.

---

## What success looks like (2 years)

A researcher opens the app the way they open Obsidian or Bear — to **work on
documents they already have**, not to run a converter. Their workspace holds
hundreds of markdown files across a dozen projects. Every file arrived as a
PDF, DOCX, scan, or paste; every file is editable, searchable, and backed up with
Time Machine like any other folder on disk.

Conversion, redaction, and (optional) AI enrichment are **per-document tools**
with recorded lineage. Nothing customer-owned lives on a remote server by
default. Deleting the app or wiping app caches loses convenience (search index,
thumbnails, tool history) but **never loses content**.

**Retention signal:** most sessions start by opening an existing document, not
importing a new one.

---

## Core principles (non-negotiable)

1. **The workspace folder is home.** User picks one root at first run. All
   projects and documents live under it.

2. **Disk content is authoritative.** Markdown, sources, and project metadata
   define truth. Any database is a rebuildable index.

3. **Projects are real folders.** Creating a project creates a directory. Finder
   operations are valid; the app reconciles on focus.

4. **Documents are durable immediately, and never duplicated.** Conversion
   writes the markdown straight into the project. The original stays where it
   was, or moves into the project when the user asks. The app never copies it.
   There is no “temp until export” staging area.

5. **Export is copy-out, never move.** The project copy stays. Export writes a
   copy elsewhere and records the event. The app does not track exported copies.

6. **Identity is not the path.** Every document has a stable ID that survives
   renames and moves.

7. **Conversion leaves no server copy.** Local workers process bytes and
   discard scratch state. No customer files persist on a backend volume.

8. **Tools are uniform.** Convert, redact, and enrich share one execution model:
   inputs, params, outputs, status, provenance — even if v1 ships only convert.

---

## Resolved debates

These decisions merge three greenfield positions. Rationale is one line each.

| Question | Decision | Why |
|----------|----------|-----|
| Product shape | Document library first, not batch converter | Retention and editing are the product; conversion is an import method |
| Canonical storage | Project folder, written on import | “Temp until export” loses data and breaks backup/Finder trust |
| Metadata | `document.json` sidecar per document + minimal re-link key in markdown | Keeps exported markdown clean; sidecar holds sources and tool history |
| Index | SQLite under `.toolkit/`, fully rebuildable | Needed for search and provenance at scale; must pass “delete and rebuild” test |
| Provenance depth (v1) | Tool-run ledger in index, not full content-addressed store | CAS graph is the long-term target; v1 records runs without rewriting storage |
| Originals | Leave in place, or move into the project on request. Never copy | Two copies of every file doubles storage. The original still exists, so the lossy conversion loses nothing (reversed 2026-10-03, was copy to `_sources/`) |
| Backend role | Sandboxed local worker subprocess | Ephemeral scratch only; no durable `jobs/{uuid}/` retention in product mode |
| Cloud AI (future) | Explicit consent + egress ledger before any bytes leave | “Local-first” must be demonstrable, not marketing |
| Editor model | Markdown source + preview | WYSIWYG implies an internal AST that fights file-as-truth |

---

## Core entities

### Workspace

One user-chosen directory. Exactly one open at a time in v1.

- **On disk:** `.toolkit/workspace.json` (id, schema version)
- **App config:** security-scoped bookmark only (macOS access grant — not content)

### Project

A direct child folder of the workspace containing `project.json`.

```json
{
  "schemaVersion": 1,
  "id": "p_01k2abc123",
  "title": "Acme Acquisition",
  "createdAt": "2026-08-19T14:02:11Z"
}
```

- Flat list in v1 (no nested projects)
- `Inbox` is created automatically at first run
- Renaming the folder in Finder is allowed; identity comes from `project.json`

### Document

One markdown file plus metadata. Lives inside a project.

**Markdown file:** `{slug}.md` — plain UTF-8, user-owned content.

**Sidecar:** `{slug}.document.json` — app metadata (never required for human reading):

```json
{
  "schemaVersion": 1,
  "id": "d_01k2def456",
  "title": "Quarterly Report",
  "createdAt": "2026-08-19T21:53:00Z",
  "updatedAt": "2026-08-19T22:04:00Z",
  "content": "quarterly-report.md",
  "sources": [
    {
      "path": "/Users/someone/Desktop/quarterly-report.pdf",
      "mediaType": "application/pdf",
      "sha256": "…"
    }
  ],
  "savedCheckpoint": { "sha256": "…", "savedAt": "…" }
}
```

**Re-link key:** one optional frontmatter line so rename/move survives a missing sidecar:

```yaml
---
toolkit-id: d_01k2def456
---
```

### Import

Bringing external content into a project. The original is never copied:

1. Create document row in index (status: importing)
2. Move the original into the project, only when the user turned that on
3. Hash the original where it now sits
4. Run convert tool into `{project}/{slug}.md`
5. Write sidecar with the original's path; mark import complete

A dropped folder keeps its shape: its results nest under the folder's name.

### Tool run

One invocation of a tool against a document. Stored in `.toolkit/index.db`.

| Field | Purpose |
|-------|---------|
| `id` | Run id |
| `documentId` | Target document |
| `tool` | `convert.pdf`, `redact.pii`, … |
| `toolVersion` | For replay and staleness |
| `params` | Canonical JSON |
| `inputHash` | Body or source hash at start |
| `status` | queued / running / succeeded / failed |
| `outputKind` | `replaces-body` \| `new-document` \| `annotation` |
| `outputDocumentId` | When output is a sibling doc |
| `locality` | `local` \| `remote` |
| `startedAt`, `endedAt`, `error` | Audit |

**Dedup key:** `(documentId, tool, toolVersion, paramsHash, inputHash)`.

Convert uses `replaces-body` or creates a new document with a `derivedFrom`
link — see tool registry below.

### Export

A copy-out event. Fields: document id, destination path, format, content hash
at export time, timestamp. **Not tracked after write.** Editing after export
returns document to draft state.

---

## On-disk layout

```text
{workspace}/
├── .toolkit/                         ← DERIVED (rebuildable)
│   ├── workspace.json
│   ├── index.db                      ← SQLite: documents, imports, tool_runs, FTS
│   ├── history/{docId}/              ← optional revision snapshots
│   └── cache/thumbs/…
├── Inbox/
│   ├── project.json
│   ├── report.pdf                    ← an original, only when moved in
│   ├── report.md                     ← canonical markdown
│   └── report.document.json          ← canonical metadata
└── Acme Acquisition/
    ├── project.json
    ├── 2019 MSA.md
    ├── 2019 MSA.document.json
    ├── 2019 MSA.redacted.md          ← sibling output from redact tool
    └── 2019 MSA.redacted.document.json
```

**Canonical (user would be upset to lose):** `*.md`, `*.document.json`,
any original moved into the project, `project.json`.

**Derived (safe to delete):** everything under `.toolkit/`. The app must rebuild
a working library from a tree walk + sidecars.

**Rebuild acceptance test:** delete `.toolkit/`, relaunch, run Rebuild Index —
all projects and documents reappear with correct titles and source links.

---

## Tool registry (extension model)

Each tool declares behavior in a manifest (JSON on disk or built-in):

```json
{
  "id": "convert.pdf",
  "version": "1.0.0",
  "locality": "local",
  "network": false,
  "outputs": [
    { "role": "markdown", "placement": "new-document", "extension": ".md" }
  ]
}
```

| Tool | placement | v1 |
|------|-----------|-----|
| `convert.pdf` | `new-document` | Yes |
| `convert.docx` | `new-document` | Stretch |
| `redact.pattern` | `new-document` (sibling) | Stretch — proves second tool |
| `enrich.summarize` | `annotation` or `new-revision` | No — requires consent layer |

**Placement rules:**

- `new-document` — PDF and its markdown are different documents (sibling files)
- `new-revision` — same document, prior revision retained in history
- `annotation` — structured data attached, body untouched

---

## Converter architecture (no server retention)

```text
App → spawn sandboxed worker
    → read source where it sits (path only, no upload)
    → write scratch to $TMPDIR/toolkit/runs/{runId}/
    → validate output
    → atomic move into project/
    → delete scratch
    → record tool_run in index
```

Worker must never:

- Upload source bytes by default
- Log document content
- Persist customer bytes after job end
- Share cross-job caches of customer content

**Implementation options (pick one for v1):**

1. Embedded Rust subprocess (preferred) — PDF worker isolated for crash safety
2. Ephemeral localhost adapter — only if an existing HTTP tool must be wrapped;
   bind Unix domain socket inside scratch dir, not TCP

---

## Information architecture

### Layout

Three regions, two modes:

| Region | Content |
|--------|---------|
| **Sidebar** | Workspace name, project list (+ Inbox pinned), doc counts, Settings |
| **Center — Library** | Document list for selected project: title, source type, modified, import progress inline |
| **Center — Editor** | Markdown source + preview toggle; one document at a time in v1 |
| **Inspector (⌘I)** | Source preview, tool-run history, metadata/tags |

### Default screen

Library view for last-opened project — **not** a run queue, not an output-folder
picker.

### Absent from v1

- Jobs tab / batch queue as primary surface
- Per-run output directory
- API keys on the first screen you see
- Tabs, splits, multi-window
- WYSIWYG rich text
- Cloud sync, accounts
- Plugin API

### Key commands

| Command | Action |
|---------|--------|
| ⌘O / drop | Import into selected project |
| ⌘S | Save checkpoint |
| ⌘⇧E | Export copy |
| ⌘R | Run tool on open document |
| ⌘K | Search documents + commands |
| ⌘[ | Back to library |

---

## Document lifecycle

```text
Importing → Draft ⇄ Saved → (export event, still Draft/Saved)
                ↑                    │
                └── external edit ───┘
```

| State | Meaning |
|-------|---------|
| **Importing** | Conversion in progress; the original is untouched |
| **Draft** | Content on disk; differs from last explicit save checkpoint |
| **Saved** | User (or autosave policy) recorded a checkpoint hash |

Export does not change lifecycle state. It is an event in the sidecar’s
`lastExport` field.

**Conflict rule:** if disk changed while the app had unsaved edits, write
`{name}.conflict-{timestamp}.md` and load the external version. Filesystem
wins; the app does not silently merge.

---

## Anti-goals (v1)

Do not build these in the groundwork phase:

- Cloud sync, multi-user, accounts
- Nested projects
- Non-destructive redaction overlays (sidecar spans that aren’t in markdown)
- Two-way export tracking
- Semantic / vector search
- Windows, Linux, iOS
- Batch mode as a first-class UI (multi-import yes; “job queue product” no)
- AI enrichment without consent + egress ledger
- Backend durable storage of customer files

---

## How this gets built

Phased delivery. Each phase has a verify gate before the next starts.

### Phase 0 — Decision checkpoint (you are here)

- [ ] Accept this north star (or edit §Resolved debates)
- [ ] Pick v1 convert formats (PDF only vs PDF + DOCX)
- [ ] Confirm export semantics (copy-only)

### Phase 1 — Workspace foundation

**Build:**

- First-run workspace picker + security-scoped bookmark
- `.toolkit/workspace.json` + `index.db` schema (projects, documents)
- Project CRUD (create, rename, list, delete)
- Auto-create `Inbox`
- FSEvents: adopt Finder-created `project.json` folders

**Verify:** create workspace, add two projects, rename one in Finder, relaunch —
app shows correct projects.

### Phase 2 — Import + convert

**Build:**

- Drop / ⌘O → convert into the project, original left in place or moved in, create document + sidecar
- Sandboxed local convert worker (PDF first)
- Library list with inline import progress
- Open document → editor (read existing markdown path)

**Verify:** import 10 PDFs into Inbox; all appear as `.md` + sidecar, and no PDF exists twice;
no files remain in `$TMPDIR` after completion.

### Phase 3 — Edit + save + export

**Build:**

- Editor with autosave (atomic write: temp + rename)
- Explicit save checkpoint in sidecar
- Export copy (⌘⇧E) + Reveal in Finder
- External edit detection + conflict file rule

**Verify:** edit, save, export, edit exported copy externally, confirm project
copy unchanged.

### Phase 4 — Index hardening

**Build:**

- Tool-run table + convert tool registration
- FTS search (title + body) via `index.db`
- Rebuild Index command + CI test that deletes `.toolkit/` and recovers
- Revision snapshots (optional, under `.toolkit/history/`)

**Verify:** acceptance test from §On-disk layout (delete `.toolkit/`, rebuild).

### Phase 5 — Second tool (proves model)

**Build one of:**

- `redact.pattern` → sibling document, or
- `normalize.heading-slugs` → new revision (internal conformance tool)

**Verify:** run tool on open doc; provenance visible in inspector; output file
correctly placed.

### Phase 6 — Retire legacy surfaces

**Remove or hide:**

- Per-run output folder picker as primary flow
- Batch queue as home screen
- Backend durable job retention in product mode
- History.db “already done?” keyed only on source path (replace with
  workspace/project/content hash)

**Verify:** new user flow is workspace → project → import → edit with no legacy
prompts.

---

## v1 definition of done

One acceptance sentence:

> Import 50 mixed files into three projects, edit ten, export five, reorganize
> in Finder, delete `.toolkit/`, relaunch — everything the user cares about is
> still there, correctly titled, with sources intact.

---

## Long-term hooks (not v1)

These follow from the model but are explicitly deferred:

| Capability | Hook in v1 |
|------------|------------|
| Eval / quality | Tool-run ledger + input/output hashes |
| Staleness (“re-convert?”) | Source sha256 in sidecar vs the original's current hash |
| AI enrichment | Consent grant table + `network: true` tools |
| Full CAS provenance graph | Replace revision snapshots with content-addressed blobs |
| Cross-machine | Export/import workspace; not sync |

---

## Mapping from today’s app (informational)

This section is **not** the target — it explains what changes when execution
starts. Remove it once the pivot is complete.

| Today | North star |
|-------|------------|
| Per-run `outputDir` | Fixed workspace + active project |
| `OpenDoc.id` = output path | Stable `documentId` + path |
| `history.db` in app config | `.toolkit/index.db` in workspace |
| Backend retains `jobs/{uuid}/` | Ephemeral worker, no retention |
| Launcher-first UI | Library-first UI |
| Thread pane | Editor mode in center |
| Batch run queue | Per-document tool runs in inspector |

---

## References

Debate transcripts (internal agent ids):

- Workspace editor: `96a03280-abdb-45a8-a038-8e0728a1f7d9`
- Filesystem truth: `5b3c71f7-766c-4ad3-a809-0f5494ad6aae`
- Tools pipeline: `32ddc4ed-ec6e-4a6b-ae82-7c319b6c3044`

Operational status and milestone gates remain in [`STATUS.md`](STATUS.md).
