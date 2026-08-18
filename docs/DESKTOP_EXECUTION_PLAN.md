# Desktop execution plan (M6)

**Status:** not started. Blocked only on the M2 release gates in
[`BACKEND_EXECUTION_PLAN.md`](BACKEND_EXECUTION_PLAN.md) Increment 6.

This plan re-sequences M6 ahead of M3, M4, and M5. The epic
([`BACKEND_EPIC.md`](BACKEND_EPIC.md)) lists the milestones in numeric order and
marks M6 as depending on M5. That dependency is real for exactly one test
scenario. Everything else in M6 needs only the M2 contract, which is frozen at
OpenAPI `0.3.0` and under test.

## Why M6 comes next

Going through CVR-060 to CVR-067 against the live code, **none of the eight
tickets is blocked by M3 or M4, and only one sub-scenario of CVR-067 needs M5.**

M3 changes which files the backend accepts. M4 changes which reason codes come
back. M5 changes what happens after `needs_remote`. None of the three changes a
path, a status field, an envelope, an auth scheme, or a header. A desktop client
written against today's contract keeps working through all three, as long as it
reads `inputFormats` from `/api/v1/capabilities` instead of hardcoding
`application/pdf`.

Three more reasons to go now rather than later:

- **The frontend half is already built and currently dead.** `src/app/api/`
  landed as authorized pre-M6 prep, and `commands.serviceRequest` invokes a
  Tauri command named `service_request` that is not in the `generate_handler!`
  list in `src-tauri/src/lib.rs`. Every call through that layer fails with an
  unknown-command error today. The longer it sits unused, the longer it goes
  unverified.
- **A real consumer is the only honest contract test.** Changing the contract is
  cheap now and expensive after M3 widens formats and M5 adds remote states.
- **The desktop loses nothing by starting.** CVR-066 keeps the direct Datalab
  path behind a switch, so the backend route can be wrong for weeks without
  costing a conversion.

**What to reject:** running M6 behind M3, M4, and M5 as the epic's dependency
column implies. That buys three milestones of sequencing for one test scenario.

## The backend is narrower than the app today

This is the fact that shapes the whole plan.

| | Desktop direct path | Backend today |
|---|---|---|
| Input formats | 18 extensions, PDF through EPUB | `application/pdf` only |
| Output formats | markdown, html, json, pipeline variants | markdown plus a provenance manifest |
| Scanned or table-heavy | Handled. High accuracy is **on by default** | Terminal `needs_remote`, nothing published |

So the backend route must not be all-or-nothing. **Make CVR-066's switch
per-file, not global**, and let a `needs_remote` answer fall through to
`providers::datalab_submit`. That is a small change in `src-tauri/src/jobs.rs`
and it defers M5 indefinitely without the user losing anything.

Note that `standard` and `local_only` are behaviourally identical right now.
Both run the same local PDF path and both end `needs_remote` on a non-clean PDF.
The profile is persisted and folded into the idempotency fingerprint but changes
no routing. M4 is what gives `standard` a distinct meaning. Until then the
desktop sends `standard` and treats the two as one setting. Never offer
`best_quality`: it is a hard `409 profile_unavailable`.

## Two contract problems to settle before writing code

Both sit in the transport shape that pre-M6 prep declared. Settle them first,
because both change the command surface.

1. **Multipart cannot go through the declared door.** `ServiceRequestPayload.body`
   is `string | undefined` and `hostFetch` throws on a non-string body. The
   webview never holds the file bytes. So the host has to special-case
   `POST /api/v1/conversions`, read the desktop path out of the `source` field,
   and build the multipart itself. `transport.ts` documents that branch and it
   does not exist.
2. **The markdown download must not go through that door at all.**
   `ServiceResponsePayload.body` is a `String`, so an artifact up to
   `TOOLKIT_CONVERTER_MAX_OUTPUT_BYTES` (50 MB by default) would cross IPC into
   the webview and back out. CVR-064 says download through Tauri, meaning the
   host streams it to disk. Give the download its own command that returns a
   path, not a body.

## The one genuinely new design problem

**A durable per-file idempotency key.** The key only earns its keep if it
survives a desktop restart, and surviving a restart is exactly what makes
CVR-063's replay path work. The desktop has nowhere to put it today:
`JobManager` is an in-memory `Mutex<Vec<Job>>`, and `history.rs` records only
terminal results keyed by `source_path`.

`history.rs` already owns all SQL and already answers "already done?" by source
path on every scan, so an in-flight table beside the results table is the
natural home. Keep the rule that a storage error never fails a job.

## Increments

### Increment 0: Settle the transport shape

- [ ] Decide the multipart branch and the download command, per the two problems
  above. Write the decision into `docs/HANDOFF.md` before writing code.
- [ ] Regenerate the client: `pnpm generate:api`. The committed
  `src/app/api/schema.ts` trails `backend/openapi/openapi.yaml` by the whole
  CVR-027 delta, so do this before anything types against the schema or someone
  codes against `maxEphemeralJobs`.

