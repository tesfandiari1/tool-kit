# Tool-Kit status

Where the product stands, what is broken, and what someone still intends to
build. CLAUDE.md owns the architecture contract and is not repeated here.

**Last updated:** 2026-09-12

Everything closed lives in [`archive/`](archive/README.md). The full document as
it stood before this prune is
[`archive/STATUS_2026-09-12.md`](archive/STATUS_2026-09-12.md).

---

## Current state

| Item | Value |
|---|---|
| Branch tip | `64636db` on `main`, with the known-issues fix on top of it |
| OpenAPI contract | 0.4.4 (`contract/http/openapi.yaml`) |
| Desktop version | 1.0.0 (`package.json`, `Cargo.toml`, `tauri.conf.json`) |
| Installed bundle | `/Applications/Tool-Kit.app`, 1.0.0, built from the tree after `64636db` with the 27 known-issue fixes, on 2026-09-12. Signed only, not notarized, app `4f823ae8`. The last notarized build (`8a159b90`, DMG `c38dde49`) is in the Trash |
| Backend | M0 to M4 complete. M5 unbuilt. M7 cancelled |
| Desktop | M6 landed, gate open on CVR-067 and CVR-081 |
| Where the service runs | Sidecar inside the `.app`. Docker needs a `backend-override.json` that no part of the app writes |
| Exposure | Loopback only, and staying there |
| Latest container smoke | `20260819T161851Z`, image `sha256:4a0cbdd0…`. It predates the worker-protocol refactor in `d4f83eb`, so re-run it before closing anything that touches the converter |

The keychain entitlement stays parked, so `verify-release.sh` fails its
**Keychain entitlement** section by design. CLAUDE.md says what restores it.

### Recent changes

| Date | What landed | Consequence worth keeping |
|---|---|---|
| 2026-08-19 | Five commits moved every app into `apps/` and lifted the shared and platform-specific pieces out of the converter | The call graph did not move. `packages/ui` stayed put, because there is no second client |
| 2026-08-20 | The converter, the PDF worker and the Swift Vision worker ship inside the `.app` as sidecars | The sidecar port is ephemeral, so an in-flight ledger row records the alias `sidecar` and takes the live origin at use. Recording a URL breaks Retry across every relaunch |
| 2026-08-20 | The app opens on a library bound to one folder, created or adopted by `workspace.rs` | Identity lives in `.toolkit/workspace.json`, not in the path, so renaming the folder in Finder changes nothing |
| 2026-08-21 | The library became a disclosure tree over disk, and the window became two panes | Settings left the `View` union and became a sheet, so opening it no longer evicts the Run column mid-run |
| 2026-09-12 | The window opens hidden and the frontend sizes, places and shows it. Four agents closed 45 findings. Committed at `2bc1c1b` and `1aee5d4` | Run progress and the title-bar counter measure a per-run baseline, not the whole queue |
| 2026-09-12 | The design system was replaced by the Tristin Esfandiari system: new token vocabulary, DaVinci and Red Hat Display and DM Sans, no radius, shadow or blur | Surfaces are opaque and follow the macOS appearance, `macOSPrivateApi` is gone, and the Mac App Store is now blocked only by the sidecars |
| 2026-09-12 | The 27 known issues closed: three fix agents on disjoint files, two adversarial verifiers that landed five corrections, every gate green | `convert_one` answers a `path` for a copy, `import_into_project` answers `{ landed, failed }`, the gate's `setup_workspace` reports a welcome file a quit at the last beat left unread, and `scanKey` carries `capabilities.state` so the host re-plans when the service comes back |

### What the product does today

- Convert defaults to the backend route, which is the bundled sidecar.
  Transcribe uses Rev.ai, unchanged.
- The sidecar advertises 24 media types against the Docker image's 18. Images
  convert locally through the Vision worker, so only `html` and `htm` are
  permanently direct.
- `local_only` refuses anything that would need Datalab. `standard` falls back
  and spends credits.
- Keys stay in the macOS Keychain. The webview sees neither bearer values nor
  document bytes.
- Backend jobs persist a client run id, idempotency key, origin, source mtime
  and profile before submission. Restart recovery replays against the recorded
  origin.
