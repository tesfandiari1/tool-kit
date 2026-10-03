export const meta = {
  name: 'memory-fixes',
  description: 'Land 3 memory fixes (anydoc to worker, PDF double parse, streamed uploads), each implemented then challenged',
  phases: [
    { title: 'Converter lane', detail: '#1 anydoc worker + #2 PDF parse, then Challenger' },
    { title: 'Desktop lane', detail: '#3 streamed uploads, then Challenger' },
  ],
}

const S = args.scratch
const RULES = `
Ground rules (each has bitten this repo before; breaking one fails your task):
- Never generate synthetic load or leave a process running. Bound every process you start with a timeout and kill it by a captured PID (zsh: PIDS=($(...))). Confirm with ps before you finish.
- Edit ONLY the files your lane names. No doc edits (docs/, CLAUDE.md, STATUS.md) — the orchestrator does those. No git add, commit, stash, reset, checkout or clean. Never run \`node .gitnexus/run.cjs analyze\` and never run \`pnpm sidecars\`.
- No live API calls to Datalab or Rev.ai (they bill). No new dependencies. Do not fork or patch any crate or package.
- Lint before tests. Trim tool output at the source (| tail -n 60, or redirect to ${S} and read the tail).
- Scratch files only under ${S}/fix/.
- Karpathy + ponytail full: every changed line traces to the task; reuse what the tree has; deletion over addition; one runnable check per non-trivial branch; match the surrounding comment density and style (terse, no narration, no em dashes or semicolons in comments).
- The user may edit the repo in another terminal: re-read a file before editing it.
- Quote numbers only from commands you ran.
Repo /Users/tristin/code/tool-kit, HEAD ${args.head}, macOS 27.0.1 Apple silicon. /Applications/Tool-Kit.app is a signed build of HEAD: its Contents/MacOS binaries are the BEFORE baseline.
Measurement harness from the earlier pass: ${S}/measure/measure_v3.py (drives a private tool-kit-converter over HTTP with its own data dir and token, samples \`footprint\`). Fixtures: ${S}/measure/fx (big.docx 430 KB, big3.docx 1.3 MB, text.docx, native-text-x20.pdf 600 p, scanned.pdf 8 p, ...). Measure phys_footprint (footprint, or /usr/bin/time -l "peak memory footprint"), never RSS.`

const IMPL = {
  type: 'object',
  properties: {
    files_changed: { type: 'array', items: { type: 'string' } },
    summary: { type: 'string', description: 'what changed and why, terse' },
    gates: { type: 'array', items: { type: 'object', properties: { command: { type: 'string' }, result: { type: 'string' } }, required: ['command', 'result'] } },
    measurements: { type: 'string', description: 'before/after numbers with the command that produced each' },
    parity: { type: 'string', description: 'output-equivalence evidence' },
    deviations: { type: 'string', description: 'where you departed from the brief and why, or none' },
  },
  required: ['files_changed', 'summary', 'gates', 'measurements', 'parity', 'deviations'],
}
const CHALLENGE = {
  type: 'object',
  properties: {
    verdict: { type: 'string', enum: ['ship', 'ship_with_changes', 'block'] },
    changes_made: { type: 'array', items: { type: 'string' }, description: 'each edit you applied, file:line and why' },
    unresolved: { type: 'array', items: { type: 'string' }, description: 'real problems you could not fix in scope' },
    gates: { type: 'array', items: { type: 'object', properties: { command: { type: 'string' }, result: { type: 'string' } }, required: ['command', 'result'] } },
    independent_measurements: { type: 'string' },
    net_diff: { type: 'string', description: 'git diff --stat for your lane files after your edits' },
  },
  required: ['verdict', 'changes_made', 'unresolved', 'gates', 'independent_measurements', 'net_diff'],
}

