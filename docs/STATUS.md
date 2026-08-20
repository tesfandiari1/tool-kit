# Tool-Kit status

Single source of truth for the conversion backend and its desktop integration.
It supersedes the four documents now in `docs/archive/`: `HANDOFF.md`,
`CLOSEOUT_EXECUTION_PLAN.md`, `BACKEND_EPIC.md`, and `BACKEND_SERVICE_PLAN.md`. Read it before you touch the tree, then re-read
the worktree. This file goes stale the moment someone lands a commit.

**Last updated:** 2026-08-20

Docs use **standard** STE voice: American spelling, active voice, no em dashes,
no semicolons between independent sentences. Use a colon in section titles,
increment tables, and doc-map link lines. Keep one name for one thing. Do not
rewrite code, commands, identifiers, or quotations to match prose rules.

---

## 1. Current state

| Item | Value |
|---|---|
| Branch | `main`. Run `git log --oneline -8` for the tip |
| Layout | **Restructured.** `apps/{desktop,converter}`, `crates/`, `workers/`, `contract/`, `deploy/`. See CLAUDE.md |
| OpenAPI contract | **0.4.4** (`contract/http/openapi.yaml`) |
| Backend | M0, M1, M2, M3, M4 complete. M5 and M8 unbuilt. **M7 cancelled**, see section 4 |
| Desktop | M6 implementation landed, **gate open** (CVR-067, CVR-081) |
| Exposure | Loopback only, and staying there. M7 remote access is cancelled |
| Local backend | **Sidecar landed 2026-08-20, and it is now the only path the app chooses.** The converter ships inside the `.app` and `conversion_route` defaults to `backend`. `backend_host::Deployment` reads `backend-override.json`, which only a deployment writes, so Manual (Docker) is unreachable from the UI. See section 3 |
| Backend Datalab fallback | Not built. Phase 2, now unblocked |
| Phase 1 | **Complete 2026-08-19.** All four gates met |
| Desktop version | **1.0.0** (`apps/desktop/package.json`, `apps/desktop/src-tauri/Cargo.toml`, `tauri.conf.json`) |
| 1.0 bundle | **Signed, notarized and stapled 2026-08-20, and the first bundle to carry the sidecars.** Apple accepted the app (`667c0ab9`) and the DMG (`ac8c18da`), both `Ready for distribution`, no issues. A quarantined copy of the DMG answers `accepted / source=Notarized Developer ID`. Keychain entitlement stays parked until the provisioning profile lands, so `verify-release.sh` still fails that one section by design |
| Latest container smoke | `20260819T161851Z`, image `sha256:4a0cbdd0…` |

### The sidecar, 2026-08-20

The converter now ships inside the app, so a user installs one DMG and deploys
nothing.

**What landed.** `pnpm sidecars` builds the converter, the PDF worker, and the
Swift Vision worker and stages them with the `-aarch64-apple-darwin` suffix that
`bundle.externalBin` requires, plus the pdf-inspector bcmaps as a resource.
`backend_host.rs` owns the process. Two converter edits support it: `main.rs`
logs `listener.local_addr()` rather than the configured address, which is the
port handshake and also fixes a log that lied whenever Docker used port 0, and
`TOOLKIT_CONVERTER_SHUTDOWN_ON_STDIN_EOF` gives the service a way to notice its
parent died. `verify-release.sh` grew three sections that grade the sidecars.

**Measured, not assumed.** All three binaries link only the macOS base system
and the OS Swift runtime, so nothing needs bundling. Staged as siblings, the
converter finds both workers with no env var, reports engines `pdf-inspector`,
`anydoc` and `apple-vision`, and advertises **24 media types against Docker's
18**. A PNG converted through `local_vision` and a PDF through `local_pdf`, both
offline under `local_only`. SIGTERM and stdin EOF each stop it inside 250 ms,
independently. Bundle cost is 17.5 MB of Mach-O plus 1.6 MB of bcmaps.

**The one design consequence worth remembering.** The port is ephemeral, so the
recorded origin on an in-flight ledger row cannot be a URL. Sidecar rows record
the literal alias `sidecar`, and `BackendContext` still gets the live origin.
Without that, `recovery_origin_still_configured` fails on every relaunch, the
row is deleted, `recovery_blocker` is set, and nothing ever clears it, so Retry
dies permanently.

**The release path is proven, not assumed.** A signed build stamps all three
helpers `flags=0x10000(runtime)`, team `92MA44797J`, timestamped, signed
inner-out ahead of the `.app`, so `build-sidecars.sh` is right to do no signing
of its own. `verify-release.sh` passes every section but the parked keychain
entitlement. Apple notarized the app and the DMG on the first submission either
has ever made with nested binaries, with no issues raised against any helper.
The strongest check is the last one: out of a **quarantined** copy of the
installed app, the converter starts, resolves both workers through
`current_exe().with_file_name(...)`, resolves the bcmaps through
`BaseDirectory::Resource`, and reports `pdf-inspector`, `anydoc` and
`apple-vision` across 24 media types. The hardened runtime blocks none of it,
so the app needs no entitlement to spawn its own signed helpers.

**Where the service runs stopped being a setting, 2026-08-20.** The app uses
the sidecar. `backend_host::Deployment` reads `backend-override.json` beside
`settings.json`, absent means Sidecar, and `pnpm backend:docker` is the only
thing that writes it. `local_backend_mode` is gone from `Settings` and
`backend_host` no longer reads `Settings` at all. `conversion_route` now
defaults to `backend`, which is what makes "the app uses the sidecar" true
rather than nominal: it used to default to `direct`, so the sidecar ran and
converted nothing.

**Not built.** The live backend status row. `settings.backend_url` and the
Settings field that writes it are vestigial and should go together: nothing
reads the value now, and the Manual URL comes from the override file.

### The workspace and first run, 2026-08-20

The app opened on the run queue, which is a batch tool's home. It now opens on
a library. `workspace.rs` creates or adopts the one folder an install is bound
to: `.toolkit/workspace.json` is the identity marker, an Inbox project is
guaranteed, and `.toolkit/index.db` is derived state that a missing file
answers as an empty list rather than an error. Identity lives in the marker,
not the path, so renaming the folder in Finder changes nothing. Four commands
carry it: `suggested_workspace_path`, `inspect_workspace_path`,
`setup_workspace`, `list_projects`. `setup_workspace` persists the binding from
the host rather than trusting the webview, which can lose a race against
`save_settings`.

This is the second SQLite database in the app. `history.rs` still owns every
statement against `history.db`, and `workspace.rs` owns every statement against
`index.db`. One is app state, the other is derived state a user may delete.

First run asks three questions, and `conversionMode.ts` maps the third onto the
three settings it really is, which lets the copy say "this Mac" while the app
goes on speaking in routes and profiles. Local means `local_only` rather than
`standard` with a preference, because choosing this Mac has to mean the bytes
cannot leave it.

**Not built.** Nothing imports into a project yet, so the library lists what
onboarding created and no more. The gate writes `conversionRoute: backend` with
no Settings control to change it back, which is the same missing surface the
sidecar section names.

### The restructure, 2026-08-19

Five commits moved every application into `apps/` and lifted the shared and
platform-specific pieces out of the converter. The call graph is unchanged:
`run_pipeline` reported 88 impacted symbols over 28 processes before and after.

| Commit | Content |
|---|---|
| `58d24e2` | CLAUDE.md describes the tree, and stops claiming a missing icon source |
| `26ec239` | The frontend and both crates fold into `apps/` |
| `2e94a99` | Contract, deployment, and the Vision worker leave `backend/` |
| `f99b9ef` | One protocol crate for every spawned worker |
| `8b23c46` | M7 closed out, ignore rules land ahead of what they cover |

`packages/ui` was considered and skipped: `UI.md` names the trigger as a second
client and there is not one, the library has no app imports to catch, and the
move would have dropped three test files out of Vitest's scope silently.
`workers/pdf` is deferred because `tool-kit-pdf-worker` is a `[[bin]]` reached
through `CARGO_BIN_EXE_tool-kit-pdf-worker`, which only resolves inside its own
package, so it is a crate extraction rather than a move.

The sprint that closed Phase 1, newest first. This is a snapshot, not a
running log. `git log` is the source of truth.

| Commit | Content |
|---|---|
| `f38d70a` | Phase 1 gate 3 closed, all six acceptance steps pass |
| `ceca3f2` | 20 confirmed claim mismatches reconciled against the tree |
| `40c706d` | The unused `Path` `max` knob removed |
| `abe5c2a` | Gate scripts assert their own results |
| `f1fe307` | Transient boot errors contained, dead policy surface dropped |
| `34c0949` | The remote ledger state a retry rebuilt bare, restored |
| `a337594` | Consolidation of every planning doc into this file |

**Sprint 0, Sprint 1, Sprint A, and the Phase 1 closeout are done.** `scanKey`
in `apps/desktop/src/shell/App.tsx` includes `conversionRoute` and `backendUrl` (S0.7). The
capability probe retries on an interval instead of latching Run off for the
session (S0.8).

The closeout audited every falsifiable claim in this file against the tree. It
confirmed 20 mismatches and refuted 4. Seven of them were code, not prose, and
section 9 lists all seven. The lesson to carry into M5: each one was already
marked done, and no gate caught any of them. Only reading the code against the
claim did.

**`d6fc6eb` is mistitled.** Its message reads "docs: add comprehensive backend
and desktop execution plans". It contains six `git mv` renames into
`docs/archive/` and nothing else.

### What the product does today

- Conversion defaults to the direct Datalab route. Transcription uses Rev.ai
  and is unchanged.
- A user may select the local backend. Run is planned per file from live
  `capabilities.inputFormats`, so the desktop never hardcodes the backend's
  proven format set.
- Image and HTML formats stay permanently direct. `local_only` rejects anything
  that would require Datalab.
- Keys stay in the macOS Keychain. The webview sees neither bearer values nor
  document bytes.
- Backend jobs persist a client run ID, idempotency key, original backend URL,
  source mtime, profile, and optional backend job ID before submission. Restart
  recovery replays or resumes against the recorded URL.
- Submit and poll validate UUID response identity. Poll also requires the
  response ID to equal the requested job before any artifact downloads.
- **Backend-mode history reuse and copy are disabled, by two different
  mechanisms.** Terminal backend jobs file under `output_format`
  `backend:markdown` (`jobs.rs:407`) or `backend_fallback:{format}`
  (`jobs.rs:409`). `run_pipeline` skips the lookup outright, behind an explicit
  `jt == Convert && route == Backend` guard (`lib.rs:549`). `split_reusable`
  carries no such guard (`lib.rs:351`), and instead never matches, because it
  passes `output_format_for(job_type, cfg)`, which returns the Datalab format
  string. Both paths end in "every file runs again", which is safe, so the two
  shapes only matter to whoever writes S2.3: one call site needs its guard
  removed, the other needs a provenance-aware format.
