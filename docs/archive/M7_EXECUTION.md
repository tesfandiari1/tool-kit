# M7 execution plan

**Cancelled 2026-08-19, unstarted.** M7 existed to make the converter reachable
from anywhere over a private tailnet. Adopting [`north-star.md`](../north-star.md)
removed the reason: the product is one local workspace on one Mac, and the app
now runs three engines the container cannot host at all (Apple Vision, and MLX
Whisper and mlx-lm to come). Remote reach to a service that does less than the
app is not worth hardening. Nothing here shipped. Kept as the record of what was
planned and why it stopped.

---

The sprint-level plan for M7 remote deployment. [`STATUS.md`](../STATUS.md) section 4
holds the decisions and the ticket set. This file holds the order of work, the
file-level steps, and the verification for each one. It is live for the duration
of M7 and moves to [`archive/`](README.md) when the M7 gate closes.

**Created:** 2026-08-19

Every claim about the tree below was checked against the tree. Where a step names
a file, a function, or a line, it exists. Read the file before you change it
anyway: this document goes stale the moment someone lands a commit.

Docs use **standard** STE voice: American spelling, active voice, no em dashes,
no semicolons between independent sentences. Use a colon in section titles.
Keep one name for one thing. Do not rewrite code, commands, identifiers, or
quotations to match prose rules.

---

## 1. The line

CVR-075 goes first, exactly as `docs/STATUS.md` section 4 already sequences it, and nothing touches the host until a restore has been asserted. Position 1 wins the ordering argument: remote access is the point of M7, and the ticket set past CVR-075 and CVR-070 is guessing until the box has run the image. Position 3 wins one point outright, the five-byte readiness probe, because `write_health_probe` in `src/artifacts.rs` writes `b"ready"` and calls `flush()` rather than fsync, so readiness stays green across the whole band where a 25 MB upload can no longer land. Position 2 loses its headline: cargo-chef and GHCR are build ergonomics whose rollback value is zero without CVR-075, and the recorded M7 gate does not name them, so CVR-079 leaves M7 with a trigger. Position 1 loses two specifics the cross-examiner refuted, and neither survives into this plan: CVR-071 is not a thin slice (it is the largest item in M7), and `ss -ltnp` in the host netns proves nothing once the converter lives in the sidecar's namespace.

Two corrections everyone missed are now hard steps. Tailscale goes on the TrueNAS host itself before the published port disappears, because `tailscale serve` in a container proxies one HTTP port and does not give a shell. And STATUS gate 4, the boot config line, is unbuilt in both halves and lands in sprint 3, because once the port is gone that line is the only way to confirm which configuration a container started with.

## 2. Hard dependencies

1. **Host tailnet before port removal.** Install Tailscale on the TrueNAS host (or enable Tailscale SSH) and prove a shell over it before the converter's `ports:` mapping goes away. A crash-looping converter never binds a listener, because `src/main.rs` calls `AppState::initialize` (migrations plus `StartupRecovery`) before `TcpListener::bind`, and the sidecar answers with a proxy error. Recovery is a host shell or nothing.
2. **Proven restore before the first delete.** Retention in sprint 7 writes the repo's first `DELETE FROM` and the first unlink under `quarantine/`. `grep 'DELETE FROM' backend/src/` returns nothing today.
3. **Tailscale lands in an overlay file, not in `backend/compose.yaml`.** `backend/scripts/container-smoke.sh:163` sets `BASE_ARGS=(-f "${BACKEND_DIR}/compose.yaml")` and its generated overlay declares only `image:` and `restart:`, so it inherits the base `ports:` publish on 18080. A base-file `network_mode: service:tailscale` breaks `pnpm verify:container` the same day.
4. **DB-first ordering settles before the restore assertion is written.** DB first means a job created during the backup window has bytes and no row, and `StartupRecovery::quarantine_orphans` (`src/jobs/recovery.rs:109`) renames that storage into `quarantine/preacceptance/` on the next boot. A single-succeeded-job assertion cannot see that, so the drill asserts the quarantine explicitly.
5. **Retention and storage deletion land in the same change.** Delete rows without unlinking storage and `quarantine_orphans` converts a disk problem into a slower disk problem with an extra copy, because `ArtifactStore::quarantine_preacceptance` (`src/artifacts.rs:657`) is an `fs::rename` and nothing ever unlinks that tree.
6. **The idempotency question resolves before migration 0004 is written.** STATUS line 473 records it as an open correctness question, not a detail. Section 4 below resolves it.
7. **The worker-lifecycle scope decision precedes the migration too.** Nine of the fifteen `AUTH_SCOPE` binds in `src/persistence/sqlite.rs` are writes from a job runner that has no caller identity.
8. **`openapi/openapi.yaml` `info.version` moves with `Cargo.toml` version.** `tests/http_contract.rs:2098` asserts they match, so a release tag needs both edits in one commit.
9. **A bootstrap-token transition policy precedes any per-device scheme.** The shipped signed DMG holds a bootstrap token and its only recovery is a human pasting a new one into Settings.

## 3.1 Sprint 1: backup and proven restore (CVR-075)

**Goal.** A consistent SQLite snapshot the running service can take, and a container-level drill that destroys the volume and brings the data back. Nothing else in M7 starts until this passes.

**Steps.**

