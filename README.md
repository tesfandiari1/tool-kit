# Tool-Kit

A macOS menu-bar utility that turns files into AI-ready text. Drop files or
folders, hit Run, get markdown or transcripts back next to the originals.

**Current work:** local same-machine closeout. Start at
[`docs/STATUS.md`](docs/STATUS.md).

## Current desktop implementation

| Job | Service | In → Out |
|---|---|---|
| Convert | Datalab | PDF / DOCX / XLSX / images / … → Markdown |
| Transcribe | Rev.ai | audio / video → text |

The job is chosen from what you drop, and results default to the folder the
input came from. API keys live in the macOS Keychain and are not exposed to the
webview or stored in plaintext application settings.

## Develop

Requires Node 20.19+ (24 preferred), pnpm 11, and Rust 1.97. The repo pins
those in `.node-version`, `package.json`, and `rust-toolchain.toml`.

```bash
pnpm install
pnpm tauri dev     # first run compiles the Rust desktop crate, so it is slow
```

```bash
pnpm lint          # type-aware ESLint
pnpm build         # frontend only: tsc + vite build

cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo clippy --manifest-path backend/Cargo.toml --all-targets -- -D warnings
```

See [`docs/STATUS.md`](docs/STATUS.md) for session state and
[`CLAUDE.md`](CLAUDE.md) for desktop architecture, live API smokes, and
signing details. Weekly Dependabot keeps npm, both Cargo crates, and Actions
current; do not jump to TypeScript 7 or `keyring` 4.

## Test

```bash
pnpm lint && pnpm build
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo test --locked --manifest-path backend/Cargo.toml
```

Ignored desktop tests in `src-tauri/src/live_smoke.rs` hit real Datalab and
Rev.ai endpoints and spend API credits. CI runs the commands above on every
push and pull request.

## Release

macOS `app` + `dmg` only. Sign with a Developer ID; notarize the DMG if you
will distribute it.

```bash
APPLE_SIGNING_IDENTITY="Developer ID Application: …" pnpm tauri build
```

Artifacts land in `src-tauri/target/release/bundle/`. Signing credentials live
in `.env.local` (gitignored).

## Conversion backend

The production conversion service lives in [`backend/`](backend/). It converts
native-text PDFs and supported office formats on loopback. M4 routing policy and M6 desktop integration are landed. Backend Datalab
fallback is Phase 2. LAN deploy is Phase 3. See [`docs/STATUS.md`](docs/STATUS.md).

- [`docs/STATUS.md`](docs/STATUS.md): state, critical path, milestones, traps, verify commands
- [`backend/README.md`](backend/README.md): backend setup and verification
- [`docs/archive/`](docs/archive/README.md): closed plans and verification evidence

## License

MIT. Bundled fonts are SIL OFL 1.1; see
[`src/ui/fonts/THIRD_PARTY_NOTICES.md`](src/ui/fonts/THIRD_PARTY_NOTICES.md).
The converter's runtime notices are in
[`backend/THIRD_PARTY_NOTICES.md`](backend/THIRD_PARTY_NOTICES.md).