- **No gate rebuilds `Tool-Kit.app`.** `pnpm verify:all` runs `tsc`, ESLint,
  Vitest, `vite build`, and both Clippy and test suites. None produce an app.
  Use `pnpm tauri dev` to exercise current source.
- The desktop does not render in a plain browser, and that is expected.
  `useWindowFocusClass` and `useCloseConfirm` call `getCurrentWindow()` in a
  mount effect. With no `window.__TAURI_INTERNALS__` that throws and React 19
  tears the tree down. Use `pnpm tauri dev`, or `?gallery` for the bridge-free
  design review.

---

## 2. Phase 1: local same-machine use. Complete

**Closed 2026-08-19.** All four gates below are met. Keep this section as the
record of what Phase 1 covered and how to re-run its checks. Phase 2 is the
active phase.

**Scope:** macOS desktop plus the converter on `127.0.0.1:8080`. Convert
approved text-based inputs to Markdown, open them in the thread pane, edit, and
save.

**Out of scope:** LAN deploy (Phase 3), backend Datalab fallback (Phase 2),
cutover (Phase 4), scanned PDFs, image and HTML inputs, transcription.

### Goal

A developer on one Mac can start the converter with Compose, run the desktop
app against `http://127.0.0.1:8080`, drop native-text PDFs and backend-supported
office formats, get Markdown on disk and in the thread pane, edit and save it,
and pass the text-local corpus gate plus `pnpm verify:all`.

### The 20 backend extensions

`SOURCE_FORMATS` in `apps/converter/src/conversion/model.rs` is the one admission
table. It drives extension, media-type, and magic checks at upload, engine
selection at execution, and the capabilities `inputFormats` list. It carries
**20 extensions over 18 media types**: PDF on its own isolated worker plus 19
AnyDoc extensions over 17 media types.

| Family | Extensions | Engine |
|---|---|---|
| PDF | `pdf` | `pdf-inspector` child worker |
| Word | `doc`, `docx`, `docm` | AnyDoc |
| PowerPoint | `ppt`, `pps`, `pot`, `pptx`, `pptm`, `ppsx`, `ppsm` | AnyDoc |
| Excel | `xls`, `xlsx`, `xlsm` | AnyDoc |
| OpenDocument | `odt`, `ods`, `odp` | AnyDoc |
| Other | `epub`, `rtf`, `csv` | AnyDoc |

`pps` and `pot` share `application/vnd.ms-powerpoint` with `ppt`, so the media
type list is 18 while the extension list is 20.

**`txt`, `md`, and `html` are not in the table and the backend rejects them.**
Earlier plan text listed them as in scope. That was wrong. AnyDoc's `Format`
enum has no HTML variant, and plain text and Markdown were never advertised.

**`xlsb` is the one deliberate omission.** AnyDoc maps it to the Excel parser
and `calamine` has binary-workbook support compiled in, but no upstream fixture
exists and a binary workbook cannot be honestly derived from an XML one.
Advertising it would break the rule that earns this table its trust: a format
appears only with a fixture that proves it.

**Ten desktop extensions have no local engine**: `png`, `jpg`, `jpeg`, `webp`,
`tiff`, `tif`, `gif`, `bmp`, `html`, `htm`. They belong to the remote route
(Phase 2) or the desktop's per-file direct fallback. Never to the backend.

### Size caps

| Cap | Value | Where |
|---|---|---|
| Backend upload ceiling | **25 MiB** (`26214400`) | `DEFAULT_MAX_UPLOAD_BYTES`, `apps/converter/src/config.rs` |
| Backend output ceiling | 50 MiB (`52428800`) | `TOOLKIT_CONVERTER_MAX_OUTPUT_BYTES` |
| Desktop preview and open | **2 MiB** | `MAX_PREVIEW_BYTES`, `apps/desktop/src-tauri/src/lib.rs` |
| Backend manifest read | 2 MiB | `MAX_MANIFEST_BYTES`, `apps/converter/src/conversion/service.rs` |

A source over 25 MiB fails at upload. A result over 2 MiB converts and lands on
disk but refuses to open in the thread pane with "File is too large to
preview". Acceptance should not use files near either limit.

### Setup

| # | Task | Verify |
|---|---|---|
| B.1 | `openssl rand -hex 32 > deploy/docker/secrets/bootstrap-token.txt` | File exists, mode 600 |
| B.2 | `docker compose -f deploy/docker/compose.yaml up --build` | `curl http://127.0.0.1:8080/health/ready` returns 200 |
| B.3 | Store the token in the Keychain through Settings | Capability preflight returns `inputFormats` |
| B.4 | Set route **backend**, URL `http://127.0.0.1:8080`, profile **`local_only`** | Run enabled for backend formats |
| B.5 | `pnpm tauri dev`, not the stale `.app` bundle | Window opens, no webview bridge error |

**Use `local_only` for the first run.** The persisted default is `standard`,
and `standard` today falls through to the existing desktop Datalab lifecycle
for anything the backend declines. A first acceptance pass under `standard`
spends Datalab credits on every scanned or unsupported file and hides whether
the backend handled the ones it claimed. `local_only` never sends bytes
externally, so a backend gap shows up as `needs_remote` instead of as a silent
paid success.

**Compose token trap.** `TOOLKIT_CONVERTER_TOKEN_FILE` appears twice in
`deploy/docker/compose.yaml` with two different meanings. Under `environment:` it is
the in-container path `/run/secrets/bootstrap_token`. Under `secrets:` it is a
host-side substitution, `${TOOLKIT_CONVERTER_TOKEN_FILE:-./secrets/bootstrap-token.txt}`.
`apps/converter/.env.example` sets it to an absolute host path. If you export it in
your shell or place a `.env` beside `compose.yaml` that sets it to the
container path, Compose looks for the host secret file at
`/run/secrets/bootstrap_token` and the stack fails to start. Leave it unset on
the host and let the default apply.

### Corpus gate

The routing corpus has 17 PDF cases. **Five belong to Phase 1:**
`native-text-1-page`, `native-text-10-pages`, `sparse-cover-page`,
`dense-table`, `two-column-text`. The other 12 cover scans, mixed pages,
garbled fonts, forms, blanks, and rejections. Keep them in the backend suite.
Do not delete them.

AnyDoc coverage already lives in `every_advertised_anydoc_family_converts`
inside `apps/converter/tests/http_contract.rs`.

| # | Task | State |
|---|---|---|
| C.1 | Name the five case IDs in `verify-local-corpus.sh`, assert the manifest carries them | Done. See the note below |
| C.2 | `apps/converter/scripts/verify-local-corpus.sh` runs `routing_policy` plus the AnyDoc sweep | Done |
| C.3 | `pnpm verify:local-corpus` in the root `package.json` | Done |
| C.4 | Run `pnpm verify:all` and `pnpm verify:container` | Both green 2026-08-19 |
| C.5 | Record the container smoke timestamp in section 9 | Done (`20260819T161851Z`) |
| C.6 | `apps/converter/scripts/print-corpus-pdf.sh` plus `write_native_pdf_fixture_to_env` in `corpus.rs` | Done |

**C.1 does not filter, on purpose.** `verify-local-corpus.sh` runs the whole
`routing_policy` suite, which is a superset of the five, so filtering would buy
nothing and would hide the other 12 cases. The five are named in the script and
checked against the manifest before the suites run, because a printed list that
nothing verifies goes stale the first time a case is renamed. Adding a manifest
field was rejected: `policy.rs` parses the manifest with
`deny_unknown_fields`.

### Acceptance checklist

**All six pass as of 2026-08-19.** D.1 and D.4 are reproducible against the
running container. D.2, D.3, D.5, and D.6 passed in one `pnpm tauri dev`
session. Generate inputs with `apps/converter/scripts/print-corpus-pdf.sh` if you have
no sample handy.

| # | Step | Pass when |
|---|---|---|
| D.1 | Drop a native PDF, Run | Job succeeds, `.md` lands in the output folder |
| D.2 | Open the result in the thread pane | Markdown renders |
| D.3 | Switch to edit, change text, save with ⌘S | The file on disk updates |
| D.4 | Drop a docx, Run | Job succeeds through the backend |
| D.5 | Stop the app mid-run, restart | The active job recovers or fails visibly |
| D.6 | Drop a scanned PDF under `local_only` | Job finishes `needs_remote` with no silent success |

**D.6 evidence, from a real scanned document.** The ledger row recorded
`output_format` `backend:markdown`, `status` `failed`, an empty `output_path`,
and the error "Local-only conversion could not finish locally. Choose Standard
to allow Datalab fallback." No Markdown reached the output folder. That is the
whole point of the step: a scan the backend will not convert must refuse
visibly rather than publish partial text or bill Datalab behind the profile.

`needs_remote_decision` is the only gate, `BackendAction::NeedsRemote` is the
only path into `run_datalab_fallback`, and
`needs_remote_falls_back_only_for_standard` pins both.

**`standard` will fall back and spend credits, in the same session.** One run
under `standard` published `backend_fallback:markdown` a minute before the
`local_only` run refused. Set the profile before the run, not after: the run
config is snapshotted at start.

### Running D.2, D.3, D.5, and D.6

One `pnpm tauri dev` session covers all four. Do not use the bundled `.app`.
No gate rebuilds it.

```bash
docker compose -f deploy/docker/compose.yaml up -d   # skip if already healthy
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/health/ready
apps/converter/scripts/print-corpus-pdf.sh /tmp/toolkit-native.pdf
apps/converter/scripts/print-corpus-pdf.sh /tmp/toolkit-scanned.pdf scanned
pnpm tauri dev
```

In Settings paste the token from `deploy/docker/secrets/bootstrap-token.txt`, set
route **backend**, URL `http://127.0.0.1:8080`, and profile **`local_only`**.

| # | Do this | Pass when |
|---|---|---|
| D.2 | Drop `/tmp/toolkit-native.pdf`, Run, click the finished row | The window grows to workspace phase and Markdown renders as prose |
| D.3 | Switch to edit, change a word, press ⌘S | That word is in the `.md` on disk |
| D.5 | Drop both PDFs and a docx, Run, quit from the tray on an amber row, relaunch | Every job reaches a terminal state. Run is not stuck disabled |
| D.6 | Drop `/tmp/toolkit-scanned.pdf`, Run | The row ends `needs_remote` and says so. No `.md` is written |

**D.5 must quit from the tray.** The close button only hides the window, so it
never exercises restart recovery.

Read the ledger across a D.5 restart with:

```bash
sqlite3 ~/Library/Application\ Support/dev.esfandiari.toolkit/history.db \
  'select idempotency_key, fallback_provider, fallback_request_id
     from inflight_conversions;'
```

A row with a provider and no request id is the uncertain-fallback state. The
app must refuse to resubmit it rather than risk a second charge.