- Run is planned per file from live `capabilities.inputFormats`, so the desktop
  hardcodes no format set.
- History reuse is asked per file rather than per route. Backend results file
  under `backend:markdown`, which no lookup asks for, so a backend-converted
  file always re-runs. See Next.
- No gate rebuilds `Tool-Kit.app`. Use `pnpm tauri dev` to exercise current
  source.
- The frontend does not render in a plain browser. `useWindowFocusClass` and
  `useCloseConfirm` call `getCurrentWindow()` on mount. Use `pnpm tauri dev`, or
  `?gallery`.

---

## Known issues

Five, all low, found by the verify pass over the 2026-09-12 fix. Each line names the root and the decisive file.

- While the conversion service is down, the scan counts every non-text, non-media extension as convertible, so the blocked hint can over-count a drop with junk in it. It self-corrects on the recovery re-scan and no run can start. Fix: narrow `fallback_conversion_files` once a static backend format set exists (`src-tauri/src/lib.rs`, `fallback_conversion_files`).
- A folder whose refresh read fails without being gone renders as an open branch with no rows after the one toast. Fix: cache a failure marker in `read` and render a quiet row from it (`src/shell/useProjectTree.ts`, `src/domains/library/ProjectTree.tsx`, `childrenOf`).
- `open` in `useDocuments` depends on `docs`, so every door changes identity per keystroke and App keeps `revealJobRef` for it. Fix: read the already-open check through `docsRef` and drop `docs` from the deps (`src/shell/useDocuments.ts`).
- `rustfmt --check` is dirty at seven pre-existing sites in `history.rs`, `tree.rs` and `workspace.rs`. Fix: run `cargo fmt` in its own commit (`src-tauri/src/history.rs:576`).
- `fileGlyph` maps `heic` and `m4v`, which no job accepts. Fix: drop the two entries (`src/domains/library/fileGlyph.ts`).

---

## Next

Three things someone still intends to build.

- **M5, Datalab fallback inside the converter.** Planned in August with no code
  since. Port the retry and transient-versus-terminal rules from
  `apps/desktop/src-tauri/src/providers.rs` verbatim. Do not redesign Datalab
  semantics in the backend.
- **Provenance-aware history reuse.** `jobs::output_format_for` produces a plain
  Datalab format string, so a result filed under `backend:markdown` never
  matches and every backend-converted file re-runs. That costs time and never
  costs correctness.
- **The M6 gate.** CVR-067's fallback-durability scenario waits on M5. CVR-081
  wants direct-path baselines over roughly ten representative files, compared
  against the backend path before any cutover.

### Deferred

Open on purpose. Nobody is scheduled on them.

- **M8, evaluation and cutover.** Its main ticket landed by other means:
  `conversion_route` already defaults to `Backend`. What is left is a rollback
  exercise and release evidence.
- **CVR-043, dense and complex routing calibration.** It needs a signal that
  separates a blank page from a scan. `confidence` is not one.
- **CVR-080, freeze the labeled corpus.** Run it once when the M4 fixture set
  stops changing.
- **CVR-039, skip the redundant source re-read and hash at parse.** Measure on a
  typical docx and xlsx before building it.

### Cancelled

**M7, remote deployment over a tailnet.** Cancelled 2026-08-19, unstarted.
Adopting [`north-star.md`](north-star.md) made the product one local workspace
on one Mac, and the app now runs engines the container cannot host at all. The
plan is in [`archive/M7_EXECUTION.md`](archive/M7_EXECUTION.md).

---

## Traps

Converter and Docker only. CLAUDE.md owns the desktop traps in full.

- **The Dockerfile must keep copying `migrations/` and `build.rs`.**
  `sqlx::migrate!` reads the SQL at compile time, so both are build inputs.
- **Keep `RUN install -d -o 10001 -g 10001 -m 0700 /data`.** Without it a fresh
  named volume lands as `root:root` while the service runs as `10001`.
- **No `VOLUME` instruction.** It hands a plain `docker run` an anonymous volume
  nobody prunes.
- **`TOOLKIT_CONVERTER_TOKEN_FILE` means two different things in
  `compose.yaml`.** Under `environment:` it is the container path. Under
  `secrets:` it is a host-side substitution. Leave it unset on the host.
