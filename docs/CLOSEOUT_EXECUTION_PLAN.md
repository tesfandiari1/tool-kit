# Closeout plan: M4 through M8

**Drafted** 2026-08-18 from an adversarial review of the landed M1-M6 work.
**Baseline** at `ddaf52d`: 135 backend Rust tests, 61 frontend tests, both
Clippy gates and `tsc`/ESLint green.
Tickets are in `[BACKEND_EPIC.md](BACKEND_EPIC.md)`. Ownership and traps are in
`[HANDOFF.md](HANDOFF.md)`.

## Review outcome

Seven lenses produced 35 candidate defects. Each went to a skeptic told to
refute it. Ten survived, four died. The architecture is sound. The defects sit
where a decision was recorded as done and the code went elsewhere.

Three repeating patterns:

1. A rule fixed for one instance, not its class. The `structured_document`
  crash loop got a vocabulary entry. The reason it was fatal did not change.
2. A guarantee that stops at the process boundary. `providers.rs` never
  double-bills inside a run. A restart bills again.
3. A signal collected and never read. `confidence`, `isComplex`,
  `pagesWithTables`, `pagesWithColumns` all reach the manifest, and `warnings`
   is hardcoded empty at every write site.

Measured, not inferred: a PDF whose pages are 90% native text and 10% scanned
image converts, publishes, and reports `succeeded` with `warnings: []`. The
scanned page is absent from the Markdown. `confidence` is exactly the
native-page fraction, 1.0 across 1/2/4/10-page all-native controls.

## Sprint 0: repair

Confirmed, survived refutation:


| #    | Defect                                                          | Where                                   |
| ---- | --------------------------------------------------------------- | --------------------------------------- |
| S0.1 | A restart during Datalab fallback resubmits and bills twice     | `src-tauri/src/jobs.rs:838,1307`        |
| S0.2 | One unreadable row is a permanent boot failure                  | `backend/src/jobs/recovery.rs:33`       |
| S0.3 | Migrations 0002/0003 are not atomic                             | `backend/migrations/`                   |
| S0.4 | A timed-out AnyDoc parse stalls the whole queue                 | `backend/src/conversion/service.rs:401` |
| S0.5 | Recovery sends the current token to the recorded old URL        | `src-tauri/src/jobs.rs:708`             |
| S0.6 | 17 of 18 media types publish a manifest OpenAPI forbids         | `backend/openapi/openapi.yaml:671`      |
| S0.7 | Switching route never re-scans, so Run promises the wrong count | `src/shell/App.tsx:84`                  |
| S0.8 | One failed capability probe latches Run off                     | `src/shell/App.tsx:180`                 |


Ranked below, verified by hand:


| #     | Defect                                                       | Where                                   |
| ----- | ------------------------------------------------------------ | --------------------------------------- |
| S0.9  | `text/csv; charset=utf-8` is a 415                           | `backend/src/api/conversions.rs:408`    |
| S0.10 | An engine-less media type reports file corruption            | `backend/src/persistence/sqlite.rs:815` |
| S0.11 | Stop deletes the ledger row and leaves Retry live            | `src-tauri/src/lib.rs:673`              |
| S0.12 | The quit dialog says quitting writes nothing to disk         | `src-tauri/src/lib.rs:911`              |
| S0.13 | Direct-path `fail()`/`finish()` skip the `stale()` check     | `src-tauri/src/jobs.rs:1243`            |
| S0.14 | OpenAPI multipart lists 8 media types, capabilities lists 18 | `backend/openapi/openapi.yaml:73`       |


Doc corrections: the runner is sequential and a test pins it, so "one PDF and
one non-PDF job may parse concurrently" is wrong. CVR-035 is ticked with
`warnings` empty. `SOURCES.md` claims a failure fixture for every extension and
seven have none. The corpus manifest names routes the service cannot emit. The
M3 gate should cite smoke run `20260818T202130Z`, which covers 18 media types,
not `20260818T195919Z`, which covers 8.

Refuted, do not re-file: `/health/ready` as new (the write lock is real and
already an open M7 item, the rest was wrong); the M3 gate resting on stale
container evidence; `needs_remote` double-billing today; M5's remote state
being silently dropped from recovery.

### Reopened after a second review, then closed

A follow-up review found that two Sprint 0 repairs fixed the reported instance
and left the rest of its class open. Both are now closed.