**Proven 2026-08-19 against the deployed container on `127.0.0.1:8080`,
not against a test harness:**

- D.1: native PDF, `202` then `succeeded`, route `local_pdf`, reason codes
  `[native_text_pdf]`, Markdown `200` at 1478 bytes.
- D.4: docx, `202` then `succeeded`, route `local_anydoc`, reason codes
  `[structured_document]`, Markdown `200` at 1316 bytes.

- D.6 backend half: scanned PDF under `local_only`, `202` then
  `needs_remote`, route `local_pdf`, reason codes `[scanned_pdf]`, no artifact.
  `routing_policy` pins the same case as `scanned-1-page-local-only`. Only the
  desktop rendering of that state is unproven.

Submitting by hand takes three multipart fields, `profile`, `clientRunId`, and
`source`, `clientRunId` must be a UUID, and `source` must carry an explicit
`type=` that matches the extension. curl's default guess is rejected with
`invalid_source_media_type`.

Optional: one Vitest or Tauri integration test that mocks the backend for D.1
through D.3. Not required for this gate.

### Phase 1 gates

1. `pnpm verify` green with Sprint A on `main`. **Done.**
2. `pnpm verify:local-corpus && pnpm verify:all && pnpm verify:container` all
   green. **Done 2026-08-19.** See the verification log in section 9.
3. All six acceptance steps pass once. **Done 2026-08-19.**
4. Archive the four superseded docs. No live doc contradicts another on
   milestone status or scope. **Done.** The docs are in `docs/archive/`, the
   `apps/converter/README.md` milestone line reads M4, and `apps/converter/evals/README.md`
   carries a "Phase 1 local-text cases" section. That section replaced an
   earlier plan to add a `localTextOnly` manifest field, which C.1 rules out.

Phase 1 passed acceptance, so backend Datalab work may start. S2.0 is the
first sprint.

---

## 3. Phase 2: M5 Datalab fallback and M6 close. Active

**Highest-complexity milestone.** Uncertain billable submission and
restart-safe polling are where production bugs live.

**Port the desktop taxonomy from `apps/desktop/src-tauri/src/providers.rs` verbatim.**
Especially `send_retrying()`, which retries only failures that prove the server
never started work, `terminal_poll_error()`, and the rule that a result-fetch
5xx or 429 stays transient after the work is already billed. Do not redesign
Datalab semantics in the backend.

M6 per-file fallback to the direct Datalab path may ship before M5. Treat
backend Datalab as an optimization, not a desktop blocker.

| Sprint | Content | Tickets |
|---|---|---|
| S2.0 | Typed Datalab config and the bounded submit, poll, and result adapter | CVR-050, CVR-051 |
| S2.1 | Durable remote state, persisted request ID, uncertain acceptance | CVR-052, CVR-053, CVR-054 |
| S2.2 | Apply the M4 policy, publish remote Markdown, full failure matrix | CVR-055, CVR-056, CVR-057 |
| S2.3 | Close M6: provenance-aware history reuse, fallback durability, baselines | CVR-067, CVR-081 |

### S2.3: provenance-aware history reuse

Backend results file under `backend:markdown` and Datalab fallbacks under
`backend_fallback:{format}`. The matcher in `apps/desktop/src-tauri/src/lib.rs` calls
`history::reusable(app, files, jt.id(), &jobs::output_format_for(jt, cfg))`,
which produces the plain Datalab format string and never selects a backend row.

The fix teaches the matcher about provenance, so a backend result satisfies a
backend-route re-run and a direct result satisfies a direct-route re-run,
without either substituting for the other. Until it lands, backend mode always
re-runs. That costs time and never costs correctness, which is why it ships
after M5 rather than before.

### Phase 2 gates

- **M5:** difficult fixtures route correctly, `local_only` inputs never leave
  the network, and a known or uncertain remote request is never duplicated.
- **M6:** the M5-dependent CVR-067 fallback-durability scenario passes, the
  CVR-081 direct-path baselines are captured, and history reuse selects by
  provenance.

---

## 4. Phase 3: M7 remote deployment. Cancelled

**Cancelled 2026-08-19, unstarted.** Adopting
[`north-star.md`](north-star.md) made the product one local workspace on one
Mac, and the app now runs engines the container cannot host at all. Remote reach
to a service that does less than the app is not worth hardening. The sprint plan
moved to [`archive/M7_EXECUTION.md`](archive/M7_EXECUTION.md). Everything below
is the record of what was planned, not work to do.

**Revised 2026-08-19. The exposure decision changed.** The service must now be
reachable from anywhere in the world, not only from the LAN. Caddy with internal
HTTPS answered the LAN requirement and does not answer this one, so Tailscale
replaces it. Section 7 records every consequence.

The host is a TrueNAS box treated as a plain Docker host. TrueNAS apps, its
catalog, and its ZFS tooling are out of scope. Docker and Compose are the
interface. The one host fact that still binds is the rule in section 7: the live
SQLite database stays on a host-local filesystem and never on SMB or NFS.

### The exposure decision: Tailscale

Run a `tailscale` sidecar with `tailscale serve` and join the converter to it
through `network_mode: service:tailscale`. The converter publishes no port. The
sidecar terminates TLS on a MagicDNS name and carries every request.

| Option | Outcome |
|---|---|
| **Tailscale** | **Selected.** No body cap, no proxy read timeout, works behind CGNAT over DERP, no domain, no certificate plumbing |
| Cloudflare Tunnel plus Access | Runner-up. Same CGNAT immunity, but it imports a 100 MB proxied request body limit on Free and Pro and a fixed origin read timeout near 100 seconds. Neither moves below Enterprise |
| Public Caddy plus Let's Encrypt | Rejected. CGNAT removes the inbound port, which removes both port forwarding and the HTTP-01 challenge. DNS-01 would issue a certificate for an origin nothing can reach |

Uploads are capped at 25 MB today, so the Cloudflare limits fit. They stop
fitting the moment the cap rises, a synchronous convert path appears, or
progress moves off polling. Tailscale removes the numbers instead of fitting
inside them. Keep Cloudflare Tunnel in reserve for one case: reaching the API
from a device that cannot run Tailscale.

**Tailscale is transport identity, not API authentication.** The bearer token
still travels over the tailnet, and the token checks do not relax.

### What Tailscale changes about the ticket order

Only tailnet devices can reach the service at all, so a leaked bearer token is
exploitable only by someone already inside the tailnet. That moves CVR-071 from
a prerequisite to hygiene, and it moves CVR-075 to the front. `/data` holds the
only copy of every converted document, there is no retention or cleanup job by
design, and no gate has ever proven a restore.

The unauthenticated `/health/ready` flood finding from M2 Increment 5 stays
open and stays real, because the tailnet is a smaller audience and not an empty
one. It is no longer a release blocker.

### Tickets

| Ticket | Content |
|---|---|
| CVR-070 | **Revised.** Tailscale sidecar with `tailscale serve`, converter on `network_mode: service:tailscale`, no published port on the converter |
| CVR-071 | Rotatable per-device credentials replacing the bootstrap token, with provisioning docs |
| CVR-072 | **Revised.** Converter unreachable except through the sidecar, non-root, narrow mounts, dropped capabilities, read-only root, resource limits, bounded logs. Do not use `internal: true` |
| CVR-073 | SQLite, artifacts, and Tailscale state on explicit host-local datasets, live database off SMB and NFS |
| CVR-074 | Retention, conservative orphan cleanup, disk-space readiness, redacted operational metrics |
| CVR-075 | Documented and proven coordinated backup and restore for SQLite plus artifacts |
| CVR-079 | **New.** Image build caching and registry release: cargo-chef, GHCR, sha-pinned production Compose override |

### Sequence

**The sprint-level plan is archived at [`archive/M7_EXECUTION.md`](archive/M7_EXECUTION.md).** It holds
the file-level steps, the verify command for each sprint, and the commit
boundaries. This table is the summary. Where the two disagree, the execution plan
is newer.

Two orderings below were revised after the plan read the code. A shell over the
tailnet on the **host** has to exist before the converter's published port goes
away, because a crash-looping converter never binds a listener and the sidecar
answers with a proxy error. And CVR-079 left M7 entirely: its rollback value is
zero across a migration boundary, which is the only rollback that matters here.

Item 0 comes before anything else touches the host. Items 1, 2, and 4 are
independent of each other and of the exposure work.

| # | Work | Ticket | Verify |
|---|---|---|---|
| 0 | Backup and proven restore | CVR-075 | Restore into a scratch volume, boot the container against it, assert a known job id resolves and its artifact hashes match the manifest |
| 1 | cargo-chef dependency layer | CVR-079 | Edit one line under `src/`, rebuild, observe the dependency layers cached |
| 2 | GHCR release and pinned override | CVR-079 | `docker compose -f compose.yaml -f compose.prod.yaml config` resolves to a pinned sha |
| 3 | Tailscale sidecar | CVR-070, CVR-072 | Reachable from a phone on cellular. `curl` from another LAN host fails |
| 4 | Config surface and drift test | CVR-074 | Change one knob in `.env`, `docker compose up -d`, observe the new value in the boot config line |
| 5 | Stats log line, disk readiness, Dozzle | CVR-074 | Fill `/data` past the floor, observe `/health/ready` report not ready |
| 6 | Per-device credentials | CVR-071 | Issue two, revoke one, observe the revoked one rejected without a restart |

### Decisions inside the tickets

**CVR-075 backup order is forced by the write ordering.** Artifacts land before
the row is marked `succeeded`, so snapshot the database first with
`VACUUM INTO` and the artifact tree second. Reversed, a restore holds rows
claiming success over bytes that were never captured. Orphan files are
recoverable, dangling rows are not. `cp converter.sqlite` on a live WAL database
drops every commit still in the `-wal` and can tear pages mid-copy.

**CVR-079 uses cargo-chef, not BuildKit cache mounts.** A cache mount on
`/usr/local/cargo/registry` exists only during the `RUN` that declares it, and
the Dockerfile's `find` for the `pdf-inspector-1.15.0` source runs after the
build to copy out its bcmaps. A cache mount would empty `PDF_INSPECTOR_SOURCE`
and fail the build. cargo-chef leaves the registry in a real layer, which also
survives a cold CI runner. Build `linux/amd64` only: the deploy host is x86_64,
and `codegen-units = 1` with thin LTO under QEMU emulation is untenable.

**CVR-074 exports no `/metrics` endpoint.** One container with one worker does
not justify a second listener or a time-series database. Emit the operational
numbers as one JSON log line on an interval from `jobs/`: queue depth, seconds
since the last successful conversion, outcome counts by status, conversion
duration, and free bytes on `/data`. Read free space with `statvfs` rather than
trusting the readiness write transaction, which can stay green while a 25 MB
upload can no longer land.

