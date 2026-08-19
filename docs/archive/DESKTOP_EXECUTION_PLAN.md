# Desktop execution plan (M6)

**Status:** implementation landed through `1f69cab`; milestone gate remains
open on the M5-dependent CVR-067 Datalab-fallback durability scenario and the
CVR-081 direct-path baseline.

This plan re-sequenced M6 ahead of M4 and M5. M3 is now complete, the desktop
schema matches OpenAPI 0.4.0, and CVR-060 through CVR-066 are implemented. M5
remains a dependency for exactly one CVR-067 end-to-end scenario, not for the
desktop's local conversion path.

## Implementation outcome

- `service_request` and the path-returning Markdown download command are
  registered native handlers. The host owns the backend URL, token, multipart
  stream, response bounds, response-ID validation, and artifact bytes.
- Routing is per file from live `capabilities.inputFormats`. The only permanent
  direct set is png/jpg/jpeg/webp/tiff/tif/gif/bmp/html/htm. Direct-eligible
  desktop formats absent from live backend capabilities may use the existing
  direct Datalab path under `standard`; `local_only` rejects them without
  external transmission. Arbitrary files such as mp4/exe never enter Convert.
- `standard` and `local_only` use the same local backend engines today, but the
  desktop decision after terminal `needs_remote` differs: `standard` continues
  through the existing Datalab lifecycle, while `local_only` fails with an
  actionable message. `best_quality` remains unavailable.
- Direct Datalab is still the default and the backend route is reversible.
  Backend-mode conversion history reuse/copy is deliberately disabled because
  direct and backend provenance are not interchangeable in the matcher yet.

## Settled command surface

1. `POST /api/v1/conversions` is a native special case. It reads the desktop
   path from the JSON `source` field and streams multipart without placing file
   bytes or the bearer token in the webview.
2. Markdown download has its own native command. It streams up to the host
   limit into a collision-safe output path and returns only that path over IPC.

## Durable replay and recovery

`history.rs` owns an in-flight table beside terminal history. Each row keeps the
canonical source, source mtime, output directory, stable client run UUID,
stable per-file idempotency key, original backend URL, profile, and optional
backend job UUID. The desktop persists before submit, attaches the accepted ID,
and recovers against the recorded origin after restart. Storage remains
best-effort for an otherwise valid job.

## Increments

### Increment 0: Settle the transport shape

- [x] Decide the multipart branch and the download command, per the two problems
  above. Write the decision into [`STATUS.md`](../STATUS.md) before writing code.
- [x] Regenerate the client with `pnpm generate:api`; the committed schema now
  matches backend OpenAPI 0.4.0.

Exit: the IPC surface is decided and the schema matches the contract.

### Increment 1: The reversible switch (CVR-066)

Build the switch before the thing it switches to. With the flag in from day one
every later step is shippable and reversible, and the direct path is there to
catch `needs_remote`.

- [x] Add the route setting and branch `JobType::Convert` on it in
  `src-tauri/src/jobs.rs`. Default to the direct path.
- [x] Make the branch per file, not per run, so `needs_remote` can fall through.

Exit: the setting exists, defaults to today's behavior, and changes nothing yet.

### Increment 2: Settings and credentials (CVR-060)

- [x] Add `backend_url` and `conversion_profile` to `Settings`, its frontend
  mirror, and `DEFAULT_SETTINGS`. `#[serde(default)]` on the struct already
  covers the migration.
- [x] Add a third Keychain account for the device token. `secrets.rs` is generic
  over account name, but `SecretId`, `SecretStatus`, the `set_secret` match, and
  `ProviderKind::key_name` all name exactly two, so a third touches four places.
- [x] Add the fields to the settings panel.

The wire shape stays `Authorization: Bearer` when M7 replaces the bootstrap
token with per-device credentials, so nothing built here gets thrown away.

Exit: a backend URL and token round-trip through settings and the Keychain, and
no key reaches the webview.

### Increment 3: Submit (CVR-062)

The biggest piece and the only one with real unknowns.

- [x] Implement the `service_request` command and register it.
- [x] Implement the multipart branch that streams the source from disk.
- [x] Generate and persist a stable `clientRunId` per run and a durable
  idempotency key per file.
- [x] Probe `/api/v1/capabilities` before a run so a misconfigured URL disables
  the Run button instead of surfacing as a failed job.

Exit: a PDF submits, returns `202`, and a resubmit of the same file replays
rather than converting twice.

### Increment 4: Poll and download (CVR-063, CVR-064)

Map the durable `status` onto the existing `Job` + `progressNote`. Do not add
SSE, a percent, or a new OpenAPI field; see
[`MONITORING_AND_PROGRESS.md`](MONITORING_AND_PROGRESS.md).

- [x] Poll `GET /api/v1/conversions/{id}` on the existing 5s loop.
- [x] Recover an in-progress run after an app restart from the durable key store.
- [x] Stream Markdown to a collision-safe disk path through the dedicated
  native download helper; no artifact body crosses IPC.
- [x] Write the terminal-state check as an explicit allowlist of finished
  states, never an exhaustive match on today's six values. M5 grows that set.

Exit: drop a PDF, get markdown on disk through the backend, and an app restart
mid-run picks the job back up.

### Increment 5: Surface the decisions (CVR-065)

- [x] Render `route.kind`, `route.reasonCodes`, `warnings[]`, and
  `failure{code,message}` data-driven, so M4 adds codes without touching the
  view.
- [x] Show `needs_remote` as a fall-through to the direct path, not a dead end.

Exit: a user can tell why a file routed the way it did.

### Increment 6: Tests (CVR-067)

- [x] Local success, restart recovery, backend unavailable, retry-safe replay,
  future status/route values, Stop/retry cleanup, and path-only artifact IPC.
- [ ] Run the M5-dependent end-to-end durability scenario for backend-owned
  Datalab fallback.
- [ ] Capture the CVR-081 representative direct-path baseline before cutover.

Exit: not met until the two unchecked evidence items above are complete.

## M3 coordination outcome

M3 completed first at `4590a9a`; its later `322d96e` route-kind contract fix is
also consumed. M6 then regenerated `src/app/api/schema.ts` and kept the desktop
planner dynamic, so further additive `inputFormats` changes require no desktop
format list edit.

## Recommended order from here

1. Capture CVR-081's representative direct-path baseline.
2. Complete M4's fixture-backed routing policy.
3. Implement M5 and run the deferred CVR-067 backend-owned Datalab fallback
   durability scenario.
4. Close the M6 gate only when both pending evidence items are recorded; then
   proceed to the M7 deployment subset and M8 cutover.

## Verify

```bash
pnpm check      # tsc --noEmit + eslint + vitest
pnpm verify     # check + build + src-tauri clippy/tests
```

The final shared-tree M6 landing gate passed 53 frontend tests and 60 desktop
Rust tests with 3 live smokes ignored, plus the production build and strict
Clippy. The Rust total includes 3 concurrent, unstaged `secrets.rs` tests; it is
not an exact-commit count for `1f69cab`.

Desktop clippy and tests run on `macos-latest` in CI because of
`macos-private-api` and `keyring`. The desktop does not render in a plain
browser: use `pnpm tauri dev`, or `?gallery` for a bridge-free design review.