const CONVERTER_GATES = `cargo clippy --locked --manifest-path apps/converter/Cargo.toml --all-targets -- -D warnings; cargo fmt --manifest-path apps/converter/Cargo.toml --check; cargo test --locked --manifest-path apps/converter/Cargo.toml`
const DESKTOP_GATES = `cargo clippy --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings; cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --check; cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --lib`

const CHALLENGER_PERSONA = `You are THE CHALLENGER: a ruthless senior reviewer who has been paged at 3am for code exactly like this. Another agent just implemented the change below in the working tree (uncommitted). Assume it is wrong until you prove otherwise, and assume it is too big until you have tried to shrink it.
1. Read \`git diff\` for the lane files and the code around every hunk. Trace the real flow end to end.
2. Try to BREAK it: behaviour parity with HEAD (outputs, error messages, rejection codes, retry/billing semantics), cancellation and timeouts, permits, process cleanup, temp-file cleanup, source integrity (the bytes parsed must be the bytes validated), cfg(target_os) on Linux (the converter also ships in a Linux Docker image), test coverage of the new branch.
3. Try to SHRINK it: delete code the change does not need, abstractions with one caller, narrating comments, dead helpers it orphaned, any re-implementation of something already in the tree.
4. FIX what you find directly in the lane files (same scope rules as the implementer). Do not re-architect a sound design to taste. Do not expand scope.
5. Re-run the gates and REPRODUCE the acceptance measurement yourself. Do not trust the implementer's numbers.
Verdict "block" only for a defect you could not fix in scope.`

phase('Converter lane')
phase('Desktop lane')