**CVR-071 stores `HMAC-SHA256(pepper, secret)`, not Argon2.** A 256-bit CSPRNG
secret is bounded by entropy, not by hash speed, so a slow KDF buys nothing and
spends 50 to 100 ms of CPU on the unauthenticated path of a one-worker CPU-only
box. That is a self-inflicted denial of service. Password hashing needs a slow
KDF because a password carries 20 to 30 bits and is guessable. A random token
is not.

**CVR-071 has one open correctness question.** An authentication scope plus
hashed idempotency key is unique on `conversions`, and the scope is the literal
`bootstrap` today. Per-device scopes make it vary. Resolve what that uniqueness
means across a credential rotation before writing the migration, or a rotation
silently changes replay behavior.

### Traps

- **`internal: true` blocks egress, not only ingress.** The converter will need
  outbound HTTPS for the M5 Datalab adapter. An internal network fails its DNS
  and TLS with no clear error. The shared network namespace already denies
  inbound reach, so no internal network is needed.
- **`network_mode: service:` forbids `ports:` and `networks:` on that service.**
  Move the published port to the sidecar or `compose up` fails.
- **Persist `/var/lib/tailscale`.** Without it each restart registers a new node
  and the MagicDNS name drifts to a `-1` suffix.
- **Set `TS_USERSPACE=true`** so the sidecar needs neither `NET_ADMIN` nor
  `/dev/net/tun`, and cannot rewrite the shared namespace's default route.
- **Rollback across a migration is not free.** Migrations run forward at startup
  and `migrations/` holds no down files. SQLx refuses to start when the database
  records a version the binary does not embed, so rolling the tag back across
  `0003` requires restoring the pre-deploy snapshot. Inside one migration set,
  retag and restart is clean.
- **Dozzle needs the Docker socket, which is root on the host.** Pin 9.0.3 or
  newer: CVE-2026-24740 lets a label-filtered user open a root shell in an
  out-of-scope container. Never publish it on `0.0.0.0`.
- **`env_file:` does not feed Compose interpolation.** Only the project `.env`
  does. A knob set only in `env_file` silently falls back to its default.

**M7 gate:** a restore has been performed and asserted, the converter has no
published port and answers only over the tailnet, and the host passes
security-boundary, recovery, and overload tests.

---

## 5. Phase 4: M8 evaluation and cutover

CVR-081 moved to M6, so baselines are captured before cutover rather than
after. CVR-080 overlaps the M4 corpus freeze. Run it once when M4 fixtures are
stable, not twice.

| Ticket | Content |
|---|---|
| CVR-080 | Freeze the labeled corpus and expected routing results for release comparison |
| CVR-082 | Full desktop-to-backend acceptance suite plus a real multi-document soak on the deployed host |
| CVR-083 | Make the backend route the desktop default through a reversible configuration change |
| CVR-084 | Exercise rollback and verify unfinished jobs and user files stay understandable and recoverable |
| CVR-085 | Record release evidence, known limitations, restore point, and operator sign-off |
| CVR-086 | Remove the obsolete direct conversion path, only in a separately approved cleanup after the rollback window |

**M8 gate:** the backend is the verified default, rollback has been exercised,
and legacy code is removed only after a separate approval.

---

## 6. Milestone record

| Milestone | Outcome | Status |
|---|---|---|
| M0 | Architecture, ownership boundaries, and scaffold | Complete |
| M1 | Authenticated loopback PDF conversion vertical slice | Complete |
| M2 | Durable SQLite jobs, sources, and artifacts | Complete |
| M3 | AnyDoc and proven non-PDF local conversion | Complete |
| M4 | Corpus-calibrated routing and quality policy | Complete, CVR-043 open by design |
| M5 | Restart-safe Datalab fallback and privacy policy | Planned |
| M6 | Desktop app uses the backend | Implementation landed, gate open |
| M7 | Remote deployment, operations, and recovery | Planned |
| M8 | Evaluation, reversible cutover, and cleanup | Planned |

### M0: architecture and boundaries. Closed

- [x] CVR-001: record the dirty-worktree baseline and backend-owned paths.
- [x] CVR-002: independent `apps/converter/` crate, not a root workspace.
- [x] CVR-003: select Rust/Axum as the production foundation.
- [x] CVR-004: modular monolith, one container, one in-process worker, parser
  child-process isolation.
- [x] CVR-005: embedded SQLite plus a host-local `/data` dataset.
- [x] CVR-006: `pdf-inspector` for PDF, AnyDoc for non-PDF, Datalab as fallback.
- [x] CVR-007: remove FastAPI, PostgreSQL, Redis, GPU, and speculative
  microservices from the baseline.
- [x] CVR-008: define extraction triggers instead of scaffolding unused services.

### M1: loopback PDF vertical slice. Closed

- [x] CVR-010: standalone crate, typed fail-fast config, structured logs,
  request IDs, stable errors.
- [x] CVR-011: liveness, readiness, capabilities, submission, status, and
  artifact routes plus the OpenAPI contract.
- [x] CVR-012: bootstrap bearer auth, secret-file loading, constant-time compare.
- [x] CVR-013: bounded multipart streaming, signature and field validation,
  process-local idempotency, upload backpressure.
- [x] CVR-014: pin `pdf-inspector = 1.15.0`, default features off, wrapped in
  `tool-kit-pdf-worker`.
- [x] CVR-015: worker identity and CMap checks, environment clearing, parser
  timeout, output limits, protocol validation, one-parser concurrency.
- [x] CVR-016: return `needs_remote` for scanned, image-based, mixed,
  encoding-damaged, OCR-required, empty, or oversized local output.
- [x] CVR-017: validate and atomically publish non-empty Markdown plus a
  versioned provenance manifest.
- [x] CVR-018: hardened multi-stage image and loopback-only Compose service.
- [x] CVR-019: unit and HTTP-contract coverage across auth, uploads,
  idempotency, routing, parser failure, publication, and OpenAPI.

**Gate met 2026-08-17.** All 28 Rust tests passed.

### M2: durable SQLite jobs and artifacts. Closed

- [x] CVR-020: SQLx 0.9, default features off, embedded migrations, `build.rs`
  tracking, foreign keys, WAL, `synchronous=FULL`, busy timeout.
- [x] CVR-021: fail-fast `/data` config and the persistent layout for the fixed
  `converter.sqlite` filename, sources, attempts, artifacts, and quarantine.
- [x] CVR-022: replace the in-memory `JobRegistry` with a repository plus
  durable idempotency under scope `bootstrap`.
- [x] CVR-023: durable submission. Commit source, conversion, first attempt, and
  idempotency before `202 Accepted`. Remove the staged source on replay,
  conflict, capacity rejection, or transaction failure.
- [x] CVR-024: one bounded worker loop with transactional claims, notify for
  wake-up, polling for restart recovery.
- [x] CVR-025: persist every transition, attempt, route, classification,
  warning, failure, engine version, artifact path, and hash.
- [x] CVR-026: startup reconciliation for `queued`, `converting_local`,
  `finalizing`, and successful jobs, with orphan quarantine.
- [x] CVR-027: live readiness, bounded graceful shutdown, truthful capabilities,
  matching OpenAPI update, Compose `/data` mount.
- [x] CVR-028: test idempotency, restart, recovery, interrupted publication,
  corrupt artifacts and sources, database and disk failure, quarantine,
  deterministic crash windows, and the `artifact_integrity_failed` contract.
- [x] CVR-029: full Rust checks, Compose validation, image build, graceful and
  forced-kill restart smokes.

**Gate met.** An accepted job, source, and idempotency record survive a full
restart. A recovered job cannot execute concurrently or report success without
validated, downloadable artifacts.

### M3: AnyDoc and local format routing. Closed

- [x] CVR-030: pin `anydoc =0.1.9`. MIT, `rust_version` 1.88, pure-Rust
  dependency tree, in-process `to_markdown_bytes` with content-based detection.
- [x] CVR-031: `engines/outcome.rs` carries `EngineAnalysis` plus the shared
  `EngineOutcome` and `EngineFailure`. No universal document AST.
- [x] CVR-032: AnyDoc adapter, **in-process by owner decision**.
  `to_markdown_bytes` under `spawn_blocking` plus `catch_unwind`. A parser panic
  becomes `worker_crash`, recoverable like any interrupted attempt. The adapter
  refuses PDF bytes outright, so AnyDoc's embedded pdf-inspector can never
  bypass the isolated PDF worker.
- [x] CVR-033: one admission table drives upload validation, engine selection,
  and `inputFormats`. Detection is content-based inside the engine, admission
  checks container magic.
- [x] CVR-034: PDFs route exactly once through `pdf-inspector`, non-PDFs exactly
  once through AnyDoc. A queued row with an engine-less media type fails closed
  instead of poisoning the queue.
- [x] CVR-035: persist engine, version, warnings, fallback reason, output hash,
  and diagnostics without leaking document content. **Reopened and closed in
  Sprint 0:** `warnings` was hardcoded empty at both write sites, so nothing an
  engine reported as a caveat reached the user. The M4 policy populates it.
- [x] CVR-036: PDF plus 19 AnyDoc extensions, each with a round-trip fixture and
  each family with a bounded-failure fixture. Licenses in
  `apps/converter/tests/fixtures/SOURCES.md`.
- [x] CVR-037: container and contract gates re-run with AnyDoc present, plus the
  AnyDoc hard outer timeout. Smoke `20260818T192759Z`, image `sha256:cd989ed8…`.
  Hang and panic wrapper tests pass.
- [x] CVR-038: AnyDoc containment policy recorded. See section 11.
- [ ] CVR-039: skip the redundant source re-read and hash at parse. Optional and
  unscheduled. See section 11.

**Gate met 2026-08-18.** Every advertised extension has a passing conversion
fixture. Each source is parsed by exactly one engine chosen from the stored
media type. Empty output, typed engine errors, the output ceiling, and a
cross-family mislabel all fail closed with no artifact published. The container
smoke converts a docx inside the image and asserts `local_anydoc`, so "smokes
pass with AnyDoc present" means AnyDoc ran, not merely that it linked.

**Two accepted decisions, on the record:**

1. **A timed-out AnyDoc parse ends in a supervised restart, not in-process
   containment.** The adapter wraps the parse in `tokio::time::timeout`. A
   timed-out task detaches and keeps its engine permit, bounded by AnyDoc's own
   internal limits. `catch_unwind` does not catch stack-overflow-class faults,
   so true containment needs the child-worker shape. Building
   `tool-kit-anydoc-worker` preemptively is machinery the self-hosted
   single-user threat model does not justify. The documented shape in
   `pdf_inspector.rs` stays the fallback if a field crash ever corrupts or kills
   the converter.
