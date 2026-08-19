> **Superseded by [`../STATUS.md`](../STATUS.md) as of 2026-08-19.** Historical copy only.

# Closeout plan: local same-machine use

**Scope:** macOS desktop plus backend on `127.0.0.1:8080`. Convert approved
text-based inputs to Markdown, open them in the thread pane, edit, and save.
**Out of scope for this plan:** LAN deploy (M7), backend Datalab fallback (M5),
default cutover (M8), scanned PDFs, image/HTML inputs, transcription.
**Tickets and deferred milestones:** [`BACKEND_EPIC.md`](BACKEND_EPIC.md).
**Session state:** [`HANDOFF.md`](HANDOFF.md).

## Goal

A developer on one Mac can:

1. Start the converter with Compose.
2. Run the desktop app against `http://127.0.0.1:8080`.
3. Drop native PDFs and backend-supported office formats.
4. Get Markdown on disk and in the thread pane.
5. Edit and save the result.
6. Pass the text-local corpus gate and `pnpm verify:all`.

## In scope

| Input | Engine | Notes |
|---|---|---|
| Native-text PDF | `pdf-inspector` | Corpus: `native_pdf`, `dense_table_pdf`, `two_column_pdf`, `sparse_cover_pdf` |
| docx, xlsx, xls, pptx, ppt, doc, epub, odt, ods, odp, rtf, txt, csv, md, html (backend MIME) | AnyDoc | Vendored fixtures in `backend/tests/fixtures/anydoc/` |
| Profile | `local_only` or `standard` | Both run the same local path today |

## Out of scope

| Input or work | Reason |
|---|---|
| Scanned, mixed, garbled, form, blank PDFs | Finish as `needs_remote`. Need Datalab (M5) or desktop direct fallback |
| png, jpg, webp, gif, bmp, tiff, html on desktop | No local engine. Stay on direct Datalab path |
| Rev.ai transcription | Separate provider. Unchanged |
| M5, M7, M8 | Recorded in the epic. Resume when LAN deploy matters |

## Done

**Sprint 0 (repairs)** and **Sprint 1 (M4 routing policy)** are landed. Sprint 0
closed 14 defects plus two class follow-ups. Two accepted decisions stay on the
record: a timed-out AnyDoc parse ends in a supervised restart, and AnyDoc
publishes with no completeness warning because it cannot measure one. Details
and the defect table live in git history at `a7b4976` / `5fe6463`.

M4 delivered `conversion/policy.rs`, quality signals, the routing corpus, and
`tests/routing_policy.rs`. Contract is **0.4.2**.

## Sprint A: finish desktop remainder

Two Sprint 0 fixes are in the working tree but not committed. They sit in
`src/shell/App.tsx` beside concurrent UI work.

| # | Fix | Where |
|---|---|---|
| A.1 | Re-scan when route or backend URL changes | `src/shell/App.tsx` (S0.7) |
| A.2 | Retry capability probe after failure | `src/shell/App.tsx` (S0.8) |

**Gate:** commit both with the UI session or cherry-pick them. `pnpm verify`
green.

## Sprint B: same-machine stack

| # | Task | Verify |
|---|---|---|
| B.1 | Create token: `openssl rand -hex 32 > backend/secrets/bootstrap-token.txt` | File exists, mode 600 |
| B.2 | Start backend: `docker compose -f backend/compose.yaml up --build` | `curl http://127.0.0.1:8080/health/ready` → 200 |
| B.3 | Store token in Keychain via Settings | Capability preflight returns `inputFormats` |
| B.4 | Set conversion route to **backend**, URL to `http://127.0.0.1:8080`, profile **local_only** | Run button enabled for backend formats |
| B.5 | Run `pnpm tauri dev` (not the stale `.app` bundle) | Window opens, no webview bridge error |

**Trap:** `pnpm verify:all` does not rebuild `Tool-Kit.app`. Use `pnpm tauri dev`
or `pnpm tauri build` to exercise current source.

