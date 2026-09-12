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

cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings
cargo clippy --manifest-path apps/converter/Cargo.toml --all-targets -- -D warnings
```

See [`docs/STATUS.md`](docs/STATUS.md) for session state and
[`CLAUDE.md`](CLAUDE.md) for desktop architecture, live API smokes, and
signing details. Weekly Dependabot keeps npm, both Cargo crates, and Actions
current; do not jump to TypeScript 7.

## Test

```bash
pnpm lint && pnpm build
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib
cargo test --locked --manifest-path apps/converter/Cargo.toml
```

Three gates come from outside the repo, so they check work nobody here wrote
the assertions for:

```bash
pnpm lint:api        # Spectral, ~60 OpenAPI rules over the contract
pnpm verify:deps     # cargo-deny, the RustSec advisory database and licenses
pnpm verify:contract # Schemathesis, property-based testing of the live service
```

`verify:contract` builds the backend, starts it on a throwaway port and data
root, seeds one conversion, and derives its cases from
`contract/http/openapi.yaml`. It needs `uv` and Docker is not involved.

Ignored desktop tests in `apps/desktop/src-tauri/src/live_smoke.rs` hit real Datalab and
Rev.ai endpoints and spend API credits. CI runs all of the above on every push
and pull request.

## Release

macOS `app` + `dmg` only, Apple Silicon only, macOS 26 or later. Sign with a
Developer ID, and notarize if anyone else will download it.

```bash
# sign only; fine for a local install, locally-built apps are not quarantined
APPLE_SIGNING_IDENTITY="Developer ID Application: …" pnpm tauri build

# sign + notarize: additionally export APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID
pnpm verify:release --notarized
```

Artifacts land in `apps/desktop/src-tauri/target/release/bundle/`. Signing credentials live
in `.env.local` (gitignored).

**`pnpm tauri build` notarizes the `.app` and not the DMG.** It staples the app
and merely signs the DMG, so a *downloaded* DMG is still refused by Gatekeeper
while a local install works. Finish it by hand:

```bash
xcrun notarytool submit <dmg> --apple-id … --password … --team-id … --wait
xcrun stapler staple <dmg>
```

Do not skip the verify step. A build with no signing identity still succeeds
and still produces a working `.app`, but it is ad-hoc signed: Gatekeeper on any
other Mac rejects it and notarization will not touch it. `verify-release.sh`
fails on exactly that, plus the team clause in the designated requirement, the
Info.plist keys, the bundled font licence, and the keychain entitlement. Add
`--notarized` to also check both stapled tickets and Gatekeeper.

### Keychain

API keys live in the macOS **data protection** keychain, which has no access
dialogs: reads are authorised by the `keychain-access-groups` entitlement and
matched on team id rather than by a per-item ACL bound to the code signature.

That entitlement is restricted, so it only works when
`apps/desktop/src-tauri/embedded.provisionprofile` is present to authorise it. Without the
profile the app falls back to the legacy keychain, which still works but ties
access to the signature. A bare `cargo run` binary can never carry the profile,
so the dev loop always uses the fallback and keeps its own copy of each key.

## Conversion backend

The production conversion service lives in [`apps/converter/`](apps/converter/). It converts
native-text PDFs and supported office formats on loopback. M4 routing policy and M6 desktop integration are landed. Backend Datalab
fallback is Phase 2. LAN deploy is Phase 3. See [`docs/STATUS.md`](docs/STATUS.md).

- [`docs/STATUS.md`](docs/STATUS.md): state, critical path, milestones, traps, verify commands
- [`apps/converter/README.md`](apps/converter/README.md): backend setup and verification
- [`docs/archive/`](docs/archive/README.md): closed plans and verification evidence

## License

MIT. Red Hat Display and DM Sans are SIL OFL 1.1; TRJN DaVinci is licensed. See
[`apps/desktop/src/ui/fonts/THIRD_PARTY_NOTICES.md`](apps/desktop/src/ui/fonts/THIRD_PARTY_NOTICES.md).
The converter's runtime notices are in
[`apps/converter/THIRD_PARTY_NOTICES.md`](apps/converter/THIRD_PARTY_NOTICES.md).