2. **AnyDoc publishes with no completeness warning because it cannot measure
   one.** `QualitySignals::unmeasured()` is honest. AnyDoc's own part-level
   "skip a broken piece and continue" recovery is silent, so a
   partially-degraded document can still succeed. That is upstream behavior, not
   a local gate failure.

### M4: routing and quality policy. Closed

**Scope discipline:** the M4 gate is a fixture regression suite, not a
rate-calibration program.

- [x] CVR-040: labeled corpus of native, scanned, image, mixed, dense-table,
  multi-column, form, encoding-damaged, encrypted, malformed, and over-limit
  PDFs. `tests/support/corpus.rs` generates every case in process, so no binary
  PDF enters the repository.
- [x] CVR-041: `engines::QualitySignals` is engine-neutral and carries only
  measurements: `native_text_ratio` (`None` when unmeasurable), `has_tables`,
  `has_columns`. No filename or page-count heuristic exists. A fourth field,
  `pages_needing_ocr`, was carried and never read, so it is gone. The worker
  still decides OCR from its own raw page list.
- [x] CVR-042: `conversion::policy::decide(profile, route, LocalResult) ->
  PolicyDecision` is pure. No IO, no clock, no engine type. `best_quality` stays
  rejected at the API until M5 gives it a remote leg to mean.
- [ ] CVR-043: calibrate dense and complex routing. Deferred. See section 11.
- [x] CVR-044: expose route, reason codes, warnings, and policy decisions in
  status and provenance without exposing document content. The desktop renders
  them verbatim through `jobDetailItems`, so a new code needs no frontend
  release.
- [x] CVR-045: `tests/routing_policy.rs` drives every corpus case through the
  real HTTP surface and the real worker, preventing scanned, mixed, damaged,
  empty, or incomplete Markdown from silently succeeding.

**Gate met 2026-08-19**, with CVR-043 deliberately open.

`standard` and `local_only` stay behaviorally identical, and M4 pins that
rather than ending it. `policy::decide` takes `_profile` unused on purpose: the
profile has nothing to choose between until M5 gives `standard` a remote leg.
The corpus is the proof. `partly-scanned-9-text-pages` and
`partly-scanned-9-text-pages-local-only` expect the same `succeeded` and the
same `pages_without_extractable_text` warning, and `scanned-1-page` and
`scanned-1-page-local-only` both expect `needs_remote` with no artifact.
`apps/converter/README.md` says the same thing. Do not write a profile split into any
doc before M5 builds one.

#### The confidence correction

The first cut of M4 read pdf-inspector's `confidence` as a completeness measure
and routed remote on it. Three independent skeptics rejected that and the worker
binary confirmed them.

Measured against the real worker, `confidence` on a text-based PDF is exactly
`text_pages / total_pages`. It returns 1.0 for every all-native document at 1,
2, 4, and 10 pages.

**`QualitySignals::native_text_ratio` is the share of pages carrying
extractable text. It is not a completeness measure and the policy must never
route on it.** A ten-page report with a one-line cover page and no images
anywhere reports 0.9 with nothing missing, and its Markdown contains every word.
A single page holding both text and a full-page scan reports 1.0 with the scan
lost. Routing on that signal would have billed Datalab for ordinary documents.

The signal now warns and never routes.
`a_sparse_cover_page_is_not_missing_content` stops anyone turning it back into a
route. The engine's own give-up is the only thing that produces `needs_remote`.

#### Three PDF-fixture traps, all found the hard way

1. **Body text must not begin with the word "Page".** pdf-inspector strips those
   lines, and the sparse branch then flags every page for OCR.
2. **`form_pdf` produces an inspection identical to the same page with no
   AcroForm.** There is no signal to assert on.
3. **`garbled_font_pdf` never reaches the garbled branch**, because
   `hasEncodingIssues` never sets. `garbled_text` is one of three engine
   fallback reasons with no reachable case.

### M5: Datalab fallback. Planned

- [ ] CVR-050: typed Datalab configuration, credential loaded only inside the
  backend.
- [ ] CVR-051: bounded submit, poll, and result HTTP adapter with stable errors,
  timeouts, retry classification, and redacted logs. Match `providers.rs` retry
  and transient/terminal classification.
- [ ] CVR-052: extend the durable state model for remote submission, polling,
  finalization, and uncertain acceptance through a contract-reviewed migration.
- [ ] CVR-053: persist the Datalab request ID before polling and resume a known
  request after restart.
- [ ] CVR-054: represent uncertain billable submission explicitly and never
  blindly resubmit a request whose remote acceptance is unknown.
- [ ] CVR-055: apply the M4 policy to scanned, mixed, incomplete, empty, and
  approved recoverable local failures. Enforce that `local_only` never sends
  bytes externally.
- [ ] CVR-056: validate remote Markdown and publish it through the same
  attempt-scoped artifact boundary.
- [ ] CVR-057: test success, privacy denial, rate limiting, outage, timeout,
  malformed output, interrupted polling, and uncertain submission.

### M6: desktop integration. Implementation landed, gate open

- [x] CVR-060: backend URL and device-token settings outside React state, token
  in the macOS Keychain.
- [x] CVR-061: generate and commit the Tauri HTTP client and schema against the
  backend OpenAPI contract.
- [x] CVR-062: stream selected files from Tauri to the backend with an
  idempotency key and stable client run ID.
- [x] CVR-063: poll durable job state and recover an in-progress run after app
  restart.
- [x] CVR-064: download Markdown through Tauri and preserve collision-safe
  output naming.
- [x] CVR-065: show route, warnings, privacy decisions, and actionable failures
  without exposing credentials to the webview. Run is gated on
  `capabilities.inputFormats`.
- [x] CVR-066: keep the direct-provider path behind a reversible switch until
  the deployment gate passes.
- [ ] CVR-067: end-to-end desktop tests for local success, Datalab fallback,
  restart recovery, backend unavailability, and retry-safe replay. Deterministic
  native and loopback coverage proves local success, restart and replay,
  unknown-status pending behavior, backend unavailability, `needs_remote`
  profile decisions, stop and retry cleanup, response identity, and path-only
  artifact IPC. **The M5-dependent end-to-end durability scenario for
  backend-owned Datalab fallback is pending.**
- [ ] CVR-081: capture direct-path baselines, roughly ten representative files
  covering native PDF, scanned PDF, docx, xlsx, and formats still on Datalab.
  Compare output completeness, routing, and failure behavior against the backend
  path. **Moved from M8:** run before cutover, not after M7.

**Gate not met.** The implementation completes local conversions, recovers
active jobs, keeps credentials and bytes native, and preserves the direct
fallback. CVR-067's fallback-durability scenario and CVR-081's baselines remain
open, and history reuse still ignores provenance.

#### M6 transport decisions, owner-settled 2026-08-18

- **`POST /api/v1/conversions` is a Rust host special-case.** It parses the
  source path from JSON and builds and streams the multipart request. File bytes
  and the bearer token stay out of the webview.
- **Markdown download gets a separate Tauri command** that returns an output
  path. The body never crosses IPC.
- Everything else goes through one `service_request` command. The registered
  host handler owns the base URL, bearer token, multipart file stream, response
  limits, and token redaction. `apps/desktop/src/app/api/transport.ts` plugs that
  Tauri-backed `fetch` into openapi-fetch's documented seam.

### M7 and M8

Ticket lists are in sections 4 and 5.

---

## 7. Architecture

Run document-to-Markdown conversion on the local network behind one stable API
used by the desktop app. Reuse Firecrawl's document engines rather than build
parsers. Keep the deployment small enough to operate on one TrueNAS host.
CPU-only. GPU support, local OCR, and local model serving are excluded.

### Deployment shape

```text
Desktop app
    |
    v
Tailscale sidecar (the only reachable service, M7)
    |
    v
Rust/Axum converter
    |-- HTTP API, auth, limits, health
    |-- SQLite repository and durable job ledger
    |-- one bounded in-process worker loop
    |-- PDF ------------> isolated pdf-inspector Rust worker
    |-- other documents -> AnyDoc Rust integration (in-process)
    `-- hard cases ------> Datalab HTTP adapter, when policy permits (M5)
    |
    v
host-local /data dataset
    |-- converter.sqlite
    `-- jobs/{job-id}/...
```

The production Compose stack contains `tailscale` (the sidecar that terminates
TLS and carries every request) and `converter`, which shares the sidecar's
network namespace and publishes no port of its own. SQLite is embedded in
`converter`, not a separate container. Development publishes the converter on
loopback and omits the sidecar. **AnyDoc is a Rust crate inside the single
converter container, not a separate service.**

### Fixed decisions

- Extend the existing Rust/Axum backend. Do not replace it with FastAPI.
- One converter container and exactly one job worker.
- One worker loop, strictly sequential. It awaits each claimed job before
  claiming the next, pinned by
  `single_runner_keeps_waiting_jobs_queued_and_execution_serial`. Each engine
  also holds its own permit, which matters because a timed-out AnyDoc parse
  detaches still holding one. Two jobs never parse concurrently.
- SQLx 0.9, default features disabled, only `runtime-tokio`, `sqlite`,
  `migrate`, `macros`. Track migration changes through `apps/converter/build.rs`.
- Store sources and artifacts on the filesystem, not as SQLite blobs.
- One host-local `/data` dataset. Never put the live SQLite database on SMB,
  NFS, or another network filesystem.
- Configure the data directory. Keep the filename fixed as `converter.sqlite`.
- Send PDFs directly to `pdf-inspector`. Do not parse them twice to force every
  format through AnyDoc.
- Use AnyDoc only for formats proven by fixtures. Use Datalab only for explicit
  policy-approved fallback.
- Do not add PostgreSQL, Redis, RabbitMQ, a generic queue product, or a second
  API implementation.
- Do not extract a service without an observed runtime, resource, scaling,
  release, failure-isolation, security, or ownership reason.
- Do not create a root Cargo workspace. `src-tauri` and `backend` keep separate
  lockfiles.

### Public API

| Endpoint | Purpose |
|---|---|
| `POST /api/v1/conversions` | Upload one document, return `202 Accepted`, `Location`, and an ID |
| `GET /api/v1/conversions/{id}` | Durable status, route, warnings, errors |
| `GET /api/v1/conversions/{id}/artifacts` | List validated published artifacts |
| `GET /api/v1/conversions/{id}/artifacts/markdown` | Stream completed Markdown |
| `GET /api/v1/conversions/{id}/artifacts/manifest` | Stream the provenance manifest |
| `GET /api/v1/capabilities` | Enabled formats, profiles, capacity, limits |
| `GET /health/live` | Process liveness |
| `GET /health/ready` | Database, data-root, and worker readiness |

`Idempotency-Key` is required for submission. `clientRunId` correlates a desktop
run without replacing server-generated job and attempt IDs. Polling is the
progress mechanism. Server-sent events and WebSockets are not needed for the
expected volume.

### Durable job model

