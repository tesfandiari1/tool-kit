# Audio epic: local transcription

Transcribe runs on this Mac, on the Neural Engine, through one Swift worker
inside the converter. Rev.ai stays as the Direct route.

**Last updated:** 2026-09-12. Status: sprint 1 landed and reviewed, uncommitted,
in the installed bundle (cdhash `8448ad9e`).

## Evidence

Measured in `~/code/mlx-lab` (M2 Max, 32 GB, macOS 26.6.1) and re-measured
on 2026-09-12 by `swift/tk-audio-spike/` there, on the same 16.2-minute
two-speaker recording. WER against Rev.ai is disagreement, not error: nobody
hand-labelled that file. Rev.ai and AssemblyAI disagree with each other by 9.6%.

| Engine | Chip | Realtime | vs Rev.ai | Peak RSS | On disk |
| --- | --- | --- | --- | --- | --- |
| Apple SpeechAnalyzer | ANE | 75.7x | 18.6% | 23 MB, work is in `speechanalysisd` | 0, locale assets from Apple |
| Parakeet TDT 0.6B v3, FluidAudio CoreML | ANE | 138.8x | **10.1%** | 626 MB | 470 MB |
| Parakeet TDT 0.6B v3, `parakeet-mlx` (mlx-lab) | GPU | 74.4x | 10.5% | 3.06 GB GPU | 2.3 GB |
| FluidAudio offline diarizer, 2 speakers pinned | ANE | 246x | 95.7% talk time right (mlx-lab) | 858 MB | 21 MB |

Whisper lost every benchmark and loops on trailing audio. MLX diarization
needs 20 GB of GPU for 16 minutes offline and loses speaker identity when
streamed. **MLX has no role in audio.** SpeechAnalyzer and FluidAudio both run
on the Neural Engine, so a transcription never competes with anything on the
GPU.

The spike also proved the packaging: the SwiftPM binary runs alone without the
`FluidAudio_FluidAudio.bundle` SwiftPM emits beside it (LuxTTS data only), runs
under `env -i`, and Parakeet chunks long audio internally (15 s window, 2 s
overlap). `ModelHub.offlineMode` and the manual loaders
`AsrModels.load(from:)` and `OfflineDiarizerModels.load(from:)` exist and are
cited from source, not yet run.

## Decisions

1. **One worker, `tool-kit-audio-worker`, Swift, SwiftPM, one dependency
   (FluidAudio 0.17.5).** No Python, no MLX, no GPU memory in the bundle.
   The first SwiftPM build in the tree: `swift build -c release`, not `swiftc`.
2. **It is the converter's fourth engine, in the Vision worker's shape.**
   Staging dir as argv[1], source bytes on stdin, `env_clear` plus the shared
   `TOOLKIT_WORKER_*` bindings, a report file, a `--version` handshake, exit 0
   or 70. Advertised only where the worker is present, like Vision.
3. **SpeechAnalyzer by default. Parakeet is the `accurate` tier, sprint 2.**
   6.5 points better on conversation, 470 MB to fetch.
4. **Speakers come from FluidAudio's offline diarizer, always with a count.**
   Left to guess it found 4 people in a 2-person file. The count is a per-run
   setting, blank means guess and the transcript header says so.
5. **Output is Markdown speaker turns, extension `.md`.** `[mm:ss] Speaker 1`
   then the text, neighbouring lines from one speaker merged. The converter
   publishes Markdown and nothing else.
6. **Transcribe follows `conversion_route`.** Backend is the sidecar's audio
   engine, Direct is Rev.ai unchanged. No second route setting.
7. **No Rev.ai fallback from a failed local job in v1.** A rejection fails the
   job with its reason. `needs_remote` exists on the wire for later.
8. **The worker never reaches Hugging Face.** `ModelHub.offlineMode = true`,
   models load from an explicit directory env, and an unset env is fatal
   (`TOOLKIT_CONVERTER_PDF_BCMAPS_DIR` already burned once by failing silently).
   The diarizer's 21 MB ships in `Contents/Resources`. Parakeet's 470 MB is
   sprint 2, through the Settings model list below.
