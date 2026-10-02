# Tool-Kit status

Where the product stands, what is broken, and what someone still intends to
build. CLAUDE.md owns the architecture contract and is not repeated here.
Closed work is in [`archive/`](archive/README.md).

**Last updated:** 2026-09-12

## Current state

| Item | Value |
| --- | --- |
| Branch tip | `105cdf3` on `main`, plus the uncommitted audio epic sprint 1 |
| OpenAPI contract | 0.5.0 (`contract/http/openapi.yaml`). The converter crate moves with it |
| Desktop version | 1.0.0 (`package.json`, `Cargo.toml`, `tauri.conf.json`) |
| Installed bundle | `/Applications/Tool-Kit.app`, built 2026-09-12 from the uncommitted sprint 1 tree, four sidecars. Signed, not notarized, cdhash `8448ad9e`, after the review pass. The two earlier builds of the day (`4f823ae8`, `0f297a6f`) are in the Trash. No gate rebuilds it |
| Backend | M0 to M4 complete. M5 unbuilt. M7 cancelled |
| Desktop | M6 landed. Gate open on CVR-067 and CVR-081 |
| Service | Sidecar inside the `.app`, loopback only. Docker needs a `backend-override.json` that only `pnpm backend:docker` writes |
| Container smoke | `20260819T161851Z`, image `sha256:4a0cbdd0…`. It predates the worker-protocol refactor in `d4f83eb`, so re-run it before closing converter work |

The keychain entitlement is parked, so `verify-release.sh` fails its Keychain
section by design. CLAUDE.md says what restores it.

Behaviour that is a decision, not an accident:

- Convert and Transcribe default to the sidecar route. Direct is Datalab for
  Convert and Rev.ai for Transcribe.
- `local_only` refuses anything that needs Datalab. `standard` falls back and
  spends credits.
- A backend job persists its run id, idempotency key, origin, source mtime and
  profile before submission. Restart recovery replays against the recorded
  origin.

## Known issues

Five, all low, from the verify pass over the 2026-09-12 fix.

- While the service is down, the scan counts every non-text, non-media
  extension, so the blocked hint can over-count a drop with junk in it. It
  self-corrects on the recovery re-scan. Fix: narrow `fallback_conversion_files`
  (`src-tauri/src/lib.rs`).
- A folder whose refresh read fails without being gone shows an open branch
  with no rows after one toast. Fix: cache a failure marker in `read` and render
  a quiet row (`src/shell/useProjectTree.ts`,
  `src/domains/library/ProjectTree.tsx`).
- `open` in `useDocuments` depends on `docs`, so every door changes identity per
  keystroke and App wraps `revealJob` in `useEffectEvent` for it. Fix: check
  the open list through `docsRef` (`src/shell/useDocuments.ts`).
- `rustfmt --check` is dirty at seven pre-existing sites in `history.rs`,
  `tree.rs` and `workspace.rs`. Fix: `cargo fmt` in its own commit.
- `fileGlyph` maps `heic` and `m4v`, which no job accepts. Fix: drop both
  (`src/domains/library/fileGlyph.ts`).

## Next

- **Audio epic, sprint 1 landed 2026-09-12, uncommitted.** One Swift worker on
  the Neural Engine as the converter's fourth engine: SpeechAnalyzer words,
  FluidAudio speakers, Markdown turns. Transcribe follows `conversion_route`.
  A find, refute, fix review pass landed 24 fixes the same day, the run door
  that demanded a Rev.ai key for a local transcription among them.
  `pnpm verify` and `pnpm verify:backend` green, `verify-release.sh` green
  through the four signed sidecars and the bundled resources (its Keychain
  entitlement section still fails by design, see CLAUDE.md), and the installed
  bundle transcribed a two-speaker fixture through its own sidecars.
  Sprint 2 (Parakeet tier through a Settings model list) and the next epic
  (front matter and structure for AI readers) are in
  [`AUDIO_EPIC.md`](AUDIO_EPIC.md).
- **M5, Datalab fallback inside the converter.** No code since August. Port the
  retry and transient-versus-terminal rules from
  `apps/desktop/src-tauri/src/providers.rs` verbatim.
- **Provenance-aware history reuse.** `jobs::output_format_for` yields a plain
  Datalab format, so a result filed under `backend:markdown` never matches and
  every backend-converted file re-runs. It costs time, never correctness.
- **The M6 gate.** CVR-067 waits on M5. CVR-081 wants direct-path baselines over
  about ten files against the backend path before any cutover.

Deferred, nobody scheduled:

- **M8, evaluation and cutover.** `conversion_route` already defaults to
  `Backend`. A rollback exercise and release evidence remain.
- **CVR-043, routing calibration.** Needs a signal that separates a blank page
  from a scan. `confidence` is not one.
- **CVR-080, freeze the labeled corpus.** Once the M4 fixtures stop changing.
- **CVR-039, skip the source re-read and hash at parse.** Measure first.

Cancelled: **M7, remote deployment over a tailnet**, 2026-08-19, unstarted.
[`north-star.md`](north-star.md) made the product one local workspace on one
Mac. The plan is in [`archive/M7_EXECUTION.md`](archive/M7_EXECUTION.md).

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

`pnpm verify` is desktop-scoped, so in-flight backend work cannot fail a
desktop change. `verify:container` needs Docker and minutes, so it stays out of
`verify:all` and is a local release gate. CI runs frontend, backend, desktop and
dependencies as four jobs. `dependencies` can go red on a commit that changed
nothing, because the RustSec database moves.

## Ownership

| Session | May edit | Must not edit |
| --- | --- | --- |
| Backend | `apps/converter/**`, `docs/*.md` | `apps/desktop/**` |
| Desktop | `apps/desktop/**` | `apps/converter/**` persistence, worker, OpenAPI |
| Either | `README.md`, `CLAUDE.md`, `AGENTS.md`: append or reconcile | The other session's section. Shared repo files (`.github/`, `LICENSE`, `rust-toolchain.toml`, `.node-version`, `.gitignore`, `package.json`) |

## Doc map

| File | Role |
| --- | --- |
| `docs/STATUS.md` | This file. The single live status document |
| `docs/north-star.md` | The local-first product target |
| `docs/AUDIO_EPIC.md` | The local transcription epic: evidence, decisions, sprint 1 |
| `docs/DESIGN_PORT.md` | Why the design system looks the way it does. Sources in `design/` |
| `docs/archive/` | Everything closed, with a note on why |
| `README.md` | How to run, test, and release |
| `CLAUDE.md` | Desktop architecture contract and the gotcha list |
| `AGENTS.md` | Learned user preferences and workspace facts |
| `apps/desktop/src/ui/UI.md` | Design language and the rules for extending it |
| `apps/converter/README.md` | Converter setup, environment variables and traps |
| `apps/converter/evals/README.md` | Corpus manifest data rules |
| `.impeccable.md` | Design context read by every `/impeccable` skill |