```text
queued -> converting_local -> finalizing -> succeeded
                         |                 `-> failed
                         `-> needs_remote
```

M5 may add remote states such as `converting_remote` and
`remote_submission_uncertain` through an explicit contract update.

SQLite is the source of truth. A Tokio notification wakes the worker promptly,
and periodic SQLite polling guarantees progress after a missed notification or a
restart. The notification is an optimization, not the queue.

Submission and execution:

1. Generate backend-owned job and attempt IDs.
2. Stream the upload under `/data` with byte and time limits, then close,
   validate, hash, sync, and atomically place the immutable source.
3. Insert the conversion, first attempt, and idempotency record in one
   transaction. Remove the newly staged job tree on replay, conflict, capacity
   rejection, or transaction failure. Replay stays valid even when active
   capacity is full.
4. Return `202 Accepted` only after the durable source and the commit exist.
5. Let the single worker transactionally claim the oldest eligible job, then
   revalidate source containment, file type, byte count, and hash before parsing.
6. Record `finalizing`, validate staged output, and atomically rename the
   attempt publication directory.
7. Record `succeeded` only after published artifacts and hashes are verified.

Delivery is at-least-once. Local conversion may repeat after a crash, but
attempts never overwrite each other and only one validated attempt becomes
active. Datalab submission has stricter recovery rules because it is billable.
Persist a known remote request ID and resume polling. Never blindly repeat a
submission whose acceptance is uncertain.

### Persistence layout

```text
/data/
  converter.sqlite
  converter.sqlite-wal
  converter.sqlite-shm
  jobs/
    {job-id}/
      source/
        input
      attempts/
        {attempt-id}/
          publication.staging/
          artifacts/
            result.md
            manifest.json
  quarantine/
    pre-acceptance/
      {job-id}/...
```

One immutable source belongs to the job. Every execution gets its own attempt
directory, so retries preserve provenance and cannot overwrite earlier output.
Staging and publication stay on the same filesystem so rename remains atomic.
All paths are generated by the backend and stored relative to `/data`. Client
filenames never become paths.

The SQLite model holds `conversions` (identity, client run, profile, source
metadata, state, active attempt, route, timestamps, stable failure fields),
`attempts` (execution identity, state, engine and version, classification,
warnings, fallback reason, timing, recovery count), and `artifacts` (attempt,
kind, relative path, media type, byte count, hash).

An authentication scope plus hashed idempotency key is unique on `conversions`.
The stored request fingerprint distinguishes safe replay from a conflict. The
scope is the literal `bootstrap` today. M7 introduces per-device scopes through
an explicit migration. Foreign keys, WAL, `synchronous=FULL`, a busy timeout,
embedded migrations, and bounded active-job capacity are required. Terminal rows
do not consume worker queue capacity.

### Recovery rules

- `queued`: leave eligible for the worker.
- `converting_local`: create a new attempt and requeue within the recovery
  limit, only after revalidating the immutable source.
- `finalizing`: validate any published directory and hashes. Complete success if
  publication finished, otherwise clean staging and retry within policy.
- `succeeded`: if a required artifact is missing, symlinked, or hash-mismatched,
  move the public job to `failed` with `artifact_integrity_failed`, retain the
  audit metadata, and serve no artifacts.
- `failed` and `needs_remote`: terminal until an explicit retry or fallback
  operation exists.
- A pre-acceptance job tree with no database owner: move it under
  `/data/quarantine/pre-acceptance/` without following symlinks. Nothing
  auto-deletes quarantined document data. Reviewed deletion belongs to M7.

Startup reconciliation finishes before readiness turns healthy and before the
worker claims new jobs. Startup contains a per-job failure instead of aborting
the boot. Only a repository error stops the service. An unreadable row is left
for revalidation, and an untrustworthy one is quarantined `failed` /
`recovery_state_unrecoverable`. Recovery rows decode independently through
`RecoveryCandidate`, so one bad column cannot fail the whole listing.

One converter instance only. Multi-host coordination is not part of this
architecture.

### Routing policy

| Input | Route |
|---|---|
| Native-text PDF with acceptable output | `pdf-inspector` |
| Scanned, image-based, mixed, empty, or quality-failed PDF | Datalab, or `needs_remote` under `local_only` |
| Supported non-PDF | AnyDoc |
| Approved recoverable local failure | Datalab when policy permits |
| Encrypted, malformed, unsupported, or over-limit input | Explicit rejection or failure, never an automatic upload |

Profiles: `local_only` means no document bytes leave the local network.
`standard` prefers local conversion, then approved fallback. `best_quality` is
rejected at the API until M5 gives it a remote leg to mean.

**That table is the M5 target, not today's behavior.** Every Datalab cell in it
is unbuilt, so today a scanned, mixed, or quality-failed PDF finishes
`needs_remote` under `standard` exactly as it does under `local_only`. See the
M4 section for the corpus cases that pin the two profiles together.

### Internal module boundaries

```text
api/            HTTP parsing, response mapping, OpenAPI contract
conversion/     use cases, state transitions, routing policy
persistence/    SQLite repository and embedded migrations
jobs/           claim loop, execution, shutdown, recovery
engines/        pdf_inspector, anydoc, and datalab adapters
artifacts/      staging, validation, publication, retention
auth/config/error
```

These seams support tests and later extraction without introducing network calls
between components today. Do not create a universal document AST or a generic
workflow platform.

### Security baseline

- The Tailscale sidecar is the only reachable service. The converter publishes
  no port, shares the sidecar's network namespace, and runs non-root with a
  read-only root filesystem, dropped capabilities, `no-new-privileges`,
  resource limits, and one narrow writable `/data` mount. It does not sit on an
  `internal: true` network, which would also block the outbound HTTPS the M5
  Datalab adapter needs.
- Parser children receive generated paths, a cleared environment, a hard
  timeout, an output ceiling, and no client-controlled command arguments.
- Datalab credentials stay backend-only. `local_only` content is never sent
  externally.
- Logs carry request, job, and attempt IDs, routes, versions, timings, and
  stable error codes. Not source content, not credentials. Exact engine
  versions, output hashes, warnings, and fallback reasons go in the database and
  the manifest.
- Backup and restore treat SQLite plus artifacts as one coordinated dataset. The
  release procedure quiesces writes or uses a verified SQLite backup and
  checkpoint process before the snapshot.

### Explicit non-goals

FastAPI or a parallel document API. PostgreSQL, Redis, or a message broker at
initial scale. GPU or accelerator support. Local OCR or model serving. Multiple
converter replicas or distributed orchestration. A generic job or workflow
platform. A universal document AST. A workflow dashboard, SSE, or WebSockets.
Per-page local and remote merging. Transcription or summarization
implementation in this epic. Publishing the service on the public internet
stays a non-goal: M7 reaches it over a private tailnet, not from an open port.

### Future capability rule

Transcription, summarization, and similar functions start as bounded modules or
Rust provider adapters behind the same origin. A capability becomes a separate
service only with an observed trigger: an incompatible runtime, materially
different CPU or memory needs, independent scaling or release cadence, a failure
or security-isolation requirement, or consumers that need it independently of
conversion. Reconsider PostgreSQL or a broker only if multiple instances or
hosts must coordinate durable work.

---

## 8. Traps

### Container and Compose

- **The Dockerfile must keep copying `migrations/` and `build.rs`**, or
  `sqlx::migrate!` cannot compile in the image.
- **The runtime stage must keep `RUN install -d -o 10001 -g 10001 -m 0700
  /data`**, or a fresh named volume lands as `root:root` while the service runs
  as `10001`.
- **No `VOLUME` instruction.** It hands plain `docker run` an anonymous volume
  nobody prunes.
- **`TOOLKIT_CONVERTER_TOKEN_FILE` means two different things** in
  `compose.yaml`. See Phase 1 setup.
- **`stop_grace_period` is 45s on purpose.** Docker's default 10s wait kills the
  service 20s before `TOOLKIT_CONVERTER_SHUTDOWN_GRACE_SECS` (30) can finish
  draining, so every stop would look like a crash.
- **Run the container smokes with a CPU-capped builder.** The image build is the
  only part that saturates the machine, and the default builder lives inside the
  Docker VM where `docker update` cannot reach it:

  ```bash
  docker buildx create --name toolkit-capped --driver docker-container --bootstrap
  docker update --cpus 4 buildx_buildkit_toolkit-capped0
  TOOLKIT_SMOKE_BUILDER=toolkit-capped apps/converter/scripts/container-smoke.sh
  ```

### Backend correctness

- **A PDF-shaped assumption in an engine-neutral layer crash-looped the service,
  and PDF-only tests hid it.** When you add an engine, grep for the other
  engine's vocabulary in shared layers, and make at least one restart test and
  one container test use the new engine's input.
- **The fault barrier is production code that production never arms.**
  `apps/converter/src/faults.rs` is reachable only from a test holding an `AppState`.
  Do not add a flag, an environment variable, or a route that arms it. Its four
  call sites sit between committed transactions on purpose. A parked task must
  hold no SQLite write lock.
- **A format is advertised only with a passing round-trip fixture.**
  `advertised_media_types_match_the_migration_check` and
  `advertised_media_types_match_the_openapi_upload_contract` pin the admission
  table to the migration CHECK and the upload contract. Drift turns a clean 415
  into an INSERT constraint error at runtime.
- **Migrations 0002 and 0003 wrap their rebuild in one transaction**, pinned by
  `every_no_transaction_migration_wraps_its_rebuild_in_one_transaction`. Editing
  an applied migration is safe only because nothing is deployed. **After the
  first deployment, a broken migration needs a new file.**
- **`execute_claimed` bounds its engine-permit wait and honors shutdown.** A
  detached AnyDoc parse still holds its permit on purpose, so the wait is real.
  Past the bound the runner exits and startup recovery requeues.
- **`TOOLKIT_CONVERTER_SCRATCH_PARENT` is dead config.** `config.rs` parses and
  validates it and nothing reads it. It stays on purpose, because removing an
  environment variable changes the public surface and deserves its own decision.
  Do not wire it to anything on the assumption it was forgotten.
- **`/health/ready` is unauthenticated** and opens a `BEGIN IMMEDIATE`
  transaction against a four-connection pool per request. Loopback-only today.
  Give it a real rate limit when M7 remote access lands. The tailnet is a
  smaller audience than the LAN, not an empty one.

### Desktop billing safety

- **The desktop ledger is schema 4, and a remote fallback is recorded before its
  request is sent.** A provider set with no request ID means the outcome is
  unknown, and recovery refuses to resubmit rather than risk a second charge.
  Before this, every restart mid-fallback billed the file twice.
- **Both writes happen before a billable Datalab request, and both are
  required.** The ledger survives a crash, and the in-memory `FallbackContext`
  survives a same-process Retry. Recording only one left Retry able to pay twice
  with no restart involved.