## Sprint C: text-local corpus gate

The full routing corpus has 17 PDF cases. Only **6** belong to this plan:
native (1 and 10 pages), sparse cover, dense table, two-column. The other 11
cover scans, mixed pages, garbled fonts, forms, and rejections. Keep them in
the backend suite. Do not delete them.

AnyDoc coverage already lives in
`every_advertised_anydoc_family_converts` inside `backend/tests/http_contract.rs`.

| # | Task | Verify |
|---|---|---|
| C.1 | Add `localTextOnly: true` to the 6 in-scope rows in `evals/corpus-manifest.yaml` | Manifest still passes vocabulary check |
| C.2 | Add `backend/scripts/verify-local-corpus.sh` that runs `routing_policy` plus the AnyDoc contract test | Script exits 0 |
| C.3 | Add `pnpm verify:local-corpus` in root `package.json` | One command runs the script |
| C.4 | Run `pnpm verify:all` and `pnpm verify:container` | All green |
| C.5 | Record the container smoke timestamp in `BACKEND_EPIC.md` verification log | One line, no narrative |

**Gate:** `pnpm verify:local-corpus && pnpm verify:all && pnpm verify:container`.

## Sprint D: desktop acceptance

Manual checks on the same machine. One native PDF and one AnyDoc file are enough
for a first pass. Use the vendored `text.docx` path from the contract test if
you have no sample handy.

| # | Step | Pass when |
|---|---|---|
| D.1 | Drop a native PDF, Run | Job succeeds, `.md` in output folder |
| D.2 | Open the result in the thread pane | Markdown renders |
| D.3 | Switch to edit, change text, save (⌘S) | File on disk updates |
| D.4 | Drop a docx, Run | Job succeeds via backend |
| D.5 | Stop the app mid-run, restart | Active job recovers or fails visibly |
| D.6 | Drop a scanned PDF under `local_only` | Job finishes `needs_remote` with no silent success |

Optional: add one Vitest or Tauri integration test that mocks the backend for
D.1 through D.3. Not required for this gate.

**Gate:** all six manual steps pass once.

## Sprint E: doc prune

Apply [`ste-writing`](../../.claude/skills/ste-writing/SKILL.md) **standard**
mode. Cut duplicate narrative. Keep facts, traps, and commands. Do not rewrite
code, identifiers, or quotations.

| File | Action |
|---|---|
| `docs/CLOSEOUT_EXECUTION_PLAN.md` | This file. Already trimmed |
| `docs/HANDOFF.md` | Drop adversarial-review replay. Keep traps, verify commands, ownership table |
| `docs/BACKEND_EPIC.md` | Fix status line (M4 complete, M6 in progress). Trim verification log prose |
| `docs/BACKEND_SERVICE_PLAN.md` | Fix stale "M3 in progress". Keep architecture decisions |
| `backend/README.md` | Fix stale milestone line. Keep run commands |
| `backend/evals/README.md` | Add `localTextOnly` field to the table. Keep data rules |
| `docs/archive/*` | No edits unless a live doc still links to wrong facts |

Move anything that repeats the epic ticket list out of live docs. One tracker
(`BACKEND_EPIC.md`) owns CVR status.

**Gate:** no live doc contradicts another on milestone status or scope.

## Order

```
A (desktop remainder) → B (stack) → C (corpus gate) → D (acceptance) → E (doc prune)
```

B and C can overlap across backend and desktop sessions. D needs B. E can run
in parallel with C or D but should land last so it does not prune facts that
Sprint C adds.

## Later (not this plan)

When LAN deploy matters, resume from [`BACKEND_EPIC.md`](BACKEND_EPIC.md):

- **M5:** backend Datalab fallback, `best_quality`, remote polling state machine
- **M6 close:** provenance-aware history reuse, CVR-067, CVR-081 baselines
- **M7:** Caddy, per-device tokens, host-local `/data`, backup proof
- **M8:** default cutover, soak, rollback, optional direct-path removal