| # | What was still open | Fix |
|---|---|---|
| S0.1b | `begin_fallback` returned `bool` and the result was ignored, so a failed ledger write still let the billable request go out. And the in-memory `FallbackContext` was only set *after* a successful response, so an uncertain timeout left `fallback: None` and same-process **Retry** resubmitted without any restart. | Both writes now happen before the request and both are required. The ledger survives a crash; the in-memory context survives a Retry. |
| S0.2b | Per-job containment only began after `list_recovery_candidates` materialised every row, and that used `?` per row. One malformed UUID or JSON column still aborted the boot before containment ran. | Rows decode independently into `RecoveryCandidate::{Loaded, Undecodable}`. An undecodable row is quarantined by its raw id. `list_conversion_ids` skips unparseable ids instead of aborting orphan quarantine. |
| NEW-1 | `set_status` was the last unguarded write-back in the direct path, so a Stop landing during a submit relabelled the stopped row. | Collapsed into the guarded setter. Both direct-path callers now honour the result and stop. |
| NEW-2 | `in_flight_source_for_key_is_current` loaded every ledger row to find one primary key. | Point query. |

The first regression test only cleared a classification, which is a row that
decodes and *then* fails invariant validation. That is a different bug from a
row that will not decode at all, and only the second one aborts the listing.
Both are now covered:
`an_unreadable_conversion_is_quarantined_instead_of_stopping_the_boot` and
`a_row_that_cannot_be_decoded_is_quarantined_instead_of_stopping_the_boot`.

### Two accepted decisions, recorded rather than fixed

- **A timed-out AnyDoc parse ends in a service restart, not in-process
  containment.** The full sequence, worst case: the parse hangs, its job fails
  `worker_timeout`, the detached thread keeps the only AnyDoc permit, the next
  AnyDoc job waits one more timeout, the runner returns `EnginePermitStalled`,
  and the process exits. **The restart is the containment** — the detached
  thread dies with the process, so the permit comes back fresh and startup
  recovery requeues the blocked job. That costs one restart, not a loop, and
  the blocked job's recovery budget bounds even a repeat: after
  `TOOLKIT_CONVERTER_RECOVERY_LIMIT` attempts it fails visibly as
  `recovery_limit_exceeded` rather than restarting forever.

  What this buys is that a hung parser cannot silently eat the queue. What it
  costs is a hard dependency on a supervisor: running the binary directly,
  without Compose or launchd, leaves the service down. Under `cpus: 2.0` the
  detached thread is also capped at two cores until the restart. Moving AnyDoc
  behind a killable child worker is the alternative; CVR-038's documented
  trigger for that is a field crash, which has not happened.
- **AnyDoc publishes with no completeness warning.** It cannot measure
  completeness and its part-level "skip a broken piece" recovery is silent.
  Warning on all 19 AnyDoc formats would attach a caveat to nearly every
  non-PDF conversion and train users to ignore warnings. `structured_document`
  claims a parser ran, not that the output is complete.

**Gate:** `pnpm verify:all` plus `pnpm verify:container`.

## Sprint 1: M4 routing and quality policy — landed

- **S1.1** `conversion/policy.rs`: one pure `decide(profile, route,
  LocalResult) -> PolicyDecision`. No IO, no clock, no engine type. CVR-042.
- **S1.2** Wired through `execute_claimed` and `finalize_success`, so reason
  codes and warnings reach the attempt row, the status response, and the
  manifest. `warnings: Vec::new()` is gone from every write site. CVR-044.
- **S1.3** `engines::QualitySignals`, engine-neutral and measurement-only. The
  PDF engine fills it from `Inspection`; AnyDoc reports `unmeasured()` because
  it genuinely cannot measure completeness. CVR-041.
- **S1.4** In-process generators in `tests/support/corpus.rs` plus a real
  `evals/corpus-manifest.yaml`, 17 cases. No binary PDF enters the repo, and
  every expectation came from running the real worker over the generated bytes.
  CVR-040.
- **S1.5** `tests/routing_policy.rs`: 8 tests over status, route, reason codes,
  warnings, failure code, artifact listing, downloaded bytes, and manifest.
  CVR-045.
- **S1.6** OpenAPI documents the vocabulary as an **open** list (description
  plus `x-vocabulary`), not an enum, because M5 adds remote reason codes and a
  closed enum would make that breaking. Two tests pin it to what the code can
  emit, in both directions.

No worker protocol change was needed: diagnostics reach the parent on the
`Converted` path too. A new `FallbackReason` would need an arm in the
exact-match coherence table at `pdf_inspector.rs:353`, so the new vocabulary
stays parent-side.

`best_quality` stays unavailable until Sprint 2. Rate calibration stays
deferred to M8.

### The correction adversarial review forced

The first cut read `confidence` as a completeness measure and routed remote on
it under `standard`. Three independent skeptics rejected that, and the worker
confirmed them: a ten-page report with a one-line cover page and **no images
anywhere** reports the same 0.9, and its Markdown contains every word. Routing
on that signal would have billed Datalab for the most ordinary shape of
business PDF there is. It fails the other way too — a page holding both text
and a full-page scan counts as a text page, so a document can report 1.0 with a
whole scan lost.