9. **Audio gets its own ceilings.** `max_upload_bytes` and `pdf_timeout` are
   25 MB and 60 s. An hour of WAV clears both by an order of magnitude.

## Decisions taken 2026-09-12

Q1, Q3, Q4 and Q5 at their defaults: SpeechAnalyzer installs locale assets
from Apple inside the first job; audio gets a 1 GB ceiling and a 30 minute
timeout on their own keys; sprint 1 admits wav, m4a, mp4, mov, mp3 and flac;
the speaker count is a per-run Settings field, blank means guess.

Q2, Parakeet's 470 MB: **a model list in Settings, not the DMG.** The pattern
is Dictation languages and Xcode Components, and every FluidAudio app does it:
one row per optional model with name, purpose and size, one control that
cycles Download, progress, Installed, Remove.

- The desktop drives the download, because network calls live in Rust. It
  spawns `tool-kit-audio-worker --fetch <model> <dir>`, which reuses
  FluidAudio's own downloader and prints progress lines the desktop turns into
  a `model-updated` event.
- The job path never goes online. `ModelHub.offlineMode` is set at worker
  start and models load from an explicit directory. `--fetch` is the one path
  allowed on the network.
- Models live in `<app_data>/converter/models/<repo>/` with a `manifest.json`
  recording repo, revision, file hashes and the FluidAudio version. Remove
  deletes the directory. A mismatch against the worker's pin reads as not
  installed.
- No "latest" control in v1. The CoreML layout is coupled to the FluidAudio
  version compiled into the worker, so the pin moves with an app update.
  Re-download covers a corrupt install.
- The tier control stays disabled until the model is installed. Picking
  Accurate first offers the download.
- The diarizer's 21 MB still ships in the bundle, fetched at build time by the
  same `--fetch` into `resources/`, so the default path needs no download.

## Blast radius

GitNexus, upstream, before any edit. The index is 5 commits behind HEAD.

| Symbol | Risk | Note |
| --- | --- | --- |
| `run_job` | CRITICAL | Not edited. It already delegates to `run_backend_job` when `job.backend` is set. The change lands in `run_pipeline`'s job construction |
| `output_extension_for` | CRITICAL | Where `.txt` versus `.md` for a backend transcript lands. Feeds `result_extensions_for` |
| `route_for` | CRITICAL | The one engine-to-route rule. Its `LocalAudio` arm lands in the same commit as the `LocalEngineKind` variant |
| `result_extensions_for` | HIGH | One caller, the tree's pairing rule, fanning out across the whole library. A wrong Transcribe answer unpairs every transcript silently |
| `JobType` | HIGH in practice | 44 `JobType::` sites across `jobs.rs`, `lib.rs`, `tree.rs`. The index does not track enum variant use |
| `servable_media_types` | LOW | Two callers. The positional `vision_available` bool becomes an availability record |

## Architecture

```text
Tool-Kit.app/Contents/
  MacOS/       tool-kit  tool-kit-converter  tool-kit-pdf-worker
               tool-kit-vision-worker  tool-kit-audio-worker
  Resources/   pdf-inspector/bcmaps/
               fluidaudio/speaker-diarization-coreml/speaker-diarization/  (25 files + manifest)

desktop run_pipeline ──(conversion_route = Backend, ext in capabilities)──▶ sidecar
  POST /api/v1/conversions  multipart: source, profile, speakerCount
    converter admits by container magic ▶ AudioEngine ▶ spawn worker
      worker: SpeechAnalyzer ▶ FluidAudio diarizer ▶ join ▶ result.md + report
    GET …/artifacts/markdown ▶ desktop writes {stem}.md beside the source
```

```text
crates/worker-protocol/src/audio.rs     the wire contract (mirrors vision.rs)
workers/audio/                          Package.swift, build.sh, Sources/, bin/ (gitignored)
apps/converter/src/engines/audio.rs     AudioEngine (mirrors vision.rs)
apps/converter/tests/audio_worker.rs    grades the protocol, not transcript quality
```

About 2,400 lines across 30 files. Full file-by-file map in the workflow
journal for run `wf_34e6a624-9a5`.