- **`stop_grace_period` is 45s on purpose.** Docker's default 10s kills the
  service 20s before its own 30s drain can finish, so every stop reads as a
  crash.
- **Run the container smoke through a CPU-capped builder.**
  `TOOLKIT_SMOKE_BUILDER` names one. The default builder lives inside the Docker
  VM, where `docker update` cannot reach it.
- **A PDF-shaped assumption in an engine-neutral layer crash-looped the
  service.** When you add an engine, grep the shared layers for the other
  engine's vocabulary.
- **`apps/converter/src/faults.rs` is production code that production never
  arms.** Only a test holding an `AppState` reaches it. Do not add a flag, a
  variable, or a route that arms it.
- **A format is advertised only with a passing round-trip fixture.** Two tests
  pin the admission table to the migration CHECK and the upload contract. Drift
  turns a clean 415 into an INSERT constraint error at runtime.
- **A no-transaction migration must wrap its rebuild in one transaction.**
  `every_no_transaction_migration_wraps_its_rebuild_in_one_transaction` pins all
  four. After the first deployment, a broken migration needs a new file.
- **`execute_claimed` bounds its engine-permit wait and honors shutdown.** A
  detached AnyDoc parse still holds its permit, so the wait is real.
- **`TOOLKIT_CONVERTER_SCRATCH_PARENT` is dead config.** `config.rs` parses and
  validates it, and nothing reads it. Do not wire it to anything.
- **`/health/ready` is unauthenticated** and opens a `BEGIN IMMEDIATE`
  transaction per request against a four-connection pool. Loopback only today.

---

## Verify

```bash
pnpm check               # tsc --noEmit, eslint, vitest        (fast inner loop)
pnpm verify              # check, build, desktop clippy and Rust tests
pnpm verify:backend      # lint:api, converter clippy and tests
pnpm verify:all          # both of the above
pnpm verify:local-corpus # routing_policy plus the AnyDoc sweep, no container
pnpm verify:container    # built image, graceful and SIGKILL restart smokes
pnpm lint:api            # Spectral over the contract   (also in verify:backend)
pnpm verify:api-drift    # schema.ts still matches the contract
pnpm verify:deps         # cargo-deny over all three crates
pnpm verify:contract     # Schemathesis against a live converter, needs uv
pnpm verify:release      # the built bundle, after pnpm tauri build
```

`pnpm verify` is deliberately desktop-scoped, so in-flight backend work cannot
fail a desktop change. `verify:container` stays out of `verify:all` because it
needs a Docker daemon and takes minutes, and the everyday gate must still run on
a machine with Docker stopped.

CI runs frontend, backend, desktop and dependencies as four jobs.
`dependencies` is separate because the RustSec database moves without us, so it
can go red on a commit that changed nothing. CI does not run the container
smoke. That one is a local release gate.

---

## Ownership

| Session | May edit | Must not edit |
|---|---|---|
| Backend | `apps/converter/**`, `docs/*.md` | `apps/desktop/**` |
| Desktop | `apps/desktop/**` | `apps/converter/**` persistence, worker, OpenAPI |
| Shared docs | `README.md`, `CLAUDE.md`, `AGENTS.md`: append or reconcile | Reverting the other session's section |

Shared repo files (`.github/`, `LICENSE`, `rust-toolchain.toml`,
`.node-version`, `.gitignore`, `package.json`) belong to neither session. Do not
revert them as part of a patch.

---

## Doc map

| File | Role |
|---|---|
| `docs/STATUS.md` | This file. The single live status document |
| `docs/north-star.md` | The local-first product target |
| `docs/DESIGN_PORT.md` | Why the design system looks the way it does. Sources in `design/` |
| `docs/archive/` | Everything closed, with a note on why |
| `README.md` | How to run, test, and release |
| `CLAUDE.md` | Desktop architecture contract and the gotcha list |
| `AGENTS.md` | Learned user preferences and workspace facts |
| `apps/desktop/src/ui/UI.md` | Design language and the rules for extending it |
| `apps/converter/README.md` | Converter setup and environment variables |
| `apps/converter/evals/README.md` | Corpus manifest data rules |
| `.impeccable.md` | Design context read by every `/impeccable` skill |
