# Tool-Kit status

Where the product stands, what is broken, and what someone still intends to
build. CLAUDE.md owns the architecture contract and is not repeated here.
Closed work is in [`archive/`](archive/README.md).

**Last updated:** 2026-10-03

## Current state

| Item | Value |
| --- | --- |
| Branch tip | `main` at `35e24fa`, pushed. It holds everything through the local-only removal (`7f03a88`), the 2026-10-03 cleanup (PRs #29 and #30) and four Dependabot bumps (thiserror, uuid twice, reqwest). CI green |
| HTTP contract | `apps/converter/tests/http_contract.rs`. The OpenAPI, Spectral and Schemathesis suite was removed on 2026-10-03 |
| Desktop version | 1.0.0 (`package.json`, `Cargo.toml`, `tauri.conf.json`) |
| Installed bundle | Current: `/Applications/Tool-Kit.app` is a signed, not notarized, build of `35e24fa`, installed 2026-10-03. `verify-release.sh` passes in full |
| Backend | M0 to M4 complete. M5 and M7 cancelled |
| Desktop | M6 landed. Gate open on CVR-067 and CVR-081 |
| Service | One sidecar inside the `.app`, loopback only. The Docker image and Manual mode were removed on 2026-10-03 |

Behaviour that is a decision, not an accident:

- Local only (2026-10-03). Tool-Kit converts sensitive documents, so no file
  content leaves the Mac. Datalab, Rev.ai, the Direct route and the Standard
  profile are gone. A local engine that gives up fails the job with its reason.
  A format no local engine takes is refused. Users lost HTML
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
  origin. The 2026-10-03 cleanup kept this on purpose: a quit or a converter
  crash mid-run resumes on its own, and the converter still requeues
  interrupted jobs.

## Known issues

- Dependabot reopens the TypeScript 7 bump every week. `.github/dependabot.yml`
  has no ignore rule for it, and CLAUDE.md pins TypeScript to 6.x.

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

## macOS release pipeline rebuild (planned, 2026-10-03)

Owner decision: GitHub Actions produces verified **draft** releases. Local
builds are for development. Apple Silicon macOS `.app` and `.dmg` only;
retain `dev.esfandiari.toolkit`, developer team `92MA44797J`, and the macOS 26
runtime minimum. This is a plan; no release automation has been implemented.

Use the [Karpathy guidelines](/Users/tristin/.agents/skills/karpathy-guidelines/SKILL.md):
replace custom release machinery with supported tools, retain only necessary
native-input preparation and product-specific checks, and delete each old
path once its replacement passes. Preserve the in-flight speech-model work.

| Current piece | Disposition |
| --- | --- |
| `verify-release.sh` (248 lines) | Delete. Use native Apple validation plus focused bundle smoke tests. Preserve checks for identity, nested code, resources and notices; discard signature-output parsing and newest-DMG selection. |
| `build-sidecars.sh` (214 lines) | Replace with a small native-input preparation step. Rust/Swift compilation, target-suffixed binaries, CMaps, diarizer resources and redistribution notices remain necessary. |
| Worker `build.sh` scripts | Retain compiler entrypoints; remove duplicated model staging when the shared preparation step owns it. Adapt existing tests to consume that same staged model set. |
| Root `tauri` wrapper and repeated preparation | Move preparation to Tauri's dev/build hooks. Keep an explicit call for direct Cargo checks that run outside those hooks. |
| CI placeholder binaries/resources | Remove. Desktop CI must compile real native inputs. Missing worker/model prerequisites must fail required macOS checks instead of silently skipping them. |
| Manual signing/notarization/install recipes | Remove after cutover. Tauri owns bundling/signing, Apple owns notarization/validation, GitHub owns draft assets, Finder installs the resulting DMG. |

Implementation order and acceptance gates:

1. **Prove the hosted toolchain without signing credentials.** Build all four
   native helpers and the desktop from a fresh checkout. Honor the existing
   Node, pnpm, Rust and dependency pins. The Vision source references macOS 27
   APIs, so select Xcode 27.0 with its macOS 27 SDK while retaining the macOS 26
   deployment floor. GitHub currently documents `xcode-27` ARM64 as a preview
   runner; ordinary `macos-26` lists only Xcode 26. Verify availability and a
   real cold build before making this the release runner. If unavailable,
   report the toolchain blocker rather than removing features or introducing
   a self-hosted runner. Gate: five arm64 executables, complete native assets,
   matching worker handshakes, no developer-machine paths required at runtime.
2. **Make native preparation one dependency.** Invoke the same minimal build
   and staging step from Tauri's `beforeDevCommand`/`beforeBuildCommand` and
   desktop Cargo CI. A bundle-only hook is too late for `tauri-build`'s
   `externalBin` validation. Let Cargo/SwiftPM handle incremental compilation;
   stage the pinned diarizer set once and preserve required license notices.
   Gate: fresh `pnpm tauri dev`, an unsigned CI bundle, and desktop checks work
   without a manually run prerequisite or placeholder files.
3. **Add one manually dispatched release workflow.** Use the official
   `tauri-apps/tauri-action`, pinned to a reviewed release commit, with the
   repository's CLI and `projectPath: apps/desktop`. Run the existing quality
   gates on the same source SHA. Keep signing secrets confined to a trusted
   release job: Developer ID certificate/password and App Store Connect API
   credentials in GitHub secrets, with a temporary keychain and cleanup.
   Keep the non-secret expected identity in one configuration location.
   Gate: missing/wrong credentials fail; local dev and PR checks need none.
4. **Finish and verify exact artifacts before uploading.** Tauri signs the
   nested code and app and performs app notarization. Use Apple's `notarytool`
   and `stapler` for the DMG step the selected Tauri version does not perform.
   Consume this run's explicit artifact paths. Check Developer ID/team,
   nested signatures, app/DMG tickets and Gatekeeper with native Apple tools.
   Exercise the bundled converter/workers using existing PDF, OCR and audio
   fixtures, with the checkout's binaries/resources unavailable as fallbacks.
   Gate: a tampered helper, missing model/CMap, or rejected notarization fails
   the workflow; a required smoke test cannot pass by skipping. Hosted speech
   or AI limitations must be recorded and covered on a physical Mac before
   publication, not described as a successful end-to-end test.
5. **Create the draft from the verified output.** Run the Tauri action in
   build-only mode; after the final checks, use GitHub CLI to create/upload a
   draft tied to the built version and SHA. Attach the final DMG, its checksum
   and the verification result. Fail on a version mismatch or existing
   release rather than replacing assets implicitly. Gate: no release assets
   uploaded on failure; promotion publishes these exact bytes without a
   rebuild. Download/install on a clean Apple Silicon Mac running macOS 26
   and verify the optional macOS 27 feature separately before publication.
6. **Delete the superseded process.** Remove the old verifier, wrappers,
   placeholder staging and manual repair recipes once the first draft passes.
   Reconcile `README.md`, `CLAUDE.md`, package scripts and this status together;
   README owns the usage instructions. Gate: every documented build/release
   entrypoint uses the replacement and no callers reference the removed tools.

No custom installer/updater, release framework, additional task runner,
conversion-engine refactor, or public publication is part of this rebuild.
Only native preparation and bundle behavior tests justify custom code.

References: [Tauri action](https://github.com/tauri-apps/tauri-action),
[Tauri hooks](https://v2.tauri.app/reference/config/#buildconfig),
[macOS signing](https://v2.tauri.app/distribute/sign/macos/),
[GitHub macOS runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners),
[Xcode 27 runner inventory](https://github.com/actions/runner-images/blob/main/images/macos/xcode-27-arm64-Readme.md).

## Next

- **Audio epic, sprint 1 landed 2026-09-12, committed in `faaceb7`.** One Swift worker on
  the Neural Engine as the converter's fourth engine: SpeechAnalyzer words,
  FluidAudio speakers, Markdown turns. Transcribe follows `conversion_route`.
  A find, refute, fix review pass landed 24 fixes the same day, the run door
  that demanded a Rev.ai key for a local transcription among them.
  `pnpm verify` and `pnpm verify:backend` green, `verify-release.sh` green
  through the four signed sidecars and the bundled resources, and the installed
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

- **Converter ledger to memory.** The 2026-10-03 audit found the SQLite ledger
  (`sqlite.rs`, the migrations, `recovery.rs`, `faults.rs`, about 3,000 lines)
  is mostly crash-durability built for paid cloud calls. Replacing it is a
  rewrite with no visible gain, and auto-resume depends on it. Revisit only
  with a reason.
- **The Inbox to Drop Box migration** (`workspace.rs`, about 115 lines) can go
  once every Mac that had an `Inbox` workspace has launched a build from after
  2026-09-12. Owner call.

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
pnpm verify:backend      # converter clippy and tests
pnpm verify:all          # both of the above
pnpm verify:deps         # cargo-deny over all three crates
pnpm verify:release      # the built bundle, after pnpm tauri build
```

`pnpm verify` is desktop-scoped, so in-flight backend work cannot fail a
desktop change. CI runs frontend, backend, desktop and
dependencies as four jobs. `dependencies` can go red on a commit that changed
nothing, because the RustSec database moves.

## Ownership

| Session | May edit | Must not edit |
| --- | --- | --- |
| Backend | `apps/converter/**`, `docs/*.md` | `apps/desktop/**` |
| Desktop | `apps/desktop/**` | `apps/converter/**` persistence, worker, HTTP contract |
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