1. **`backend/src/persistence/sqlite.rs`**: add `pub async fn snapshot_into(data_dir: &Path, destination: &Path) -> Result<(), RepositoryError>`. It opens a plain `SqliteConnection` on `data_dir.join(DATABASE_FILENAME)` with `.foreign_keys(true)`, does **not** run `MIGRATOR`, and executes `VACUUM INTO ?1` with the destination bound as a parameter. Persistence owns all SQL, so this function is the only place a backup touches the database. `VACUUM INTO` cannot run inside a transaction, so no `begin()`. (S)
2. **`backend/src/bin/tool-kit-backup.rs`**: new binary beside `tool-kit-pdf-worker.rs`. Args: `--data-dir` (default `/data`), `--out` (default `<data-dir>/backup/<UTC timestamp>`). It creates the out directory, calls `snapshot_into` to write `converter.sqlite`, then writes `backup.json` carrying `serviceVersion` from `env!("CARGO_PKG_VERSION")`, the `_sqlx_migrations` high-water version, the UTC time, and the snapshot's sha256. It writes the artifact tree not at all. The destination has to sit under `/data`, because the runtime rootfs is `read_only` and `/tmp` is a 384m noexec tmpfs. No `unwrap` anywhere, per the crate deny-set. (M)
3. **`backend/Dockerfile`**: one more `COPY --from=builder /build/target/release/tool-kit-backup /usr/local/bin/tool-kit-backup`. `cargo build --locked --release --bins` already builds it, so the builder stage is untouched. (S)
4. **`backend/src/persistence/sqlite.rs` tests module**: add `snapshot_captures_wal_commits_a_file_copy_would_drop`: open a temp repository, insert rows through the existing helpers so the WAL is non-empty, `snapshot_into` a sibling path, reopen the copy through `SqliteRepository::open` (which needs the file named `converter.sqlite` in an absolute directory), and assert the conversion count matches. A `cp converter.sqlite` fails this test, which is the point. (S)
5. **`backend/scripts/container-smoke.sh`**: extend, do not write a new script. Add `restore` to the `--phase` case at line 60 and to the usage comment at line 14. The phase reuses `submit`, `poll_until`, `wait_ready`, `verify_bundle`, `verify_artifact`, `assert_eq`, `assert_json`, and `assert_smoke_volume` unchanged. Sequence:
   - graceful submit of the pinned 668-byte fixture, poll to `succeeded`, `verify_bundle`, record the manifest `.output.sha256`.
   - start a second job against the hold-worker overlay (`HOLD` array, already defined at line 165) so one job sits in `converting_local` with source bytes on the volume and, after the DB snapshot, no `succeeded` row.
   - `compose exec -T converter /usr/local/bin/tool-kit-backup --data-dir /data --out /data/backup/drill`.
   - `compose stop -t 45 converter`, then `docker run --rm -v ${PROJECT}_converter-data:/data alpine tar -C /data -cf - jobs quarantine backup > "${RUN_DIR}/artifacts.tar"`. DB first, artifacts second.
   - `compose "${SMOKE[@]}" down -v` behind `assert_smoke_volume`, then `up -d --no-start`, untar into the fresh volume, move `backup/drill/converter.sqlite` to `/data/converter.sqlite` with no `-wal` or `-shm` alongside, `chown 10001:10001`, `up -d --wait`.
   - `wait_ready 120`, then re-run `verify_bundle` on the first job id. (L)
6. **Same script**: assert the DB-first hazard rather than hiding it: after the restore boot, `assert_eq` that `GET /api/v1/conversions/<held job id>` returns 404, and that `docker run --rm -v ...:/data alpine sh -c 'ls /data/quarantine/preacceptance | wc -l'` is `1`. Reversing the backup order makes this assertion fail. (S)

**Verify.** Run `backend/scripts/container-smoke.sh --phase restore` and observe exit 0 with `backend/target/container-smoke/<UTC>/evidence.md` carrying, after the restore: `markdown bytes match the ledger` equal to the pre-backup `.output.sha256`, `markdown etag` equal to `"sha256-<that sha>"`, `artifact count` 2, the held job at 404, and one directory under `quarantine/preacceptance`. Then run `cargo test --locked --manifest-path backend/Cargo.toml` and observe the new persistence test pass.

**Commit boundary.** One commit: the binary, the persistence function, the Dockerfile COPY, the unit test, and the smoke phase. Base compose is untouched, so `pnpm verify:container` stays green.

**Contract.** None.

## 3.2 Sprint 2: host bring-up on loopback (CVR-070 part 1, CVR-073 as a recorded fact)

**Goal.** The image runs on the real box, the restore drill passes there, and a shell over the tailnet exists before any port disappears. No Rust changes.

**Steps.**

1. **TrueNAS host**: install Tailscale on the host itself and disable node key expiry for that node. This is dependency 1 and it is not optional. (S)
2. **Host**: clone the repo, `docker compose -f backend/compose.yaml up -d --wait` with `backend/compose.yaml` exactly as it stands, still publishing `127.0.0.1:8080`. The host is amd64, so there is no cross-build and no QEMU. (M)
3. **Host**: re-run `backend/scripts/container-smoke.sh --phase restore` against the host's filesystem. A restore proven inside Docker Desktop's Linux VM is not a restore proven on the NAS. (S)
4. **Desktop**: from the Mac, `ssh -L 8080:127.0.0.1:8080 <host>` over the tailnet, point the app's backend URL setting at `http://127.0.0.1:8080`, and convert one real PDF end to end. That exercises `hostFetch` → `service_request` → `classify_route` → converter before any namespace change can confuse the diagnosis. (S)
5. **`backend/README.md`**: one deploy section: the commands above, the tailnet host name, and the evidence path from step 3. (S)