const converterLane = async () => {
  const impl = await agent(`Implement fixes #1 then #2 in the converter crate. ${RULES}

Lane files: apps/converter/src/engines/anydoc.rs, apps/converter/src/bin/tool-kit-pdf-worker.rs, apps/converter/src/app.rs, apps/converter/src/engines/child.rs (only if a shared helper needs exposing), crates/worker-protocol/src/* (only if you add a report type), and test files under apps/converter/tests/ or the #[cfg(test)] modules of those files.

## #1 Move AnyDoc parsing out of the long-lived converter (the main win)
Measured: a 1.3 MB text DOCX took tool-kit-converter from 4.5 to 458 MiB phys_footprint and left 110 MiB retained after the job (freed, not leaked: macOS 27 xzone malloc keeps freed pages and malloc_zone_pressure_relief returns 0). The heavy work is anydoc::to_markdown_bytes and anydoc::to_document running in-process on spawn_blocking (anydoc.rs convert, marked_pictures). The file header records the 2026-08-18 in-process decision and names "the child-worker shape in pdf_inspector.rs" as the fallback: take that fallback, and rewrite the header to say why (measured memory retention), in two or three lines.

Design (deviate only with a stated reason):
- Add an anydoc mode to the EXISTING tool-kit-pdf-worker binary (no new sidecar: packaging, signing and the Docker image stay unchanged). E.g. \`tool-kit-pdf-worker --anydoc <admitted_label> [--mark] <output_dir>\`. Reuse the worker's stdin protocol exactly: the converter passes the validated open file as stdin with WORKER_EXPECTED_SOURCE_BYTES_ENV / _SHA256_ENV / WORKER_MAX_OUTPUT_BYTES_ENV, and the worker verifies length and digest (copy_exact_source) before parsing. env_clear, kill_on_drop, wait_for_child with the engine timeout and cancellation, exactly as PdfInspectorEngine::convert does (pdf_inspector.rs ~86-130). The engine needs the pdf worker path: wire it from settings.pdf_worker_path in app.rs.
- Format detection, the PDF refusal, the CSV-by-label rule and the cross-family label check move into the worker (Format::from_bytes needs the whole ZIP), so the converter never reads the document into memory.
- In one worker run: write the plain Markdown (to_markdown_bytes of the source) to the staged markdown path. When --mark is passed (describer configured) and the format is Docx or Pptx, also run marked_pictures and write the marked Markdown (to_markdown_bytes of the marked package, tokens unfilled), the pictures as files, and placements plus the rendered token count. anydoc::document_to_markdown is private in 0.2.4, so the marked path really does need its own to_markdown_bytes parse; that is fine inside a child.
- The converter keeps describe() and fill_descriptions(): if pictures exist it runs the Vision describer over the worker's picture files, fills the marked Markdown, and on Some replaces the staged Markdown. Otherwise the plain Markdown stands. This must reproduce today's output exactly, including "a skipped picture loses Office's guess once one picture is described" and the fallback to unmarked output.
- Report: smallest thing that carries format label, processing_time_ms, rejection code, and whether marked output exists. Prefer reusing the worker-protocol report pattern; a small anydoc report type there is fine. Keep the AnyDocRejection codes and messages the API returns unchanged.
- Delete what the move orphans: run_bounded, catch_unwind plumbing, the permit-in-closure dance and its ponytail comment (a killed child releases everything). Keep EngineFailure mapping: nonzero exit or panic → Crashed, timeout → Timeout, bad report → Protocol. Empty Markdown → invalid_document. Output over max → document_exceeds_limits.
- Tests: keep the pure-function unit tests (mark_tags, fill_descriptions, collect_tokens). Engine-level tests that now need the worker binary go where CARGO_BIN_EXE_tool-kit-pdf-worker is available (integration tests, see tests/failure_modes.rs:265).

Acceptance for #1 (all required):
a) Gates green: ${CONVERTER_GATES}
b) Parity: build release binaries (cargo build --release --manifest-path apps/converter/Cargo.toml --bins). Convert every non-PDF fixture you can find (apps/converter/tests/fixtures, ${S}/measure/fx: docx, pptx, xlsx, epub, odt, ods, odp, rtf, doc, ppt, xls, csv) with BOTH the /Applications HEAD converter and the new build (same env, separate data dirs) and diff the Markdown byte for byte. Include at least one DOCX or PPTX with pictures, so the describe path runs on both. Report any difference and fix it.
c) Memory: with the new build, run big.docx and big3.docx through the converter (measure_v3.py pattern, pointing TOOLKIT_CONVERTER_PDF_WORKER_PATH at the new worker). Report converter baseline, peak, after, plus the worker's peak. Target: converter peak under 30 MiB and back within 5 MiB of baseline.

## #2 Mixed-PDF double parse (measurement-gated)
tool-kit-pdf-worker.rs: for a PDF that needs OCR on some pages, the first pass (process_pdf_mem_with_options, ScanStrategy::Full) builds whole-document Markdown that is thrown away, then stage_native_pages re-reads the source and calls extract_pages_markdown_mem. Facts already checked: ProcessMode::Analyze is NOT an option, because in Analyze mode pdf-inspector 1.25.2 skips detect_encoding_issues(md) (lib.rs ~4842-4870), which would change has_encoding_issues and the routing. Minimal candidate: release the first result's Markdown (and anything else large) before the second parse, and on macOS reuse the bytes already read instead of reading the tempfile again. Measure BEFORE (the /Applications worker) and AFTER on a mixed PDF: build one with pdfunite or gs from native-text-x20.pdf plus scanned.pdf, run each worker directly with /usr/bin/time -l (stdin = the file, env as pdf_inspector.rs sets, RAYON_NUM_THREADS=2, a temp output dir), 3 runs each. Land it only if the peak footprint drops 10% or more AND the report and native_pages.json are byte-identical. Otherwise revert #2 and report the numbers.

Return the schema. In measurements give #1 and #2 separately.`, { label: 'implement #1 + #2', phase: 'Converter lane', effort: 'high', schema: IMPL })

  const challenge = await agent(`${CHALLENGER_PERSONA} ${RULES}

Lane: converter crate. Files in scope: ${(impl && impl.files_changed || []).join(', ') || 'apps/converter/**, crates/worker-protocol/**'}.
Implementer's report:
${JSON.stringify(impl, null, 1)}

Focus points for this lane:
- Output parity with HEAD on every non-PDF fixture, including a describe-path DOCX or PPTX. Re-run the byte-for-byte diff yourself against /Applications/Tool-Kit.app/Contents/MacOS/tool-kit-converter.
- The source integrity chain: the worker parses only bytes it hashed.
- The describe fallback semantics are unchanged. Worker temp files (pictures, marked Markdown) are cleaned up, or live under the attempt dir that the existing cleanup already removes.
- Nothing stays in the converter that still reads the whole document.
- #2: if it landed, re-measure. If its gain is under 10%, revert it.
- Re-measure the converter footprint on big3.docx yourself: baseline, peak, after.
Gates: ${CONVERTER_GATES}`, { label: 'challenger: converter', phase: 'Converter lane', effort: 'xhigh', schema: CHALLENGE })
  return { impl, challenge }
}