So the signal warns and never routes. `pages_without_extractable_text` says
exactly what is known: some pages produced no text, and the engine cannot say
whether they were blank or a scan. That kills the *silent* half of the silent
failure, which is what CVR-045 asks for, without spending money on a guess.
`a_sparse_cover_page_is_not_missing_content` is the regression that stops the
routing from coming back.

**The M4 follow-up worth doing:** getting a real routing decision back needs
evidence the engine does not report yet — pages that reference an image XObject
and yielded no text. That is a worker protocol change, not a policy change.

### Three traps the corpus surfaced

Worth knowing before anyone writes another PDF fixture.

- **Body text must not start with the word "Page".** pdf-inspector strips
  page-number-looking lines, the Markdown drops under 500 bytes, and its
  sparse-extraction branch then flags every page for OCR with no reason given.
  Every native fixture failed until the leading token became "Sheet".
- **`form_pdf` proves nothing about forms.** The identical page with the
  AcroForm, Fields, Annots and widget removed returns a byte-identical
  inspection. Its 0.5 confidence is the sparse-extraction branch.
- **`garbled_font_pdf` never reaches the garbled branch.** `hasEncodingIssues`
  stays false, so it lands on `ocr_required` like any sparse page.
  `FallbackReason::GarbledText` is unreachable in practice, and three of the
  seven engine reasons still have no corpus case.

**Gate:** 160 backend tests, 63 desktop Rust tests, 80 frontend tests, both
Clippy gates, the production build. Contract at 0.4.2.

## Sprint 2: M5 backend Datalab fallback

Three things settled before code:

- **S2.0 Runner shape.** One sequential loop awaits each job. A remote job
polls for minutes, so waiting inside that loop freezes the queue. Remote
waiting must be a persisted state a reconciler polls.
- **S2.1 State seam.** Extend `attempts`, no side table: artifacts and the
manifest are already keyed by (job, attempt) and CVR-056 wants the same
boundary. Add `RemoteQueued` and `ConvertingRemote` to both enums; put
acceptance on columns (`remote_request_id`, `remote_submitted_at`,
`remote_acceptance IN ('unknown','accepted')`), not states.
- **S2.2 Taxonomy gap.** "Port `providers.rs` verbatim" is not enough.
`parse_datalab_submit` returns `Result<Submitted, String>`, which cannot
express "maybe billed": a timeout after the body was sent and a hard 400 both
collapse to `Err(String)`. The adapter needs
`SubmitOutcome::{Accepted, Rejected, Uncertain}`, with `Uncertain` written to
the ledger before anything else.
- **S2.3 Attempt seam.** `interrupt_and_requeue` is the only way to mint a
second attempt, accepts only `ConvertingLocal`/`Finalizing`, and burns the
recovery budget. A remote leg needs `start_remote_attempt` from
`NeedsRemote` that leaves `recovery_count` alone.

Then CVR-050 through CVR-057 with a scripted mock Datalab: success, denial,
429, outage, timeout, malformed output, interrupted polling, uncertain
submission. `best_quality` becomes available when a credential is configured,
and `remoteFallback.available` starts telling the truth so the desktop stops
owning fallback.

**Gate:** full Rust suite plus the container smoke.

## Sprint 3: close M6

- **S3.1** CVR-067 durability scenario end to end. Sprint 0 proves the desktop
half.
- **S3.2** Provenance-aware history reuse. Backend results already carry
distinct `output_format` values, so the matcher can select by provenance
instead of the blanket disable.
- **S3.3** CVR-081 baseline. **Needs the owner: ~10 representative files and
Datalab credits.** Harness is buildable now, the run is not.



## Sprint 4: M7-lite

CVR-070 Caddy with internal HTTPS, CVR-071 rotatable per-device tokens,
CVR-073 host-local data layout, CVR-074 retention and disk-space readiness,
CVR-075 proven backup and restore. Rate-limit `/health/ready`, which takes a
SQLite write lock per unauthenticated request. CVR-076/077/078 stay deferred.

`auth_scope` is a compile-time constant pinned by a migration CHECK, so
per-device tokens need a migration.

**Needs the target host.**

## Sprint 5: M8 cutover

Freeze the corpus, run the acceptance suite and a soak, flip the desktop
default, exercise rollback, record evidence. CVR-086 cleanup stays behind its
own approval. **Needs the deployed host.**

## Order

Sprint 0 first: two of its defects cost money or a working service. Sprint 1
before Sprint 2, because M5 has no policy to act on until M4 defines one.
Sprint 3 after Sprint 2 for its one M5-dependent scenario. Sprints 4 and 5 stop
where they need a machine this session does not have.