**Verify.** On the host, `curl --fail -s http://127.0.0.1:8080/health/ready | jq '.checks'` prints `database`, `dataRoot`, and `worker` all `"ok"`. `stat -f -c %T "$(docker volume inspect ${PROJECT}_converter-data -f '{{.Mountpoint}}')"` prints a local type and never `nfs` or `smb2`. From a tethered laptop off the LAN, `ssh <host>` over the tailnet gets a shell.

**Commit boundary.** One docs commit. No code moved.

## 3.3 Sprint 3: observability and the filesystem guard (CVR-074 metrics, CVR-073, STATUS gate 4)

**Goal.** The three cheap things that have no contract cascade: a boot config line, a periodic queue stats line, and a hard refusal to open the database on SMB or NFS. All three land before exposure, because after exposure logs are the only channel.

**Steps.**

1. **`backend/Cargo.toml`**: promote `rustix` to a direct dependency with `features = ["fs"], default-features = false`. It is already in `backend/Cargo.lock` at 1.1.4 through `tempfile`, and its license is on the `deny.toml` allow list, so the crate graph does not grow and `pnpm verify:deps` has nothing new to grade. This is the answer to `unsafe_code = "forbid"` in `backend/Cargo.toml` lines 57-58, which `forbid` makes un-overridable by an inner `#[allow]`. Position 3's claim that libc alone suffices was wrong and does not carry forward. (S)
2. **`backend/src/artifacts.rs`**: add `pub fn free_bytes(&self) -> Result<u64, ArtifactError>` using `rustix::fs::statvfs` on `self.root`, plus a new `ArtifactError::FilesystemStat` variant. Gate the call `#[cfg(target_os = "linux")]` and return `Ok(u64::MAX)` elsewhere, so native macOS dev keeps working. (S)
3. **`backend/src/persistence/sqlite.rs`**: in `SqliteRepository::open`, after the existing absolute-path and `is_dir` checks, call `rustix::fs::statfs` on the data dir under `#[cfg(target_os = "linux")]` and reject `NFS_SUPER_MAGIC` (0x6969), `SMB_SUPER_MAGIC` (0x517b), `CIFS_MAGIC_NUMBER` (0xff534d42), and `SMB2_MAGIC_NUMBER` (0xfe534d42) with a new `RepositoryError::NetworkFilesystem { path, magic }`. Deny-list only: an unrecognized magic passes, so an unusual local filesystem cannot brick startup. `StartupError` in `backend/src/app.rs` already surfaces `RepositoryError`. (M)
4. **`backend/src/persistence/sqlite.rs`**: add `pub async fn queue_snapshot(&self) -> Result<QueueSnapshot, RepositoryError>`, one `SELECT status, COUNT(*) FROM conversions GROUP BY status`, decoded into a struct with a field per status. This is the first `GROUP BY` in the crate and sprint 6 reuses it. Leave `accepting_jobs` alone. (M)
5. **`backend/src/jobs/mod.rs`**: emit the stats line at the **top of the `loop` in `run`**, before `stop_aware_claim`, gated on a `last_emitted: Instant` so it fires at most once a minute. Do **not** put it in the `ClaimDecision::Idle` arm at lines 273-284: that arm is reached only when the claim returns `Idle`, so a queue stuck busy would go silent in exactly the failure mode the line exists to catch. Fields: `queued`, `converting_local`, `finalizing`, `failed`, `free_bytes`, `seconds_since_last_success`. (M)
6. **`backend/src/main.rs`**: extend the existing `tracing::info!` at lines 20-24 into the boot config line STATUS gate 4 needs: `data_dir`, `max_jobs`, `max_upload_bytes`, `max_output_bytes`, `worker_poll_interval_secs`, `recovery_limit`, `shutdown_grace_secs`, alongside the existing `bind_address` and `service_version`. Never the token path contents. (S)
7. **`backend/compose.yaml`**: add `env_file: - .env` beside the existing `environment:` block and a comment recording the trap already written in STATUS: `env_file:` does not feed Compose interpolation, only the project `.env` does. (S)
8. **`backend/tests/failure_modes.rs`**: extend with `data_root_on_a_network_filesystem_refuses_to_open`, `#[cfg(target_os = "linux")]` and `#[ignore]`d, documenting the manual mount it needs. The suite already owns `migration_failure_prevents_startup`, so this is the right file. (S)

**Verify.** Run `docker compose -f backend/compose.yaml up -d --wait && docker compose -f backend/compose.yaml logs converter | jq -r 'select(.fields.message=="conversion service listening")'` and observe `max_jobs`, `data_dir`, and `shutdown_grace_secs` in the line. Then submit two jobs and run `docker compose -f backend/compose.yaml logs converter | jq -r 'select(.fields.message=="queue stats")' | wc -l` and observe at most one line per minute with a `free_bytes` that tracks `df` on the volume. Then set `TOOLKIT_CONVERTER_MAX_JOBS: 2` in `.env`, `docker compose up -d`, and observe the new value in the boot line. Then `cargo clippy --locked --manifest-path backend/Cargo.toml --all-targets -- -D warnings` and `pnpm verify:deps` both clean.

**Commit boundary.** One commit. No contract file changed, so the frontend job is untouched.

**Contract.** None.

## 3.4 Sprint 4: Tailscale sidecar and the reachability proof (CVR-070, CVR-072)

**Goal.** The converter answers only through the sidecar, from anywhere, and `pnpm verify:container` still passes.

**Steps.**

