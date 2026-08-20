# Yaak architecture reference (steal patterns, not the product)

**Last verified:** 2026-08-17 against [`mountain-loop/yaak` `main`](https://github.com/mountain-loop/yaak).  
**Repo:** https://github.com/mountain-loop/yaak  
**Do not fork Yaak.** It is an API client. Tool-Kit is drop → convert/transcribe. Copy host, folder, and contract patterns. Leave their UI, router, plugin marketplace, and Git-sync of collections behind.

This note exists because the TrueNAS split is a **host** problem. If React `fetch`es the conversion box, the bearer token and file bytes land in the webview. Yaak already solved that class of mistake. The files below are the working source, not a blog summary.

Links are `blob/main/...`. Paths can move. Re-check `main` before copying.

## License

Yaak's GitHub tree is [MIT](https://github.com/mountain-loop/yaak/blob/main/LICENSE). Reuse of source is allowed. Keep the copyright notice if you copy a substantial portion.

Official prebuilt binaries are a different product. [Yaak's terms](https://yaak.app/terms) say those terms apply to binaries from Mountain Loop Labs, and that use of the source alone is MIT. Tool-Kit is not shipping Yaak binaries. We are reading their code.

## What we got wrong earlier

Yaak does **not** use specta for IPC. Current `main` uses:

| Layer | Yaak | Tool-Kit |
|---|---|---|
| Conversion-box HTTP contract | N/A (Yaak is the client) | OpenAPI: `backend/openapi/openapi.yaml` → `pnpm generate:api` → `src/app/api/schema.ts` |
| Desktop command contract | [`ts-rs`](https://github.com/mountain-loop/yaak/blob/main/Cargo.toml) `11.1.0` + [`define_rpc!`](https://github.com/mountain-loop/yaak/blob/main/crates/common/yaak-rpc/src/lib.rs) | Today: handwritten `src/app/commands.ts`. Later: generated Tauri/RPC types for the Mac, not a second OpenAPI client in the webview |
| Wire to Rust | One Tauri command `rpc` with `{ cmd, payload }` | M6: one Tauri command `service_request` for the box. Existing commands stay for local jobs, settings, Keychain |

## How the repo is laid out

Workspace members are listed in [`Cargo.toml`](https://github.com/mountain-loop/yaak/blob/main/Cargo.toml):

- `crates/`: engines with **no** Tauri dependency (`yaak-http`, `yaak-models`, `yaak-crypto`, `yaak-plugins`, …)
- `crates/common/`: `yaak-rpc`, `yaak-rpc-schema`, `yaak-database`
- `crates-tauri/`: desktop adapters (`yaak-app-client`, `yaak-mac-window`, `yaak-system-appearance`, …)
- `crates-cli/`: CLI host
- `crates-proxy/`: hosted send proxy
- `apps/yaak-client/`: React UI
- `packages/platform/`: the **only** TypeScript module that may import `@tauri-apps/*`

That split is the pattern. HTTP, SQLite, encryption, and send orchestration compile without Tauri. The desktop crate only supplies a window, a response directory, and a Keychain.

Tool-Kit already has a thinner version of the TS side in `src/platform/host.ts`. Do not create a root Cargo workspace here (see `CLAUDE.md`). Steal the *folder* idea when `src-tauri` and `backend` are ready to share crates, not before.

## The concern, in Yaak's own words

A command handler is written against a host, not against Tauri:

> A command handler is invoked on behalf of one client (a desktop window today) and needs a handful of things from its surroundings. `Host` is that handful and nothing more. The desktop implements it over a `WebviewWindow`. A server would implement it over a connection. Handlers are generic over it, so the same handler body runs under either without knowing which.
>
> What is deliberately *not* here is anything only a desktop can do: open a native window, run the updater, show a native dialog.

Source: [`crates/yaak-commands/src/host.rs`](https://github.com/mountain-loop/yaak/blob/main/crates/yaak-commands/src/host.rs) (file header, lines 1–16).

The TypeScript twin is the platform package. Desktop installs Tauri. A browser build swaps the entry so `@tauri-apps/*` never enters the module graph:

> This line is the swap point, and a browser build swaps it by resolving the package to `index.web.ts` instead of this file. Because nothing else in the app imports a host directly, that is the whole change.

Source: [`packages/platform/src/index.ts`](https://github.com/mountain-loop/yaak/blob/main/packages/platform/src/index.ts).

Their browser host is **not** Tool-Kit's plan (tokens stay in Tauri, not in a tab). The README is still the best write-up of "same UI, different host, capabilities tell the truth": [`packages/platform/src/web/README.md`](https://github.com/mountain-loop/yaak/blob/main/packages/platform/src/web/README.md).

If the Mac webview ever `fetch`es `https://truenas.local/...` with a bearer token, we have recreated the thing they spent this architecture avoiding.

## Pattern catalog

Each row: what to copy, exact files, what to leave.

### 1. Host trait (desktop vs future server)

**Steal.** Handlers take `Host`, not `AppHandle`. Native dialogs and windows stay desktop-only.

| File | Why |
|---|---|
| [`crates/yaak-commands/src/host.rs`](https://github.com/mountain-loop/yaak/blob/main/crates/yaak-commands/src/host.rs) | `Host` and `PluginHost` traits. `PluginHost` is a separate trait so a DB-only command never demands a plugin runtime. |
| [`crates/yaak-commands/src/lib.rs`](https://github.com/mountain-loop/yaak/blob/main/crates/yaak-commands/src/lib.rs) | Handler crate. Bodies live here. Schema lives elsewhere. |
| [`crates-tauri/yaak-app-client/src/rpc_ext.rs`](https://github.com/mountain-loop/yaak/blob/main/crates-tauri/yaak-app-client/src/rpc_ext.rs) | Desktop `ClientCtx` implements `Host`. One Tauri command `rpc` is the envelope. Lines 1–17 state the contract. |
| [`crates-cli/yaak-cli`](https://github.com/mountain-loop/yaak/tree/main/crates-cli/yaak-cli) | Proof the same engines run without a webview. |

**Do not steal.** PluginHost as "spawn Node and talk over a socket". Tool-Kit's analog of an isolated engine is the PDF worker (`backend/src/bin/tool-kit-pdf-worker.rs`), not a Node sidecar.

**Tool-Kit mapping.** M6 `service_request` is the desktop host talking to the conversion box. `ConversionService` on the box is the other host. Do not put Datalab/Rev.ai credentials in either webview.

### 2. Typed RPC + TS codegen (ts-rs, not specta)

**Steal the idea.** One schema crate. Generated TS. A typo'd command name is a compile error.

| File | Why |
|---|---|
| [`Cargo.toml` workspace.dependencies `ts-rs`](https://github.com/mountain-loop/yaak/blob/main/Cargo.toml) | Pin is `ts-rs = "11.1.0"`. |
| [`crates/common/yaak-rpc/src/lib.rs`](https://github.com/mountain-loop/yaak/blob/main/crates/common/yaak-rpc/src/lib.rs) | `RpcRouter`, `RpcRequest`/`RpcResponse`, `define_rpc!` exporting `RpcSchema` to `gen_rpc.ts`. Transport-agnostic `dispatch()`. |
| [`crates/common/yaak-rpc-schema/src/lib.rs`](https://github.com/mountain-loop/yaak/blob/main/crates/common/yaak-rpc-schema/src/lib.rs) | Wire schema. Header: "Nothing here depends on Tauri or on any host." |
| [`apps/yaak-client/lib/rpc.ts`](https://github.com/mountain-loop/yaak/blob/main/apps/yaak-client/lib/rpc.ts) | Frontend: `cmd` is `keyof RpcSchema`. Host-plugin commands (`plugin:…`) stay behind their own facades. |
| [`packages/platform/src/tauri/index.ts`](https://github.com/mountain-loop/yaak/blob/main/packages/platform/src/tauri/index.ts) | `invoke("rpc", { cmd, payload })`. Blobs are ids. Path lookup is `cmd_http_response_body_path`. `rpcStream` registers the listener **before** the command runs. |

**Do not steal.** Generating 109 API-client commands into a 560px instrument face. Tool-Kit's Mac command surface stays small (`scan_inputs`, `run_pipeline`, `save_settings`, `service_request`, …).

**Tool-Kit mapping.** The conversion box already has OpenAPI. Do not duplicate that contract with ts-rs. If we later generate Mac IPC types, generate them from the Tauri commands, and keep `src/platform/host.ts` as the only `@tauri-apps/*` import (Yaak's `packages/platform` rule).

### 3. HTTP without Tauri (sleep, LAN, long uploads)

**Steal the crate split and the body-on-disk rule.** Leave cookie jars, TLS client certs, and request templating.

| File | Why |
|---|---|
| [`crates/yaak-http/src/lib.rs`](https://github.com/mountain-loop/yaak/blob/main/crates/yaak-http/src/lib.rs) | Module list: `client`, `sender`, `transaction`, `manager`, `tee_reader`. |
| [`crates/yaak-http/src/client.rs`](https://github.com/mountain-loop/yaak/blob/main/crates/yaak-http/src/client.rs) | reqwest client construction. Independent of Tauri. |
| [`crates/yaak-http/src/sender.rs`](https://github.com/mountain-loop/yaak/blob/main/crates/yaak-http/src/sender.rs) | Actual send. |
| [`crates/yaak-http/src/transaction.rs`](https://github.com/mountain-loop/yaak/blob/main/crates/yaak-http/src/transaction.rs) | Cancellation + event stream around one exchange. |
| [`crates/yaak-http/src/manager.rs`](https://github.com/mountain-loop/yaak/blob/main/crates/yaak-http/src/manager.rs) | Connection reuse. |
| [`crates/yaak/src/send.rs`](https://github.com/mountain-loop/yaak/blob/main/crates/yaak/src/send.rs) | Orchestration: persist metadata, chunk bodies, write under `response_dir`. `SendHttpRequestByIdParams.response_dir` (~line 231). On connect they `create_dir_all` and join `response.id` (~lines 775–781). |
| [`crates-tauri/yaak-app-client/src/http_request.rs`](https://github.com/mountain-loop/yaak/blob/main/crates-tauri/yaak-app-client/src/http_request.rs) | Desktop adapter. `response_dir = app_data_dir/responses` (~line 160). Calls the Tauri-free `send_http_request_with_plugins`. |

Their web README "slice 2" is the TrueNAS send path in prose: the client posts a rendered request, the proxy keeps nothing, bodies go through `blob_put`, cookies round-trip. See [slice 2 in the browser-host README](https://github.com/mountain-loop/yaak/blob/main/packages/platform/src/web/README.md).

**Do not steal.** Putting response bytes in React state. Yaak's frontend holds an id and asks the host for a path or a `convertFileSrc` URL.

**Tool-Kit mapping.** Keep `providers.rs` retry rule: retry only failures that prove the server never started work (429/502/503/504/529 and connect errors). Pair that with M2 `Idempotency-Key` / `clientRunId`. 30-minute uploads stay in Rust (`UPLOAD_TIMEOUT`). The webview never streams the PDF.

### 4. Metadata vs blobs

**Steal.** SQLite rows are metadata. Bodies are chunks or files. The UI never owns the bytes.

| File | Why |
|---|---|
| [`crates/yaak-models/src/query_manager.rs`](https://github.com/mountain-loop/yaak/blob/main/crates/yaak-models/src/query_manager.rs) | DB pool for models. |
| [`crates/yaak-models/src/blob_manager.rs`](https://github.com/mountain-loop/yaak/blob/main/crates/yaak-models/src/blob_manager.rs) | Separate blob DB. `BodyChunk` rows. Comment on the type: do not wrap the pool in a Mutex (it serializes every blob access). `get_body_size` without loading data. |
| [`crates/yaak-models/src/migrate.rs`](https://github.com/mountain-loop/yaak/blob/main/crates/yaak-models/src/migrate.rs) | Model migrations. |
| [`packages/platform/src/tauri/index.ts`](https://github.com/mountain-loop/yaak/blob/main/packages/platform/src/tauri/index.ts) `blobs.read` / `blobs.url` | Frontend: id → path → `readFile` / `convertFileSrc`. |

**Do not steal.** Their HTTP-response data model (`HttpResponse` rows, cookie jars). Tool-Kit already has `history.rs` (path-keyed reuse) and the conversion service's `ArtifactStore`.

**Tool-Kit mapping.** Job rows and history stay metadata. Markdown and audio stay on disk. `read_text_file` (2 MB cap) is the inspector path. Do not put `outputText` for a 200-file run into React.

### 5. Isolated engines (child process, not in-process plugins)

**Steal the isolation.** Do not steal Node.

| File | Why |
|---|---|
| [`crates/yaak-plugins/src/manager.rs`](https://github.com/mountain-loop/yaak/blob/main/crates/yaak-plugins/src/manager.rs) | Runtime lifecycle. |
| [`crates/yaak-plugins/src/nodejs.rs`](https://github.com/mountain-loop/yaak/blob/main/crates/yaak-plugins/src/nodejs.rs) | `start_nodejs_plugin_runtime`: spawn sidecar, `HOST`/`PORT` env, kill channel, unexpected-exit watch. |
| [`crates/yaak-plugins/src/events.rs`](https://github.com/mountain-loop/yaak/blob/main/crates/yaak-plugins/src/events.rs) | Event types across the socket. |
| [`crates-tauri/yaak-app-client/src/plugins_ext.rs`](https://github.com/mountain-loop/yaak/blob/main/crates-tauri/yaak-app-client/src/plugins_ext.rs) | Wires vendored `yaaknode` + `index.cjs`. Crash toast. Exit drains the sidecar. Dev-path comment: in-place rewrite of the Node binary on macOS taints the code signature (`SIGKILL`). |

**Do not steal.** A plugin marketplace, template tags, or auth plugins. Extra conversion formats belong in a child worker (already the PDF inspector worker), not in the webview and not in a Node runtime.

### 6. Secrets stay in the OS keychain

**Steal the Keychain + in-process encrypt split.** Leave workspace-key UX.

| File | Why |
|---|---|
| [`crates/yaak-crypto/src/master_key.rs`](https://github.com/mountain-loop/yaak/blob/main/crates/yaak-crypto/src/master_key.rs) | `keyring::Entry`. Master key created on first use. XChaCha20-Poly1305. |
| [`crates/yaak-crypto/src/encryption.rs`](https://github.com/mountain-loop/yaak/blob/main/crates/yaak-crypto/src/encryption.rs) | Encrypt/decrypt primitives. |
| [`crates/yaak-crypto/src/manager.rs`](https://github.com/mountain-loop/yaak/blob/main/crates/yaak-crypto/src/manager.rs) | Workspace keys wrapped by the master key. Comment on `get_master_key`: hold the lock for the whole call so concurrent access does not prompt Keychain twice. |
| [`crates/yaak-commands/src/encryption.rs`](https://github.com/mountain-loop/yaak/blob/main/crates/yaak-commands/src/encryption.rs) | Host-generic commands. Thin wrappers. |

**Do not steal.** Human-exportable workspace keys (`YKM_…`) unless we later encrypt collections on disk. Tool-Kit keys are already Keychain accounts `datalab` / `revai` (service `dev.esfandiari.toolkit`). M6 adds the conversion-box token the same way. They live in the data protection keychain via `security-framework`.

### 7. Capabilities as a honest host report

**Steal.** The UI gates on `useCapability("localFiles")`, not `if (isTauri)`.

| File | Why |
|---|---|
| [`packages/platform/src/capabilities.ts`](https://github.com/mountain-loop/yaak/blob/main/packages/platform/src/capabilities.ts) | `useCapability(name)`. |
| [`packages/platform/src/tauri/index.ts`](https://github.com/mountain-loop/yaak/blob/main/packages/platform/src/tauri/index.ts) `ALL_CAPABILITIES` | Desktop: everything true. |
| [`packages/platform/src/web/README.md`](https://github.com/mountain-loop/yaak/blob/main/packages/platform/src/web/README.md) Capabilities table | Browser: `localFiles`, `encryption`, `plugins`, `git` are false. Declined commands return `UnsupportedCommandError` with a reason. |

**Tool-Kit mapping.** The conversion box already has `GET /api/v1/capabilities` ([`STATUS.md`](../STATUS.md) section 7). Desktop should read that through `service_request`, not invent a second list in React. Version skew is a capabilities problem, not a UI problem.

### 8. macOS chrome (optional, later)

**Steal only if we hit a native-chrome bug.** We already use overlay title bar + vibrancy.

| File | Why |
|---|---|
| [`crates-tauri/yaak-mac-window/src/lib.rs`](https://github.com/mountain-loop/yaak/blob/main/crates-tauri/yaak-mac-window/src/lib.rs) | Traffic-light positioner on `window_ready`. |
| [`crates-tauri/yaak-mac-window/src/mac.rs`](https://github.com/mountain-loop/yaak/blob/main/crates-tauri/yaak-mac-window/src/mac.rs) | AppKit bits. |
| [`crates-tauri/yaak-mac-window/src/commands.rs`](https://github.com/mountain-loop/yaak/blob/main/crates-tauri/yaak-mac-window/src/commands.rs) | `set_title` / `set_theme`. |
| [`crates-tauri/yaak-system-appearance/src/lib.rs`](https://github.com/mountain-loop/yaak/blob/main/crates-tauri/yaak-system-appearance/src/lib.rs) | OS appearance vs window appearance. Polls `dark_light::detect()`. |
| [`crates-tauri/yaak-window/src/window.rs`](https://github.com/mountain-loop/yaak/blob/main/crates-tauri/yaak-window/src/window.rs) | Window open/state. |

**Do not steal.** Their multi-window settings UI.

### 9. Frontend folder rules

**Steal the host boundary.** Do not steal the app.

| Path | Why |
|---|---|
| [`packages/platform/`](https://github.com/mountain-loop/yaak/tree/main/packages/platform) | Only `@tauri-apps/*` imports. |
| [`apps/yaak-client/lib/rpc.ts`](https://github.com/mountain-loop/yaak/blob/main/apps/yaak-client/lib/rpc.ts) | Typed `rpc()` wrapper. |
| [`apps/yaak-client/`](https://github.com/mountain-loop/yaak/tree/main/apps/yaak-client) | commands, components, lib, routes. |

**Do not steal.** TanStack Router (`routeTree.gen.ts`), Tailwind, their component kit, Zustand, or an OpenAPI client that talks to the network from the webview. Tool-Kit's `src/app/api/` is typed against the conversion contract and plugged into a Tauri `fetch`. That is the opposite of browser-fetching the box.

## Roadblocks → Yaak file → Tool-Kit answer

| Roadblock | Read in Yaak | Do here |
|---|---|---|
| Sleep / LAN drop mid-upload | `yaak-http` transaction + `send.rs` cancellation | Retry only if work never started. `Idempotency-Key` + `clientRunId`. Server is source of truth after submit. |
| 30-minute uploads | Desktop adapter sets `response_dir`. Body not in IPC payload. | `UPLOAD_TIMEOUT` in `providers.rs`. M6 streams from disk in Rust. |
| Version skew Mac vs box | `AppMetaData` in rpc-schema. `useCapability`. | `GET /api/v1/capabilities`. Gate Run on what the box reports. |
| Reconnect after crash | Model rows + blob ids survive the UI | M2 durable jobs. Desktop looks up `clientRunId`. CVR-063. |
| Huge artifacts | `blob_manager` + path ids | History paths + `read_text_file` cap. Never `outputText` for the whole run. |
| Tokens in the webview | Keychain master key. Platform `rpc` only. | Keychain. `service_request` attaches the header. |
| Extra formats / heavy PDF | Plugin sidecar (wrong analog) | Child worker (`tool-kit-pdf-worker`). Not in-process, not Node. |

## What not to copy (even though it is good Yaak)

- TanStack Router, Query, or a file-per-route app
- Node plugin runtime as a product
- Git-sync of collections (`crates/yaak-git`, `crates/yaak-sync`, `cmd_git_*`)
- SSE/WebSocket/gRPC engines (`yaak-sse`, `yaak-ws`, `yaak-grpc`) until polling the conversion job actually hurts
- Browser wasm SQLite host (`crates/yaak-web`, `packages/platform/src/web/`) as Tool-Kit's LAN UI
- Their license/updater/fonts Tauri plugins

## How this lands on M6

Already in this tree:

- `src/platform/host.ts`: only Tauri imports for dialogs, window, drag-drop
- `src/app/api/{schema,client,transport}.ts`: OpenAPI types + `hostFetch` → `service_request`
- Keychain in `src-tauri` for Datalab/Rev.ai

Still M6 ([`BACKEND_EPIC.md`](BACKEND_EPIC.md) CVR-060–067):

1. Rust `service_request` handler
2. Backend URL in settings, device token in Keychain
3. Stream the dropped file from disk (source path in the JSON, bytes never in the webview)
4. Poll durable job state, recover with `clientRunId`
5. Download markdown through Tauri

When you implement those, open the Yaak files in this note first. Copy the split (engine crate vs host adapter vs blob id). Do not copy the HTTP-client domain model.