const desktopLane = async () => {
  const impl = await agent(`Implement fix #3 in the desktop crate. ${RULES}

Lane files: apps/desktop/src-tauri/src/providers.rs only (plus its #[cfg(test)] module).

providers.rs:119-150 read_file_bytes reads the whole upload into one Bytes in the long-lived Tauri host for the whole Direct-route upload (up to UPLOAD_TIMEOUT, 30 min, plus retries), and up to 4 jobs run at once. Callers: datalab_submit (~236), datalab_pipeline_submit (~360), revai_submit (~508), all through send_retrying, whose make closure rebuilds the multipart body per attempt because bodies are single-use.
Fix: stream from disk per attempt with reqwest::multipart::Part::file(path).await (reqwest 0.13, the "stream" feature is already on). The tree already does this at conversion_service.rs:582. Part::file sets Content-Length from metadata (Datalab and Rev.ai refuse chunked bodies, see the bytes_part comment), the file name, and the MIME type through mime_guess::from_ext. Delete read_file_bytes and bytes_part.
Constraints:
- An unreadable file must still fail before any request with "Could not read file: {e}". A file error must never be retried or classed as uncertain/billed. Keep SubmitError mapping identical for each caller. The smallest shape may be a check up front (open or metadata) plus a per-attempt open. Choose the smallest one that keeps send_retrying's contract (only connect errors and 429/502/503/504/529 retry). Changing send_retrying's signature is allowed only if it ends up simpler.
- File name: today a non-UTF-8 name falls back to "file"/"media", and Part::file uses to_string_lossy. Keep today's behaviour only if it costs at most a line. Otherwise state the difference.
- MIME: today mime_guess::from_path(...).first_or_octet_stream(). Confirm Part::file gives the same for the extensions that reach these paths.
- One runnable check: a test that sends a submit through the new path to a local listener (reuse any local-server test pattern in this crate, e.g. conversion_service.rs tests) and asserts that Content-Length is present, the file bytes arrive intact, and the filename matches. No network beyond 127.0.0.1.
Gates: ${DESKTOP_GATES}
Measurements: none needed beyond the test. Say so.`, { label: 'implement #3', phase: 'Desktop lane', effort: 'high', schema: IMPL })

  const challenge = await agent(`${CHALLENGER_PERSONA} ${RULES}

Lane: desktop crate, apps/desktop/src-tauri/src/providers.rs only.
Implementer's report:
${JSON.stringify(impl, null, 1)}

Focus points: retry and billing semantics are unchanged (CLAUDE.md: only failures that prove the server never started work may retry, and only SubmitError::Refused may clear a billing claim); Content-Length is always set; the error text matches today's; no second copy of the file is held anywhere; the test really exercises the production path and not a copy of it. Shrink it.
Gates: ${DESKTOP_GATES}`, { label: 'challenger: desktop', phase: 'Desktop lane', effort: 'high', schema: CHALLENGE })
  return { impl, challenge }
}

const [converter, desktop] = await parallel([converterLane, desktopLane])
return { converter, desktop }