- **Recovery refuses to resume against a backend URL that is no longer
  configured**, because the Keychain holds exactly one backend token.
- **Every write-back to a job needs a `stale()` check in front of it**, not just
  the expensive steps. A submit can run for 30 minutes, so a finished upload
  that writes "processing" over a row the user already stopped leaves a job no
  task will ever finish, and `running` sticks on. `set_status` is the guarded
  setter for both routes. One direct `JobManager::update` remains, stamping
  `started_at` at `jobs.rs:1268`. It is safe only because an explicit `stale()`
  check sits immediately above it with no `await` in between. Keep that pairing
  intact if you touch it.
- **Direct-path `fail` and `finish` take a generation**, like their backend
  counterparts. Nothing writes back to a stopped row.
- **`retry_job` and `retry_failed` re-persist the durable row Stop deleted.**

### Desktop platform

`CLAUDE.md` stays live and owns these in full. The two that bite backend work:

- **macOS has two keychains and the app targets the one without dialogs.**
  Release builds reach the data protection keychain through the
  `keychain-access-groups` entitlement, which needs the embedded provisioning
  profile to authorise it. A bare `cargo run` binary cannot carry that profile,
  so dev falls back to the legacy keychain with its own separate copy of every
  key. Let the app create the items. Do not seed them with the `security` CLI.
- **Custom Tauri commands need no capability entries, but core commands do.**
  `core:window:default` does not include `hide` or `destroy`. A missing entry
  fails silently at runtime.

The rest are in `CLAUDE.md` under Gotchas: `setSize` clamping order, page zoom
versus the traffic lights, `apps/desktop/src/shell/geometry.ts` as the only home for window
geometry, `react-resizable-panels` layout keying, and the ⌥⌘V shortcut choice.

### Process

- **Never `git add -A` or `git add .`.** Desktop and backend work share one
  dirty tree. Stage explicit paths.
- **Never let an agent generate synthetic load.** One asked to prove a timing
  assertion built a spin-loop farm, then failed to clean it up because `jobs -p`
  returns nothing in a non-interactive `zsh -c`. The machine sat near 960% CPU
  until killed by hand.
- **Scope agents to explicit paths and verify they finished before reverting.**
  A gate agent did an unrequested docs consolidation, then re-applied the same
  change after the first revert because it was still running. Check file mtimes
  rather than trusting a task-status report. Tracked files come back with
  `git checkout HEAD --`. An untracked file is simply gone, so `git add` new
  work early.
- **A read-only "audit and synthesize" fan-out is the wrong shape for this
  repo.** One produced a long ranked report and fixed nothing, while every real
  blocker came from reading the code directly. If you fan work out, have the
  agents land patches behind a gate.

### Dependencies

Pins: Rust **1.97.1** (`rust-toolchain.toml`), Node **24** (`.node-version`),
pnpm **11.22.0** (`package.json` `packageManager`).

Deliberately not latest, with the reason each pin exists:

| Pin | Reason |
|---|---|
| TypeScript **6.0.3** | 7.0 has no compiler API and `typescript-eslint` crashes. `tsconfig.json` must not set `baseUrl` |
| `security-framework` with the **`OSX_10_15`** feature | Not a default feature, and it gates `use_protected_keychain()`. Without it `secrets.rs` silently targets the legacy keychain |
| `pdf-inspector` **`=1.15.0`** | The Dockerfile copies that exact crate path |
| `anydoc` **`=0.1.9`** | The verified and fixture-proven revision |
| `libc` under 1.0 | 1.0 is still alpha |
| `openapi-typescript` on TypeScript 5 peer | Only the `pnpm generate:api` CLI. Keep it paired with `openapi-fetch` 0.17 |

**pnpm 11** gates package build scripts. esbuild is approved through
`allowBuilds: { esbuild: true }` in `pnpm-workspace.yaml`. Bumping a Cargo major
means editing `Cargo.toml` ranges, not only `cargo update`.

---

## 9. Verify

```bash
pnpm check            # tsc --noEmit + eslint + vitest         (fast inner loop)
pnpm verify           # check + build + src-tauri clippy/tests (desktop scope)
pnpm verify:backend   # lint:api + converter clippy/tests
pnpm verify:all       # both
pnpm verify:local-corpus # routing_policy plus the AnyDoc sweep, no container
pnpm verify:container # built image + graceful and SIGKILL restart smokes

pnpm lint:api         # Spectral over the contract          (part of verify:backend)
pnpm verify:api-drift # schema.ts still matches the contract
pnpm verify:deps      # cargo-deny over both crates
pnpm verify:contract  # Schemathesis against a live service

apps/desktop/src-tauri/scripts/verify-release.sh  # the built bundle, after pnpm tauri build
```

`pnpm verify` is deliberately desktop-scoped, so in-flight backend work cannot
fail a desktop change.

`verify:container` is deliberately **not** part of `verify:all`. The others are
offline cargo, tsc, and eslint runs. The container gate needs a Docker daemon,
builds an image, and takes minutes, so folding it in would break the everyday
gate on any machine without Docker running.

CI (`.github/workflows/ci.yml`) runs frontend, backend, desktop, and
dependencies as four jobs on push and pull request. `dependencies` is separate
because the RustSec database moves without us, so it can go red on a commit
that changed nothing, and that must not read as a broken build. Desktop clippy and tests use `macos-latest`
because of `macos-private-api` and `security-framework`. **CI does not run the container
smoke.** It is a local release gate, so re-run it by hand before closing any
milestone that touches persistence, the worker, the Dockerfile, or Compose.
Evidence lands in `apps/converter/target/container-smoke/<utc-timestamp>/evidence.md`,
which is gitignored. Paste the relevant lines into this file rather than linking
the path.

### Suites written outside this repo

Three gates assert things nobody here wrote the assertions for. Every other
suite in the tree was written by whoever wrote the code under it, which is the
blind spot section 9 already recorded once: the billing test asserted literals
it had just constructed and could not fail.

| Gate | Suite | What it grades |
|---|---|---|
| `pnpm lint:api` | Spectral `spectral:oas`, ~60 rules | the contract as a document |
| `pnpm verify:deps` | RustSec advisory database, via cargo-deny | every crate in both lockfiles |
| `pnpm verify:contract` | Schemathesis, cases derived from `openapi.yaml` | the running service against its contract |

`verify:contract` builds the converter, starts it on a throwaway port and data
root, seeds one real conversion from `tests/fixtures/anydoc/text.docx`, and
then runs the whole derived suite twice: once with the seeded id, once with a
UUID no conversion has. Roughly 400 cases per pass. It needs `uv` and no
Docker. Config lives in `contract/http/schemathesis.toml` and every check
turned off there names the reason.

**Its first run found three admission rules the contract never documented**:
the `source` part must carry a filename, its extension must agree with the
part's `Content-Type`, and the bytes must open with that format's container
signature. A client generated from the contract alone could not submit. All
three are now in `openapi.yaml` and reach `schema.ts` as documentation.

Both `deny.toml` files pin the platforms they grade. `src-tauri` names macOS
only, which drops Tauri's GTK3 stack and the ten unmaintained advisories that
come with it, rather than ignoring them by id on a platform we do ship. Every
remaining exception names its crate and its reason, so a new unmaintained
dependency still arrives as a failure.

### Release 1.0

Cut from the Phase 1 feature set. M5, M7, and M8 stay unbuilt and are 1.1.

An audit of six lenses (desktop Rust, backend Rust, frontend, call
aggregation, bloat, release readiness) produced 53 findings; adversarial
verification refuted 30. **No memory leak, infinite loop, or runaway CPU path
survived.** The generation-counter design held: exactly one write-back in
`jobs.rs` was missing its `update_if_generation` guard, and it is now guarded.

Landed for 1.0:

| Change | Where | Why |
|---|---|---|
| Upload buffer is `Bytes`, not `Vec<u8>` | `providers.rs` | `send_retrying` takes `Fn`, so a per-attempt `Vec` clone held two copies of the file. Transcribe accepts video with no size cap, times four permits |
| The last unguarded write-back is guarded | `jobs.rs` | Plain `update` drops the lock before the emit, so a Stop in that window shipped a stale `queued` row and `running` stuck on |
| `output_text` removed from `Job` | `jobs.rs`, `App.tsx` | Every result crossed IPC and stayed in the webview for the session. Copy reads the file through `read_document_text`, which has its own 50 MiB ceiling because a result too large to edit is still worth copying |
| `MarkdownViewer` memoized | `MarkdownViewer.tsx` | react-markdown memoizes nothing, so an open document re-parsed on every one of a run's ~1000 events |
| One `reqwest::Client` per process | `jobs.rs`, `conversion_service.rs` | A client owns the connection pool. One per job meant 200 TLS handshakes on a 200-file run |
| Poll emits only on change | `jobs.rs` | The backend poll called `set_status` with the same pending note every 5s per job |
| Staged job tree survives cancellation | `api/conversions.rs` | A dropped request future reaches no `.await`, so a client that quit mid-upload left up to 25 MiB nothing deleted. Mirrors `impl Drop for ProbeFile` |
| CSP set, `allow-destroy` dropped | `tauri.conf.json`, `capabilities/default.json` | The app renders Markdown converted from confidential documents, so a remote image reference is an outbound request. `style-src 'unsafe-inline'` is load-bearing: CodeMirror injects `<style>` at runtime |
| OFL text ships in the bundle | `apps/desktop/src/ui/fonts/OFL.txt` | Three OFL woff2 files land in `Contents/Resources` and clause 2 requires the licence to accompany them |
| Copyright, category, min system version | `tauri.conf.json` | `LSMinimumSystemVersion` was defaulting to 10.13. It is now **26.0**, so the DMG will not launch on macOS 15 or earlier |
| Four dead artifacts deleted | `FileViewer.tsx`, `PanelTone "bare"`, `icons/tray.png`, `icons/icon.png` | Nothing imported or loaded any of them |
| Gallery chunk kept out of the build | `main.tsx` | `import.meta.env.DEV &&` lets Rollup drop the dynamic import. The packaged webview can never reach `?gallery` |

Deliberately **not** done: the artifact double-hash in `service.rs` threads new
return values through the code that guards artifact serving to save
microseconds on 1.4 KB outputs. Not worth the risk.

Not covered by the audit, and worth knowing before 1.0 goes to anyone else:
first-run states (empty Keychain, no network, revoked key, disk full mid-write),
the Rev.ai path specifically, npm dependency licences, a `history.db` migration
across a real version step, and accessibility. Nothing has run the shipped
`.app` end to end from a quarantined DMG.

### Live API smoke tests

`apps/desktop/src-tauri/src/live_smoke.rs` hits the real Datalab and Rev.ai endpoints. They
are `#[ignore]`d because they spend API credits, and they read keys from the
environment. Note that `REV_AI_API_KEY` must be exported as `REVAI_API_KEY`.