1. **`backend/compose.tailscale.yaml`**: new deploy-only overlay, never merged into the base. It adds a `tailscale` service (`tailscale/tailscale`, pinned tag) with `TS_USERSPACE=true` so it needs neither `NET_ADMIN` nor `/dev/net/tun`, `TS_STATE_DIR=/var/lib/tailscale` on a new named volume `tailscale-state`, `TS_AUTHKEY` from a Compose secret, and `TS_SERVE_CONFIG=/config/serve.json` mounted read-only, proxying 443 to `http://127.0.0.1:8080`. It sets `network_mode: service:tailscale` on `converter`, `ports: !reset []`, `depends_on: tailscale`, and `TOOLKIT_CONVERTER_BIND_ADDR: 127.0.0.1:8080` written out explicitly. Compose merge cannot delete a map key by omission, so the base override of `0.0.0.0:8080` has to be replaced by value, not dropped. Note the inversion in a comment: under a shared namespace the Dockerfile ENV default becomes the correct value and the base compose override becomes the wrong one. (L)
2. **`backend/compose.tailscale.yaml`**: record two traps as comments and in the README: recreating the `tailscale` container destroys the namespace, so the converter must be recreated in the same command or its restart policy loops on "cannot join network of a non-running container"; and the tailnet node needs key expiry disabled in the admin console or the service goes dark on a timer with no code change to blame. (S)
3. **`backend/compose.dozzle.yaml`**: separate overlay, Dozzle 9.0.3 or newer per the CVE noted in STATUS, read-only Docker socket, published to the tailnet address only. It never touches the converter service, which keeps `cap_drop: ALL`, `no-new-privileges`, and the read-only rootfs intact. (S)
4. **`backend/scripts/container-smoke.sh`**: no change. It loads the base file only, and the base file did not move. Confirming that is part of the verify. (S)
5. **`backend/README.md`**: the deploy command becomes `docker compose -f backend/compose.yaml -f backend/compose.tailscale.yaml -f backend/compose.dozzle.yaml up -d --wait`, with the recreate-both rule spelled out. (S)

**Verify.** From a phone on cellular over the tailnet, `curl --fail https://<host>.<tailnet>.ts.net/health/live` returns 200, and an authenticated `GET /api/v1/capabilities` returns 200. On the host, `docker inspect -f '{{.HostConfig.NetworkMode}} {{json .NetworkSettings.Ports}}' <converter id>` prints `container:<tailscale id>` and `{}`. From a LAN machine that is **not** on the tailnet, `curl --max-time 5 http://<lan-ip>:8080/health/live` fails to connect, and `docker run --rm --network bridge alpine sh -c 'nc -z -w2 <converter-ip> 8080'` fails from another container. Do not use `ss -ltnp` in the host namespace: the listener lives in the sidecar's namespace and the check would pass trivially. Then run `pnpm verify:container` on the dev Mac and observe exit 0, proving the overlay did not reach the base file. Finally, in Dozzle over the tailnet, the `queue stats` line from sprint 3 appears within one minute.

**Commit boundary.** One commit: two overlay files plus README. Base compose unchanged.

**Contract.** None.

## 3.5 Sprint 5: disk-space readiness (CVR-074): touches the OpenAPI contract

**Goal.** Close the false green. `write_health_probe` writes five bytes and calls `flush()` rather than fsync, so a data root with 8 KB free reports `dataRoot: ok` while a 25 MB upload fails, the compose healthcheck believes it, and `restart: unless-stopped` never fires. STATUS's own CVR-074 note already says to read free space with statvfs rather than trusting the write probe.

**Steps.**

1. **`backend/src/config.rs`**: add `TOOLKIT_CONVERTER_MIN_FREE_BYTES` to the fail-fast parser with a bound, default 1 GiB, as a `min_free_bytes: u64` field on `Settings`. (S)
2. **`backend/tests/support/mod.rs`**: add the field to the `Settings` struct literal at lines 407-428. `Settings` has no `Default` impl, so the harness will not compile until this lands in the same commit. (S)
3. **`backend/src/api/health.rs`**: add `storage: &'static str` to `ReadinessChecks` and a fourth arm to the existing `tokio::join!` at line 49, wrapped in the same `bounded()` helper. The probe calls `ArtifactStore::free_bytes` from sprint 3 inside `spawn_blocking`, so the 2 second `CHECK_TIMEOUT` still bounds it, and fails when free bytes fall below `min_free_bytes`. The reason goes to the log, not the body, matching the existing unauthenticated-endpoint rule. (M)
4. **`backend/openapi/openapi.yaml`**: **contract change.** Add `storage` to `ReadinessChecks`, whose required list becomes `["database","dataRoot","storage","worker"]` with `additionalProperties: false` kept. (S)
5. **`backend/tests/http_contract.rs`**: extend `openapi_parses_and_documents_only_the_live_routes` at line 2087. `paths.len()` stays 8. The `ReadinessChecks` required list assertion at ~2207 becomes four names. Do not add a new test. (S)

**Verify.** Boot with `TOOLKIT_CONVERTER_MIN_FREE_BYTES` set to twice `df --output=avail -B1 /data`, then `curl -s -o /tmp/r.json -w '%{http_code}' localhost:8080/health/ready` prints 503 and `jq '.checks' /tmp/r.json` shows `storage: "failed"` with the other three `"ok"`. Then run `cargo test --locked --manifest-path backend/Cargo.toml --test http_contract`, `pnpm lint:api`, `pnpm verify:api-drift`, and `backend/scripts/contract-fuzz.sh` and observe all four clean.

**Commit boundary.** One commit spanning `openapi.yaml`, the regenerated `src/app/api/schema.ts`, config, harness literal, health, and the contract test. Splitting it fails CI: `verify:api-drift` runs in the **frontend** job, so a backend-only commit that skips `pnpm generate:api` fails a job that looks unrelated.