## Sprint 1: a local transcript end to end

Every ticket lands behind a green gate. Worker code seeds from
`~/code/mlx-lab/swift/tk-audio-spike/Sources/tk-audio-spike/main.swift`.

| Id | Ticket | Files | Gate |
| --- | --- | --- | --- |
| AUD-1 | Audio wire contract | `crates/worker-protocol/src/audio.rs`, `lib.rs`; `apps/converter/src/lib.rs` re-export | converter clippy |
| AUD-2 | Worker: handshake, SpeechAnalyzer, bundled diarizer, Markdown turns, report | `workers/audio/{Package.swift,build.sh,Sources/}` | `workers/audio/build.sh && bin/tool-kit-audio-worker --version` |
| AUD-3 | Protocol suite on the `vision_worker.rs` shape, one fixture per container plus a two-speaker clip | `apps/converter/tests/audio_worker.rs`, `tests/fixtures/` | `cargo test --test audio_worker` |
| AUD-4 | Admission: `LocalEngineKind::Audio`, container magics, `SOURCE_FORMATS`, migration 0005, availability record replacing the Vision bool | `conversion/model.rs`, `migrations/0005_audio_source_formats.sql` | `cargo test --lib conversion::model` |
| AUD-5 | `AudioEngine`, config (worker path, models dir, own timeout and ceiling), app wiring, `RouteKind::LocalAudio`, `ReasonCode::TranscribedAudio`, `speakerCount` threaded like the OCR fields | `engines/audio.rs`, `config.rs`, `app.rs`, `conversion/{policy,service}.rs`, `persistence/` | `cargo test --test routing_policy` and `--test crash_recovery` |
| AUD-6 | First transcript over HTTP: capabilities advertise audio, submit, poll, fetch | `api/{capabilities,conversions}.rs`, `tests/http_contract.rs` | `cargo test --test http_contract` |
| AUD-7 | Contract: media types, engine, route, reason, `speakerCount`, version bump, regenerate `schema.ts` | `contract/http/openapi.yaml` | `pnpm lint:api && pnpm verify:api-drift` |
| AUD-8 | Bundle: build and stage the fourth sidecar and the diarizer resource, `externalBin`, release sweep | `build-sidecars.sh`, `verify-release.sh`, `tauri.conf.json` | `pnpm sidecars && verify-release.sh` |
| AUD-9 | Desktop route: `run_pipeline` and `scan_inputs` take Transcribe to the backend, `result_extensions_for` pairs `.md`, form carries `speakerCount` | `lib.rs`, `jobs.rs`, `conversion_service.rs`, `settings.rs` | desktop `cargo test --lib` |
| AUD-10 | Speaker count in Settings, mirrored in `types.ts` `DEFAULT_SETTINGS` | `src/app/types.ts`, `src/domains/settings/` | `pnpm lint && pnpm build`, then run the app |

All ten landed 2026-09-12. Gates: `pnpm verify` (167 desktop tests),
`pnpm verify:backend` (208 converter tests, the with-worker HTTP transcription
among them), `verify-release.sh` green on four signed sidecars, and the
installed bundle transcribed the two-speaker fixture through its own converter
and worker in 2 s with two named speakers. One pre-existing flake surfaced as
the suite grew: the PDF engine's handshake window was 2 s where Vision's and
Audio's are 10, and a fork storm from the parallel suite on a loaded Mac timed
out a script that only echoes. It is 10 s now. What remains manual: drop a
recording on the running app and read the `.md` beside it.

**Review pass, 2026-09-12.** Four finders, four refuters, four owners landing
fixes: 25 findings, 24 confirmed, 1 refuted, all 24 fixed behind the crate
gates. The ones that mattered:

- The run door demanded a Rev.ai key for a local transcription and skipped the
  backend preflight for Transcribe, so the feature was unreachable without a
  key nobody needs. `App.tsx`, `routes.ts`, `plan.ts`, plus `transcribeFiles`
  on `Scan`.
- A recording with under about two seconds of speech transcribed and then
  died as `worker_crash`: FluidAudio's diarizer throws `noSpeechDetected` on
  short audio. The worker now answers one speaker there.