Exit: the IPC surface is decided and the schema matches the contract.

### Increment 1: The reversible switch (CVR-066)

Build the switch before the thing it switches to. With the flag in from day one
every later step is shippable and reversible, and the direct path is there to
catch `needs_remote`.

- [ ] Add the route setting and branch `JobType::Convert` on it in
  `src-tauri/src/jobs.rs`. Default to the direct path.
- [ ] Make the branch per file, not per run, so `needs_remote` can fall through.

Exit: the setting exists, defaults to today's behavior, and changes nothing yet.

### Increment 2: Settings and credentials (CVR-060)

- [ ] Add `backend_url` and `conversion_profile` to `Settings`, its frontend
  mirror, and `DEFAULT_SETTINGS`. `#[serde(default)]` on the struct already
  covers the migration.
- [ ] Add a third Keychain account for the device token. `secrets.rs` is generic
  over account name, but `SecretId`, `SecretStatus`, the `set_secret` match, and
  `ProviderKind::key_name` all name exactly two, so a third touches four places.
- [ ] Add the fields to the settings panel.

The wire shape stays `Authorization: Bearer` when M7 replaces the bootstrap
token with per-device credentials, so nothing built here gets thrown away.

Exit: a backend URL and token round-trip through settings and the Keychain, and
no key reaches the webview.

### Increment 3: Submit (CVR-062)

The biggest piece and the only one with real unknowns.

- [ ] Implement the `service_request` command and register it.
- [ ] Implement the multipart branch that streams the source from disk.
- [ ] Generate and persist a stable `clientRunId` per run and a durable
  idempotency key per file. `uuid` is not yet a `src-tauri` dependency.
- [ ] Probe `/api/v1/capabilities` before a run so a misconfigured URL disables
  the Run button instead of surfacing as a failed job.

Exit: a PDF submits, returns `202`, and a resubmit of the same file replays
rather than converting twice.

### Increment 4: Poll and download (CVR-063, CVR-064)

Map the durable `status` onto the existing `Job` + `progressNote`. Do not add
SSE, a percent, or a new OpenAPI field; see
[`MONITORING_AND_PROGRESS.md`](MONITORING_AND_PROGRESS.md).

- [ ] Poll `GET /api/v1/conversions/{id}` on the existing 5s loop.
- [ ] Recover an in-progress run after an app restart from the durable key store.
- [ ] Stream the markdown to disk through `write_output`, unchanged. Its
  collision-safe naming already does what CVR-064 asks for.
- [ ] Write the terminal-state check as an explicit allowlist of finished
  states, never an exhaustive match on today's six values. M5 grows that set.

Exit: drop a PDF, get markdown on disk through the backend, and an app restart
mid-run picks the job back up.

### Increment 5: Surface the decisions (CVR-065)

- [ ] Render `route.kind`, `route.reasonCodes`, `warnings[]`, and
  `failure{code,message}` data-driven, so M4 adds codes without touching the
  view.
- [ ] Show `needs_remote` as a fall-through to the direct path, not a dead end.

Exit: a user can tell why a file routed the way it did.

### Increment 6: Tests (CVR-067)

- [ ] Local success, restart recovery, backend unavailable, retry-safe replay.
- [ ] Datalab fallback is the one scenario that waits for M5.

Exit: the M6 gate, minus the deferred scenario.

## Running M3 in parallel

M3 and M6 can be two sessions from day one. The ownership table in
[`HANDOFF.md`](HANDOFF.md) already separates them: backend gets `backend/**`,
desktop gets `src/**` and `src-tauri/**`.

Fully independent, touching no desktop file: CVR-030, CVR-031, CVR-032, CVR-035,
CVR-036, CVR-037.

One coordination point, and it is mechanical: CVR-033 widens `inputFormats` in
`backend/openapi/openapi.yaml`, which changes the generated schema. That is not
a merge conflict, it is one `pnpm generate:api` run by whoever lands second.
**The M3 session never writes `src/app/api/schema.ts`**, the same rule Increment
5 followed when it diffed the CVR-027 delta under `/private/tmp` only.

## Recommended order after M2

1. **M6**, in the increment order above, which is not ticket-number order.
2. **M3 in parallel**, in a second session.
3. **M4 after M3.** Genuinely blocked: routing policy needs a second engine and
   the labeled corpus before CVR-043 can calibrate it.
4. **M5 last, and treat it as optional.** Once the switch routes `needs_remote`
   back to the direct path, M5 buys consolidation rather than capability: one
   credential, one audit trail, one policy boundary. Worth doing. Not worth
   doing first.

## Verify

```bash
pnpm check      # tsc --noEmit + eslint + vitest
pnpm verify     # check + build + src-tauri clippy/tests
```

Desktop clippy and tests run on `macos-latest` in CI because of
`macos-private-api` and `keyring`. The desktop does not render in a plain
browser: use `pnpm tauri dev`, or `?gallery` for a bridge-free design review.