## 3.6 Sprint 6: `GET /api/v1/status` (CVR-074): touches the OpenAPI contract and the desktop host

Size: L. Positions 1 and 2 cut this as "greenfield in five places" and undercounted by two CI gates. Full shape and cascade in section 5.

**Steps.**

1. **`backend/src/persistence/sqlite.rs`**: add `failure_rollup(since: &str)` (`GROUP BY failure_code WHERE status='failed' AND updated_at >= ?1`, capped at the top eight codes) and `latest_success_at()`. `updated_at` is RFC3339 TEXT and sorts lexicographically, so a formatted cutoff bound as a parameter needs no SQLite date functions. Reuse `queue_snapshot` from sprint 3. (M)
2. **`backend/src/api/status.rs`**: new module, `StatusEnvelope { data: Status }`, `#[serde(rename_all = "camelCase")]`, `service_version: env!("CARGO_PKG_VERSION")`, copying the `CapabilitiesEnvelope` precedent in `backend/src/api/capabilities.rs`. (M)
3. **`backend/src/api/mod.rs`**: one `.route("/api/v1/status", get(status::get))` line inside the `protected` builder, plus `mod status;`. Authenticated, unlike capabilities. (S)
4. **`backend/openapi/openapi.yaml`**: **contract change.** New path, `Status` schema, `bootstrapBearer` security, 200 and 401 responses, a unique `operationId`, and a `summary` (Spectral runs at `--fail-severity=warn` and only turns off `operation-description`, `operation-tags`, and `info-contact`). (M)
5. **`backend/tests/http_contract.rs`**: extend the same drift test: `paths.len()` 8 to 9 and `("/api/v1/status", "get")` in the route list. Add one behavioral test asserting the counts move after a submit. (S)
6. **`src-tauri/src/conversion_service.rs`**: three edits: a `ContractRoute::Status` variant at line 107, an arm in `classify_route` at line 445, and `requires_authentication` at line 119. Extend the existing unit test at lines 860-871. Without all three the app answers "Conversion-service method/path combination is not allowed". (M)
7. **`src/app/api/schema.ts`**: regenerate via `pnpm generate:api`, commit in the same change. (S)

**Verify.** Run `cargo test --locked --manifest-path backend/Cargo.toml --test http_contract` and observe the 9-path assertion pass. Run `pnpm lint:api`, `pnpm verify:api-drift`, and `backend/scripts/contract-fuzz.sh` and observe the fuzz suite report nine documented paths instead of eight, twice, under both `TOOLKIT_FUZZ_SEED_ID` values. Run `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib` and observe `classify_route("GET", "/api/v1/status")` resolve. Then `curl -H "Authorization: Bearer $TOKEN" https://<host>.<tailnet>.ts.net/api/v1/status | jq '.data.queue'` returns nonzero counts while jobs run.

**Commit boundary.** One commit across backend, contract, generated schema, and `src-tauri`. `pnpm verify:api-drift` makes any split fail.

## 3.7 Sprint 7: retention and quarantine cleanup (CVR-074)

**Goal.** Bound growth. This is the only step in M7 that deletes user bytes, which is why it sits behind a restore proven twice.

**Steps.**

1. **`backend/src/config.rs` + `backend/tests/support/mod.rs`**: `TOOLKIT_CONVERTER_RETENTION_DAYS`, default 90, 0 disables. Add the field to the harness struct literal in the same commit. (S)
2. **`backend/src/persistence/sqlite.rs`**: `sweep_retention(cutoff: &str) -> Result<Vec<Uuid>, RepositoryError>`: inside one transaction, select `succeeded` conversions with `updated_at < ?1`, delete their `artifacts` and `attempts` rows, then the `conversions` rows, and return the ids. First `DELETE FROM` in the crate. Never touch `queued`, `converting_local`, `finalizing`, `failed`, or `needs_remote`. (M)
3. **`backend/src/artifacts.rs`**: `purge_job(job_id)` that removes `jobs/<id>` outright, reusing the ownership and symlink guards `discard_unaccepted_job` already applies. Add `purge_quarantined(older_than)` that unlinks under `quarantine/preacceptance/` by directory mtime. Nothing unlinks that tree today. (M)
4. **`backend/src/jobs/recovery.rs`**: call the sweep once at boot, after `quarantine_orphans`, deleting rows first and storage second so a crash mid-sweep leaves orphan bytes rather than dangling rows. Rows delete then storage deletes is the same ordering rule as the backup, for the same reason. (M)
5. **`backend/tests/crash_recovery.rs`**: extend, do not add a file. That suite already keeps one data root alive across repeated `AppState::initialize` calls, which is what a retention sweep needs. Seed aged succeeded jobs with `insert_queued_job` plus the direct-SQLite helpers, boot, and assert `count_job_directories`, `artifact_row_count`, and `attempt_count` drop by the expected amount while a fresh job still downloads with a sha matching its manifest. Add a second case asserting a `failed` row survives the sweep. (M)

**Verify.** Run `cargo test --locked --manifest-path backend/Cargo.toml --test crash_recovery` and observe both new cases pass. Then on the host, `docker run --rm -v ${PROJECT}_converter-data:/data alpine du -sh /data/jobs /data/quarantine` before and after a restart with `TOOLKIT_CONVERTER_RETENTION_DAYS=1`, and observe both shrink while `GET /api/v1/status` still resolves a recent job.

**Commit boundary.** One commit. No contract change.

## 3.8 Sprint 8: per-device credentials (CVR-071)