- A queued audio job was failed for good on any boot without the audio engine.
  The claim query now takes the boot's servable media types, so the row waits.
- Convert routed anything mime_guess calls audio or video, which flipped
  autodetect. Convert now drops those before routing.
- A local `.md` transcript unpaired when the route flipped to Direct.
  Transcribe pairs on `md` and `txt` on both routes.
- A SIGKILLed worker left its up-to-1 GB scratch source in an attempt
  directory nothing swept. The engine removes it on every failure path.
- Capabilities advertised one upload ceiling while audio had another.
  `maxAudioUploadBytes` is on the wire.
- The container signature was checked only after a whole upload landed on
  disk. It is checked the moment the 1 KB window fills.
- The audio suites skipped silently on any machine but this one. `build.sh`
  stages the diarizer models, and a skip prints why.
- The worker linked fastcluster and VBx and neither notice shipped. The
  bundle carries FluidAudio's third-party licences and the sweep checks them.

## Sprint 2

- **Parakeet `accurate` tier.** The Settings model list above, the tier
  setting, the worker flag, a warm-load measurement (only the cold 49 s
  including download exists).
- **Long recordings, measured 2026-10-02** through the bundled converter on
  real files. Every one covered to its last minute:

  | Recording | Size | Wall time | Worker peak |
  | --- | --- | --- | --- |
  | 78 min m4a, Zoom workshop | 49 MB | 83 s | 991 MB |
  | 112 min WAV, event | 1129 MB | 80 s | 1163 MB |
  | 132 min WAV | 1337 MB | 132 s | 1263 MB |

  The 1 GiB upload ceiling refused both WAVs, so the app now passes the
  converter's own 4 GiB bound (`41eddf6`). Past 132 minutes is unmeasured.
- **Container widening**, each with a fixture, and the Rev.ai fallback
  question once a rejection has been seen in the wild.

## Next epic: transcripts an AI can read

Stated 2026-09-12: the transcript Markdown must carry the recording's
metadata and structure (`#`, `>`, `-`) so an AI tool reading the file
understands the conversation's context without the audio.

Sprint 1's output is deliberately the minimum: turns only. The shape is the
worker's alone (`workers/audio/Sources/tool-kit-audio-worker/render.swift`),
so the next epic changes one file plus whoever supplies the fields.

Proposed shape, open for review:

```markdown
---
source: interview.m4a
duration: "16:13"
recorded: 2026-08-19
transcribed: 2026-09-12
language: en-US
speakers: 2
speakers-guessed: false
engine: local-audio 26.6.2+fluidaudio-0.15.6
---

# interview

## Speakers

- Speaker 1, 08:12 of talk time (51%)
- Speaker 2, 07:48 (49%)

## Transcript

### [00:00] Speaker 1

> And I think I get the gist of it. I want to ask you one thing …

### [00:16] Speaker 2

> A piece of content. What is a piece of content? …
```

Decided 2026-09-12: **the desktop prepends the front matter** when it writes
the file, and the worker stays a pure transcriber. The worker's fields
(duration, locale, speakers, talk time) reach the desktop through the
manifest's diagnostics, so the worker's report grows a talk-time field and
nothing else. The source name, its mtime and the run date are the desktop's
already.

Still open:

- **Front matter versus the sidecar.** `north-star.md` puts provenance in
  `{slug}.document.json` and keeps the Markdown clean. An AI tool reads the
  Markdown, so a short front matter block stays in the file and the full
  manifest goes to the sidecar.
- **Speaker names.** `Speaker 1` is what the diarizer knows. A per-file rename
  (Deborah, Contractor) is a document edit, not a run setting.
- **Structure past turns.** Chapters, topics and a summary need a language
  model. Apple's on-device model runs on the ANE with no egress, which is the
  one enrichment `north-star.md`'s consent rule does not block. Entities
  (names, dates, amounts) come free from `NLTagger`. Both are candidates,
  neither is committed.

## Improvements folded into sprint 1

- `engines/child.rs` already held the report read and the staged-Markdown
  checks, so `audio.rs` mirrors `vision.rs`'s forty lines of outcome mapping
  and no trait was added (AUD-5).