```bash
DATALAB_API_KEY=… REVAI_API_KEY=… \
  cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib live_smoke -- --ignored --nocapture
```

### Verification log

| Date | Event |
|---|---|
| 2026-08-17 | M1 gate. 28 Rust tests pass |
| 2026-08-18 | M3 Increment 0 AnyDoc spike returns go |
| 2026-08-18 | M3 gate. Smoke `20260818T192759Z`, image `sha256:cd989ed8…` |
| 2026-08-19 | Sprint 0 repairs. Contract 0.4.1 |
| 2026-08-19 | Smoke `20260819T022230Z`, image `sha256:8bb30ae5…` |
| 2026-08-19 | Sprint 1 / M4 gate. Contract 0.4.2 |
| 2026-08-19 | Smoke `20260819T031914Z`, image `sha256:4a0cbdd0…`. Evidence names commit `d6fc6eb`, taken over a dirty tree |
| 2026-08-19 | Phase 1 closeout. All three verify gates green at clean tree `a337594` |
| 2026-08-19 | Smoke `20260819T161851Z`, image `sha256:4a0cbdd0…`, 116 PASS, 0 FAIL. Same image id, so the earlier run did cover M4 |
| 2026-08-19 | Acceptance D.1 through D.6 all pass. **Phase 1 gates met.** M5 is unblocked |
| 2026-08-19 | Closeout merged to `main` and published. `verify:all` green on the merge: 166 backend, 64 desktop, 79 frontend |

Per-increment evidence through M6 lives in
[`archive/BACKEND_VERIFICATION_LOG.md`](archive/BACKEND_VERIFICATION_LOG.md).

**Sprint 0 summary.** An adversarial review of M1 through M6 confirmed ten
defects. All are fixed. A second review then showed two of those repairs had
closed the reported instance and left the rest of the class open, and both are
now closed as well. The Datalab ledger write is required and the in-memory
fallback context is written before the request, so a same-process Retry can no
longer resubmit. Recovery rows decode independently, so a row that will not
parse is quarantined instead of aborting the boot. Sprint 0 closed 14 defects
plus the two class follow-ups. The full defect table is in git history at
`a7b4976` (desktop) and `5fe6463` (backend).

**When you add a state or a vocabulary, the question is no longer "did I update
every layer" but "what does this cost if I did not".**

**Sprint 1 gate.** 160 backend tests, 63 desktop Rust tests, 80 frontend
tests, both Clippy gates, the production build, and container smoke
`20260819T031914Z` against image `sha256:4a0cbdd0…`. Graceful and SIGKILL,
migrations rebuilding fresh in the image, AnyDoc converting a docx
in-container, and artifacts byte-identical across a restart. The 160 does not
reproduce and what it counted is not recorded, so use the closeout breakdown
below instead.

**Phase 1 closeout gate, 2026-08-19, clean tree `a337594`.** Every number below
came from one `pnpm verify:all` run, so it is reproducible rather than recalled.

| Suite | Passed |
|---|---|
| `backend` lib | 79 |
| `backend` main | 1 |
| `tool-kit-pdf-worker` | 3 |
| `crash_recovery` | 9 |
| `failure_modes` | 7 |
| `http_contract` | 46 |
| `integrity_matrix` | 12 |
| `routing_policy` | 9 |
| **backend total** | **166** |
| desktop Rust lib | 64, plus 3 `#[ignore]`d live-API tests |
| frontend Vitest | 79 across 12 files |
| Spectral | 0 findings at `--fail-severity=warn` |
| cargo-deny | 0 vulnerabilities in either crate, 6 unmaintained tolerated with reasons |
| Schemathesis | ~800 cases over two passes, all 8 operations |

`verify:local-corpus` and `verify:container` are green at the same tree. The
container smoke re-ran because the previous evidence named a commit that
predates M4, which made the old gate unprovable from its own record even though
the identical image id later showed it had been fine.

**Closeout repairs, 2026-08-19.** An audit of every falsifiable claim in this
file confirmed 20 mismatches and refuted 4. The doc corrections are folded in
above. Seven were code, and all seven are fixed:

| Fix | Where |
|---|---|
| Stop, Retry, then crash resubmitted an already-billed Datalab request. The rebuilt ledger row dropped every remote column | `jobs.rs`, `ledger_replay` |
| A failed `begin_fallback` left the in-memory claim set, so every later Retry was told a request that never existed may have been charged | `jobs.rs` |
| The billing test asserted literals it had just constructed and could not fail | `jobs.rs` |
| A transient database error at boot permanently quarantined a healthy job | `sqlite.rs`, `RepositoryError::is_row_shape` |
| `NeedsRemote` carried a vector that was always one code, so an empty `fallback_reason` could reach the durable row | `policy.rs`, `service.rs` |
| `QualitySignals::pages_needing_ocr` was computed and never read, behind an unreachable failure branch | `outcome.rs`, `pdf_inspector.rs` |
| `Path` took a `max` prop no product caller passed | `Path.tsx`, `pathCrumbs.ts` |

Two gate scripts also claimed success they never checked. `verify-local-corpus.sh`
printed five case names on trust and printed PASS for an AnyDoc sweep that a
rename would reduce to zero tests, and `print-corpus-pdf.sh` printed "Wrote" for
a file cargo never produced. A name filter that matches nothing is `ok. 0
passed` and exit 0, which `pipefail` cannot see, so each now asserts its own
result. `container-smoke.sh` records whether the tree was dirty, because the
build context is the tree and the `commit:` field alone had already made one
gate unprovable.

---

## 10. Ownership

| Session | May edit | Must not edit |
|---|---|---|
| Backend | `apps/converter/**`, `docs/*.md` | `apps/desktop/**` |
| Desktop | `apps/desktop/**` | `apps/converter/**` persistence, worker, OpenAPI |
| Shared docs | `README.md`, `CLAUDE.md`, `AGENTS.md`: append or reconcile | Reverting the other session's section |

Shared repo files (`.github/`, `LICENSE`, `rust-toolchain.toml`,
`.node-version`, `.gitignore`, `package.json`) belong to neither session. Do not
revert them as part of a backend patch.

---

## 11. Deferred

Open on purpose. None blocks its milestone.

### CVR-043: dense and complex routing calibration

**Deferred to M8 or until real volume exists.** The M4 gate passes on fixture
pass and fail only. Do not block M5 on percentage targets.

Calibration needs a signal that distinguishes "this page produced no text
because it is blank" from "because it is a scan". `confidence` does not. A
ten-page report with a one-line cover page and no images anywhere reports 0.9,
exactly like a document with a scanned page. Until the worker reports pages that
reference an image XObject and yielded no text, calibration would be tuning a
coin flip.

### CVR-080: freeze the labeled corpus

**Deferred to Phase 4.** Run once when the M4 fixture set stops changing, not
twice alongside CVR-043 calibration. Trigger: no corpus manifest edits for one
release cycle, or explicit owner sign-off to freeze.

### CVR-039: parse-path optimization

**Optional and unscheduled. Measure before building it.** Skip the redundant
full source re-read and SHA-256 recomputation at parse time when the immutable
source path, byte length, and hash were validated at claim and submit and the
file has not changed. Measure on typical docx and xlsx before landing.

Performance ladder, to follow in order rather than ticket separately:

1. In-process AnyDoc, concurrency 1 per engine. Current.
2. AnyDoc hard timeout. Done in CVR-037.
3. Skip the redundant re-read and hash. CVR-039.
4. Raise AnyDoc concurrency after memory profiling. Post-M3 gate only.
5. Child-worker fallback, only if containment fails in the field.

### CVR-038: AnyDoc containment policy

**Closed as a recorded decision, and the work it defers stays deferred.**
In-process for now, hard outer timeout required and delivered, child-worker
fallback trigger documented on CVR-032. **Do not build `tool-kit-anydoc-worker`
preemptively.** Revisit only if a field crash corrupts or kills the converter.

### CVR-076, CVR-077, CVR-078: M7 operations depth

Deferred without blocking LAN cutover.

- **CVR-076** (full SBOM and transitive license review): run `cargo auditable`
  once and document licenses instead.
- **CVR-077** (operator runbooks): one README ops section until a second host
  exists.
- **CVR-078** (host-level restart, overload, timeout, disk-full,
  Datalab-outage, TLS, auth, and restore tests): the container smoke subset plus
  a manual host checklist.

### Deferred from M2 Increment 6, on the record

Output-write failure injection has no portable way to induce it. The M1 PDF
fixture corpus was folded into CVR-040 because no `.pdf` exists in the
repository. A multi-process restart test in Rust is covered by the container
smoke against the real binary.

---

## 12. Doc map

| File | Role |
|---|---|
| `docs/STATUS.md` | **This file. The single live status document** |
| `docs/north-star.md` | The local-first product target. Supersedes the M7 remote-deployment track |
| `docs/archive/` | Everything closed, with a note on why |
| `README.md` | How to run, test, and release |
| `CLAUDE.md` | Desktop architecture invariants |
| `AGENTS.md` | Learned user preferences and workspace facts |
| `apps/desktop/src/ui/UI.md` | Design language and the rules for extending it |
| `apps/converter/README.md` | Converter setup and environment variables |
| `apps/converter/evals/README.md` | Corpus manifest data rules |
| `.impeccable.md` | Design context read by every `/impeccable` skill |

The former live documents live in `docs/archive/` with a redirect at the top of
each file: `HANDOFF.md`, `CLOSEOUT_EXECUTION_PLAN.md`, `BACKEND_EPIC.md`, and
`BACKEND_SERVICE_PLAN.md`.

### GitNexus

Indexed as **`tool-kit`** on **1.6.9**. The graph lives in `.gitnexus/`, which
is gitignored. MCP tools must pass `repo: "tool-kit"`.

```bash
gitnexus analyze . --index-only   # incremental, roughly 15s on this repo
gitnexus status                   # indexed commit vs HEAD
```

`--index-only` is load-bearing. A plain `analyze` rewrites the
`<!-- gitnexus:start -->` block in `AGENTS.md` and `CLAUDE.md` with current node
and edge counts, leaving two modified tracked files unrelated to the work in
hand.

Two traps. Upgrade the copy in Homebrew's prefix with
`/opt/homebrew/bin/npm install -g gitnexus@latest`, because a plain `npm i -g`
uses fnm's npm and installs a second copy that shadows it on PATH. And an index
written by a newer GitNexus is unreadable by an older one. That symptom reads
like a transient lock but is a version mismatch, so upgrade the binary and then
restart the MCP server.

Community detection tracks the folder layout, so a file spanning several
communities holds more than one concern. That is how `App.tsx` was split. The
graph is unreliable for dead-code hunting here, because object-literal methods,
destructured hook returns, and default exports behind a dynamic `import()` all
read as uncalled. Verify any "unused" claim with grep before acting on it.