Size: L, the largest item in M7, larger than CVR-075 and larger than CVR-079. Position 1's "thin slice, one command" framing was refuted and does not carry forward. Detail in section 4.

**Steps.**

1. **`backend/migrations/0004_per_device_auth_scopes.sql`**: the table rebuild. Section 4. (L)
2. **`backend/migrations/0005_auth_credentials.sql`**: `auth_credentials(id TEXT PRIMARY KEY, scope TEXT NOT NULL, label TEXT NOT NULL, secret_hmac TEXT NOT NULL, created_at TEXT NOT NULL, revoked_at TEXT)` with a unique index on `secret_hmac` and a partial index on `revoked_at IS NULL`. `HMAC-SHA256(pepper, secret)`, not Argon2, per the recorded decision: a 256-bit CSPRNG secret is bounded by entropy, and a slow KDF on the unauthenticated path of a one-worker CPU-only box is a self-inflicted denial of service. (M)
3. **`backend/src/auth.rs`**: `BootstrapAuth` becomes `CredentialStore`. `authorizes(&HeaderMap) -> bool` becomes `resolve(&HeaderMap) -> Option<Scope>`. Constant-time compare over the loaded set via `subtle`, keeping the duplicate-Authorization-header rejection and the zeroize of the plaintext. Add `reload()` so rotation does not need a restart, which is what STATUS gate 6 actually asks for. (L)
4. **`backend/src/api/mod.rs`**: `require_auth` inserts the resolved `Scope` into request extensions, the same way `trace_request` inserts `RequestId`. (S)
5. **Thread the scope**: `backend/src/api/conversions.rs` `create` adds it to `Submission`, `ConversionService::submit` passes it to `NewConversion` in `backend/src/persistence/model.rs`, and the four request-path reads (`get`, `list_artifacts`, `download_markdown`, `download_manifest`) pass it into `load_conversion`. `NewConversion` has three construction sites: `sqlite.rs:3282`, `conversion/service.rs:143`, `tests/support/mod.rs:182`. (L)
6. **`backend/src/persistence/sqlite.rs`**: split the fifteen `AUTH_SCOPE` binds. Section 4 names which go and which stay. (L)
7. **Bootstrap transition**: the migration seeds one `auth_credentials` row from the existing token file with scope `bootstrap`, so the shipped signed DMG keeps working. Removing that row is a later, deliberate act. (S)

**Verify.** Issue two credentials, revoke one through the CLI path, and without restarting the container observe the revoked bearer return 401 with the `ErrorEnvelope` shape from `backend/src/error.rs` while the other returns 200. Run `cargo test --locked --manifest-path backend/Cargo.toml` and observe the new cross-scope isolation and two-scope recovery tests pass. Then restart the container and observe `docker compose logs converter | jq -r 'select(.fields.message | test("quarantined"))'` print nothing.

**Commit boundary.** Three commits, each green: (a) migrations 0004 and 0005 plus the widened `AUTH_SCOPE` handling with the constant still bound to `bootstrap` everywhere it stays, (b) `auth.rs` plus the extension plumbing, (c) the CLI issue/revoke path plus docs.

**Contract.** None. The response shapes do not change, and a 404 for another scope's job reuses the existing not-found envelope.

## 4. The auth_scope migration

The riskiest item in M7. The migration itself is the cheap half. Migrations `0002_non_pdf_source_formats.sql` and `0003_anydoc_full_format_set.sql` are working templates that already rebuild both tables, twice. Copy `0003`'s shape and do not invent one.

**The rebuild, in order.**

1. `-- no-transaction` marker comment at the top of the file.
2. `PRAGMA foreign_keys = OFF;` **outside** any transaction. `PRAGMA foreign_keys` is a no-op inside one, and `SqliteRepository::open` builds the pool with `.foreign_keys(true)`, so this is what makes the DROP legal.
3. `BEGIN;` explicitly. sqlx runs the file in autocommit, so without it a kill between DROP and RENAME leaves an unbootable schema.
4. `CREATE TABLE conversions_new (...)` with the column order **byte-identical** to `0003`. Widen only the `auth_scope` CHECK, from `auth_scope = 'bootstrap'` to `length(auth_scope) BETWEEN 1 AND 64 AND auth_scope = lower(auth_scope) AND auth_scope NOT GLOB '*[^0-9a-z_-]*'`. Reproduce every other constraint verbatim: `id` and `client_run_id` length 36 and lowercase, `idempotency_key_hash` and `request_fingerprint` length 64 with `NOT GLOB '*[^0-9a-f]*'`, `profile IN` three values, `status IN` six, `source_media_type IN` the eighteen media types `0003` established, `source_byte_length > 0`, `source_sha256` 64 hex, the table-level CHECKs on `source_relative_path` (length 512 and equal to `'jobs/' || id || '/source/input'`), `origin_request_id` 1..128, `route` 1..128, `failure_code` 1..128, `failure_message` 1..1024, `(failure_code IS NULL) = (failure_message IS NULL)`, the `UNIQUE(auth_scope, idempotency_key_hash)`, and the `DEFERRABLE INITIALLY DEFERRED` FK on `(id, active_attempt_id)` into `attempts(conversion_id, id)`.
5. `INSERT INTO conversions_new SELECT * FROM conversions;`. positional. Reordering one column silently shifts data.
6. `DROP TABLE conversions;`
7. `ALTER TABLE conversions_new RENAME TO conversions;`
8. `CREATE TABLE attempts_new (...)`, also rebuilt, because the FK in step 4 targets a two-column key on `attempts`. Reproduce `state IN` seven values and `classification IN` five including `structured_document`, and keep `queue_seq INTEGER PRIMARY KEY AUTOINCREMENT` in the same position.
9. `INSERT INTO attempts_new SELECT * FROM attempts;`. the copy preserves rowids, so the AUTOINCREMENT high-water mark survives and the FIFO claim order does not reshuffle.
10. `DROP TABLE attempts;` then `ALTER TABLE attempts_new RENAME TO attempts;`
11. `CREATE INDEX idx_conversions_active ...;` and `CREATE INDEX idx_attempts_fifo ...;` Both die with the dropped tables and must come back, and `idx_attempts_fifo` is partial on `state = 'queued'`.
12. `COMMIT;` then `PRAGMA foreign_keys = ON;`