- `is_servable(vision_available: bool)` becomes an availability record (AUD-4).
  A second positional bool is how audio ends up gated on the Vision worker.
- `config.rs` repeated the explicit-env-then-sibling-probe per worker. One
  helper, `optional_worker_path` (AUD-5).
- `RouteKind` carried a stale `#[expect(clippy::enum_variant_names)]` comment,
  fixed with the fourth `Local` variant (AUD-4).
- `ConversionService::new` reached clippy's seven-argument limit with the
  fourth engine and carries an `#[expect]` rather than a one-use engines
  record (AUD-5).

Deferred: the `run_pipeline` backend branch restructure (it hardcodes
`JobType::Convert` and must become job-aware, but restructuring it in the same
change makes a CRITICAL diff unreviewable) and the history provenance fix.

## Traps

- `Contents/MacOS` is sealed nested code. The worker goes there. Models and
  the SwiftPM `.bundle` never do, and a data file there passes every
  build-machine check and fails on the user's Mac.
- `env_clear` strips HOME. FluidAudio resolves its default cache through
  `getpwuid`, so it worked in the spike, and that is exactly why the models dir
  must be an explicit env with an unset value fatal.
- The diarizer env names the PARENT directory: FluidAudio's
  `Repo.diarizer.folderName` is `speaker-diarization`, and both `--fetch` and
  `OfflineDiarizerModels.load(from:)` append it themselves.
- `mime_guess` answers `audio/m4a` for `.m4a` while the contract says
  `audio/mp4`. The desktop's one `media_type()` maps it, or every m4a upload
  is refused 415.
- The diarizer's own speech detector is stricter than SpeechAnalyzer's:
  `OfflineDiarizationError.noSpeechDetected` on a short or sparse recording
  the recogniser already transcribed. One speaker is the answer, not a crash.
- `AsrManager` has no `transcribe(samples, source:)` at 0.15.6 despite the
  docs. Every overload takes `decoderState: inout TdtDecoderState`.
- The offline diarizer prints `[Profiling]` lines unconditionally. They landed
  on stderr in the spike. The report file is the only channel back anyway.
- Swift 6 strict concurrency rejects a `Task` returning `[[String: Any]]`. The
  collector needs a concrete `Sendable` row.
- `downloadAndLoad` fetches four `.mlmodelc` bundles at int8 (470 MB), not the
  3.59 GB the HF tree lists. Size off the manifest, not the listing.
- `libtext_processing_rs.a` (58.7 MB, prebuilt Rust, TTS only) linked into the
  worker until 0.17.5, where `traits: []` drops it. The release binary went
  from 16.6 MB to 10.0 MB.
- FluidAudio's own `ThirdPartyLicenses/` ships in the bundle. Its vendored
  fastcluster is BSD and requires the notice with a binary redistribution, so
  Apache-2.0 alone does not cover the worker. `verify-release.sh` checks the
  fastcluster and VBx files.
- The diarizer set is re-fetched when the staged `manifest.json` records a
  different `fluidAudioVersion` than the worker's `--version`. Presence alone
  shipped the previous release's models against a bumped pin.
- Do not add `prefixItems` for `inputFormats` or `engines` in
  `.spectral.yaml`. A fourth engine widens the spread again.
- `advertised_media_types_match_the_migration_check` parses the newest
  migration's CHECK, and a second test diffs the OpenAPI `contentType` line
  against the same table. A `SOURCE_FORMATS` row without both fails the lib
  tests.
- `types.ts` is not generated. A Rust setting missing from `DEFAULT_SETTINGS`
  is dropped on the next `save_settings` round-trip.
- Every write-back inside `run_backend_job` keeps its `stale()` check. A Stop
  mid-transcription must not leave a row no task finishes.

## Verify

```bash
workers/audio/build.sh                                   # the worker alone
cargo test --manifest-path apps/converter/Cargo.toml --test audio_worker
pnpm verify:backend                                      # lint:api, converter clippy and tests
pnpm verify                                              # desktop
pnpm tauri build && apps/desktop/src-tauri/scripts/verify-release.sh
```
