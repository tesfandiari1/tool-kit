# Tool-Kit status

Where the product stands, what is broken, and what someone still intends to
build. CLAUDE.md owns the architecture contract and is not repeated here.
Closed work is in [`archive/`](archive/README.md).

**Last updated:** 2026-10-03

## Current state

| Item | Value |
| --- | --- |
| Branch tip | `main`, not pushed. Sprint 1, local PDF OCR, image descriptions, the 2026-10-01 upgrades, the 2026-10-02 code-review fixes (`44867b9` to `aaea300`) the 2026-10-02 PDF splice (`87d482b`), the 4 GiB audio ceiling (`41eddf6`) and no-copy drops (`b1f36be`) are committed |
| OpenAPI contract | 0.5.0 (`contract/http/openapi.yaml`). The converter crate moves with it |
| Desktop version | 1.0.0 (`package.json`, `Cargo.toml`, `tauri.conf.json`) |
| Installed bundle | Stale: `/Applications/Tool-Kit.app` is a signed build of `d9a0cb5`, installed 2026-10-03. It predates no-copy drops, the AnyDoc worker and the local-only removal |
| Backend | M0 to M4 complete. M5 and M7 cancelled |
| Desktop | M6 landed. Gate open on CVR-067 and CVR-081 |
| Service | Sidecar inside the `.app`, loopback only. Docker needs a `backend-override.json` that only `pnpm backend:docker` writes |
| Container smoke | `20260819T161851Z`, image `sha256:4a0cbdd0…`. It predates the worker-protocol refactor in `d4f83eb`, so re-run it before closing converter work |

The keychain entitlement is parked, so `verify-release.sh` fails its Keychain
section by design. CLAUDE.md says what restores it.

Behaviour that is a decision, not an accident:

- Local only (2026-10-03). Tool-Kit converts sensitive documents, so no file
  content leaves the Mac. Datalab, Rev.ai, the Direct route and the Standard
  profile are gone. The desktop always sends `local_only`, and a Manual backend
  must be loopback. A format no local engine takes is refused. Users lost HTML
  conversion and the recordings AVFoundation cannot read (ogg, aac, mkv, webm,
  avi, …).
- AnyDoc parses in `tool-kit-pdf-worker --anydoc`, not in the converter
  (2026-10-03). On a 1.3 MB DOCX the converter's footprint went from a 576 MiB
  peak and 258 MiB afterwards to 7.7 MiB both. Output is byte-identical on 39
  of 40 cases. The worker protocol is now 3.
- A drop is never copied. The source stays where it is and only its result
  lands in the active project, with a dropped folder's shape kept. The
  `moveDroppedFiles` setting (off by default) renames the drop into the project
  instead.
- A backend job persists its run id, idempotency key, origin, source mtime and
  profile before submission. Restart recovery replays against the recorded
  origin.

## Known issues

- The tray left-click fix in Tauri 2.12 is unverified by hand on macOS 27.
- No-copy drops (`b1f36be`) are unverified by hand: drop a folder with the move
  setting off and on, and check where sources and results land. Moving from
  another disk is refused, not supported. Copies left by older imports stay in
  the library until someone removes them.
- The PDF splice (`87d482b`) records a mixed PDF's classification as
  `image_based`, because the Vision engine reports every conversion that way.
- A crash between migration 0006's `COMMIT` and sqlx's bookkeeping row makes
  every later start fail with `duplicate column speaker_count`. Recovery: rename
  the converter data folder (`<app_data>/converter`). The window is milliseconds
  on a one-time upgrade, so no guard is planned.

Open decisions from the 2026-10-02 code review, one per owner call:

- A recording transcribed before a same-stem PDF converts makes the tree pair
  `lecture.md` with the PDF and `lecture (1).md` with the recording. Nothing
  bills twice, but the PDF row opens the transcript. Recommended: keep it until
  real libraries show the case, then name transcripts `{stem}.transcript.md`.
- A submission in flight across the `faaceb7` upgrade gets one
  `idempotency_conflict`, because the `m2` fingerprint now always carries the
  speaker field. Recommended: accept it, a re-run clears it.
- The DOCX and PPTX picture-marking pass runs even when Foundation Models is
  off, because the Vision handshake carries no availability signal.
  Recommended: leave it until profiling shows the extra parse matters.

One low item left from the verify pass over the 2026-09-12 fix:

- While the service is down, the scan counts every non-text, non-media
  extension, so the blocked hint can over-count a drop with junk in it. It
  self-corrects on the recovery re-scan. A fix needs a fourth copy of the
  converter's format list. Recommended: close it.

## Next

- **Audio epic, sprint 1 landed 2026-09-12, committed in `faaceb7`.** One Swift worker on
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
- **Convert result reuse.** A Convert lookup never matches its own
  `backend:markdown` rows, so every converted document re-runs locally. The
  scan counts reuse for Transcribe only. It costs time, never correctness. Transcripts already match. On 2026-10-02
  `history.db` held 96 converted sources and none ran twice.
- **The M6 gate.** CVR-067 waited on M5, now cancelled. Close it with the
  local-only decision.

Deferred, nobody scheduled:

- **M8, evaluation and cutover.** Moot: there is no other route to cut over
  from.
- **CVR-043, routing calibration.** Needs a signal that separates a blank page
  from a scan. `confidence` is not one.
- **CVR-080, freeze the labeled corpus.** Once the M4 fixtures stop changing.
- **CVR-039, skip the source re-read and hash at parse.** Measure first.

Cancelled: **M5, Datalab fallback in the converter**, 2026-10-03, by the
local-only decision. **M7, remote deployment over a tailnet**, 2026-08-19, unstarted.
[`north-star.md`](north-star.md) made the product one local workspace on one
Mac. The plan is in [`archive/M7_EXECUTION.md`](archive/M7_EXECUTION.md).

## Verify

```bash
pnpm check               # tsc --noEmit, eslint, vitest        (fast inner loop)
pnpm verify              # check, build, desktop clippy and Rust tests
pnpm verify:backend      # lint:api, converter clippy and tests
pnpm verify:all          # both of the above
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
| `docs/DESIGN_PORT.md` | Why the design system looks the way it does |
| `docs/archive/` | Everything closed, with a note on why |
| `README.md` | How to run, test, and release |
| `CLAUDE.md` | Desktop architecture contract and the gotcha list |
| `AGENTS.md` | Learned user preferences and workspace facts |
| `apps/desktop/src/ui/UI.md` | Design language and the rules for extending it |
| `apps/converter/README.md` | Converter setup, environment variables and traps |
| `apps/converter/evals/README.md` | Corpus manifest data rules |
| `.impeccable.md` | Design context read by every `/impeccable` skill |