**Idempotency across a rotation.** STATUS line 473 records this as an open question. Resolve it this way: **the scope is the device, not the credential.** `auth_credentials.scope` is a stable device slug and rotating a device's secret issues a new row against the same scope, so `UNIQUE(auth_scope, idempotency_key_hash)` sees no change and `create_or_replay` keeps returning Replay after a rotation. Rotation stays invisible to idempotency, which is the property that matters, because a client retrying across a rotation must not get a second job and a second bill. Two different devices sending the same Idempotency-Key now produce two conversions where they previously collided. That is correct, since their `clientRunId` and request fingerprints differ anyway, and it is the behavior change to write down rather than discover.

**The fifteen bind sites split two ways.** They are not one thing.

- **Keep the filter, take the scope from the request** (four sites): the replay lookup at `sqlite.rs:88`, the insert at 141, and `load_conversion` at 1388, which backs `get`, `list_artifacts`, `download_markdown`, and `download_manifest` in `backend/src/api/conversions.rs`. These are tenancy. A scope must not read another scope's job.
- **Drop the filter** (the rest): `claim_next_queued` (235), `start_local` (432), `mark_finalizing` (538), `finish_needs_remote` (589), `finish_failed` (628), `interrupt_and_requeue` (734), `list_recovery_candidates` (773), `list_conversion_ids` (819), and 917, 968, 1015, 1149. These are worker lifecycle writes reached from a job runner that has no caller identity and never will. The conversion id and attempt id are already unique, so the scope bind buys nothing and costs correctness.

**`list_conversion_ids` at line 819 is the data-destruction path.** Widen the CHECK and leave that filter pinned to `bootstrap`, and the next boot's `StartupRecovery::quarantine_orphans` renames every non-bootstrap job's storage into `quarantine/preacceptance/`, because those ids no longer appear in the set it diffs against `ArtifactStore::list_owned_job_ids`. `list_recovery_candidates` at 773 has the same shape with a milder outcome: recovery goes blind to other devices' interrupted jobs.

**Test updates.**

- `backend/src/persistence/sqlite.rs:2370` asserts `row.try_get::<String,_>("auth_scope") == AUTH_SCOPE`. It becomes an assertion against the scope the test seeded. The import at line 1734 goes with it.
- `backend/tests/support/mod.rs` gains a `with_scope` constructor and a scope argument on `insert_queued_job`, `insert_queued_without_notification`, and `insert_converting_job`. `Settings` is unaffected, since credentials live in the database rather than in config.
- `backend/tests/crash_recovery.rs` gains `two_scopes_both_survive_a_restart_without_quarantine`: seed one job under each of two scopes, boot, and assert `count_job_directories` is unchanged and no quarantine directory appeared. That test fails if step "drop the filter" is missed on `list_conversion_ids`, which is exactly the failure worth catching automatically.
- `backend/tests/http_contract.rs` gains `a_scope_cannot_read_another_scopes_conversion`: submit under scope A, `GET` under scope B, assert 404 with the existing not-found envelope rather than 403, so the endpoint leaks no existence.
- `backend/tests/failure_modes.rs` already owns `migration_failure_prevents_startup` and needs no change, but run it: it is the proof that rolling the image back across 0004 refuses to boot rather than silently downgrading.

## 5. `GET /api/v1/status`

Authenticated, inside the `protected` sub-router in `backend/src/api/mod.rs`. It answers operator questions. Job UX progress stays on the existing per-job routes, per CVR-074.

**Response shape.** `StatusEnvelope { data: Status }`, `#[serde(rename_all = "camelCase")]`, copying `CapabilitiesEnvelope` in `backend/src/api/capabilities.rs`.

```
data.serviceVersion            string   env!("CARGO_PKG_VERSION")
data.acceptingJobs             bool
data.queue.queued              integer
data.queue.convertingLocal     integer
data.queue.finalizing          integer
data.queue.succeeded           integer
data.queue.failed              integer
data.queue.needsRemote         integer
data.queue.maxActiveJobs       integer
data.worker.idle               bool
data.worker.lastSuccessAgeSeconds  integer | null
data.failures[].code           string    top 8 codes in the last 24h
data.failures[].count          integer
data.storage.freeBytes         integer
data.storage.minFreeBytes      integer
```

No paths, no filenames, no request bodies, no per-job detail. `ApiError`'s `code` and `message` are `&'static str` in `backend/src/error.rs`, so the error side needs no change.

**Repository queries.** `queue_snapshot()` lands in sprint 3 and is reused here. `failure_rollup(since)` and `latest_success_at()` are new. `accepting_jobs()` already exists and stays as the bool, since the count now comes from `queue_snapshot`. Free bytes comes from `ArtifactStore::free_bytes`, also sprint 3. There is no duration arithmetic anywhere in the crate today, and this endpoint adds none: `created_at` and `updated_at` are RFC3339 TEXT and compare lexicographically against a cutoff formatted in Rust.

**The contract cascade, all seven gates.**

1. `backend/openapi/openapi.yaml`: new path, `Status` schema, `bootstrapBearer` security, 200 and 401.
2. `backend/tests/http_contract.rs:2087`: `paths.len()` 8 to 9 and the route list.
3. `pnpm lint:api` (Spectral, `--fail-severity=warn`): unique `operationId` and a `summary` are mandatory, and any example must validate against its own schema.
4. `pnpm generate:api` then `pnpm verify:api-drift`, which runs in the **frontend** CI job. A backend-only commit fails a job that reads as unrelated.
5. `backend/scripts/contract-fuzz.sh`, which runs in the **backend** CI job, twice under `TOOLKIT_FUZZ_SEED_ID`. `positive_data_acceptance` is disabled only for `createConversion`, so the new operation must answer 200 for anything Schemathesis generates. It takes no parameters, which is what makes that safe.
6. `src-tauri/src/conversion_service.rs`: a `ContractRoute` variant, a `classify_route` arm, `requires_authentication`, and the unit test at lines 860-871. `classify_route` is a closed allowlist and answers everything else with "Conversion-service method/path combination is not allowed".
7. `src/app/api/schema.ts` regenerated and committed in the same change.

A shipped DMG that predates this endpoint is unaffected. `ConversionCapabilitiesWire` in `src-tauri/src/conversion_service.rs` sets no `deny_unknown_fields`, so new response fields are additive, and an old app simply cannot call a route its allowlist does not know.

## 6. The M7 gate

M7 is done when all of these are true and each has evidence on disk or in the repo.

- `backend/scripts/container-smoke.sh --phase restore` exits 0 **on the TrueNAS host**, and its `evidence.md` shows a restored job's markdown sha256 equal to the pre-backup manifest `.output.sha256`, its ETag equal to `"sha256-<that sha>"`, an artifact count of 2, the held job at 404, and one directory under `quarantine/preacceptance`.
- `docker inspect` on the converter shows `HostConfig.NetworkMode` equal to `container:<tailscale id>` and `NetworkSettings.Ports` equal to `{}`.
- An authenticated request from a device on cellular over the tailnet returns 200 from `/api/v1/capabilities` and `/api/v1/status`, while a `curl --max-time 5` to `http://<lan-ip>:8080/health/live` from an off-tailnet LAN host fails to connect, and a `nc -z -w2` from another container on the default bridge fails too.
- `/health/ready` returns 503 with `checks.storage: "failed"` when free space is below `TOOLKIT_CONVERTER_MIN_FREE_BYTES`, and 200 with four `"ok"` checks otherwise.
- The boot config line names `data_dir`, `max_jobs`, and `shutdown_grace_secs`, and changing one knob in `.env` followed by `docker compose up -d` changes the value in that line.
- `SqliteRepository::open` refuses an NFS or SMB data root with `RepositoryError::NetworkFilesystem`, and `stat -f -c %T` on the deployed volume mountpoint prints a local type.
- Two credentials are issued, one is revoked, and the revoked bearer returns 401 **without a container restart** while the other returns 200.
- A restart after retention has swept shows no quarantine warning in the logs, and `du -sh /data/jobs /data/quarantine` shrinks.
- `pnpm verify:all`, `pnpm verify:deps`, `pnpm verify:contract`, and `pnpm verify:container` all exit 0 on the dev Mac.
- `docs/STATUS.md` section 4 sequence table is updated to this plan's order, with items 1 and 2 moved to the deferred list below.

## 7. Deferred out of M7

**CVR-079, image build caching and registry release.** Building on the amd64 deploy host from a checkout needs no registry, no cross-build, and no cargo-chef, and the digest-pin rollback lever CVR-079 would provide is worthless across a migration boundary anyway: migrations run forward, `migrations/` holds no down files, and `backend/tests/failure_modes.rs::migration_failure_prevents_startup` proves an older binary refuses a newer schema. The real rollback is restore the paired snapshot **then** repin, and sprint 1 already ships the snapshot half. **Trigger:** ten image builds inside one month, or a second deploy target. When it comes back, cargo-chef's cook stage and the real build stay in the **same** stage, because the `find /usr/local/cargo/registry/src ... pdf-inspector-1.15.0` glob at `backend/Dockerfile:19` runs in the same `RUN` as the build and cannot survive into a stage that receives only `target/`. Any release tag also needs `openapi/openapi.yaml` `info.version` bumped with `Cargo.toml`.

**Scheduled backups.** Sprint 1 ships the command and the proven restore, not a cron. **Trigger:** the first month where a manual backup is missed.

**A `/metrics` endpoint.** One container with one worker does not justify a second listener or a time-series database. The stats log line plus `GET /api/v1/status` cover it. **Trigger:** a second converter host.

**Continuous orphan reconciliation, and the reverse case.** `quarantine_orphans` stays boot-only and one-directional. A row whose bytes vanished is left to the failure the next download produces. **Trigger:** a download 500 that a boot sweep would have caught.

**Multi-arch images.** amd64 only, as recorded. `codegen-units = 1` with thin LTO under QEMU is untenable. **Trigger:** an arm64 deploy host.

**Cloudflare Tunnel.** Held in reserve for one case only: reaching the API from a device that cannot run Tailscale. Its 100 MB body cap and roughly 100 second origin read timeout fit today's 25 MB upload cap and stop fitting the moment that cap rises. **Trigger:** a device that cannot join the tailnet.

**CVR-076 SBOM, CVR-077 runbooks, CVR-078 host-level test depth.** Already out of scope in the ticket set and unaffected by this ordering.
