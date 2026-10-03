# Tool-Kit

A macOS menu-bar utility that turns files into AI-ready text. Drop files or
folders, hit Run, get markdown or transcripts back in your project. The
originals stay where they are and are never copied.

**Current work:** local same-machine closeout. Start at
[`docs/STATUS.md`](docs/STATUS.md).

## Current desktop implementation

| Job | Engine | In → Out |
|---|---|---|
| Convert | Local converter: pdf-inspector, AnyDoc, Vision OCR | PDF / DOCX / PPTX / XLSX / images / … → Markdown |
| Transcribe | Local worker (SpeechAnalyzer + FluidAudio) | audio / video → Markdown transcript |

Everything runs on the Mac. The app spawns the converter as a loopback sidecar,
and no file content leaves the machine. The job is chosen from what you drop.
Results land in the active project, and a dropped folder keeps its shape there.
A Settings toggle moves dropped files into the project instead of leaving them
in place.

## Develop

Requires Node 22.12+ (24 preferred), pnpm 11, and Rust 1.99. The repo pins
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
[`CLAUDE.md`](CLAUDE.md) for desktop architecture and signing details. Weekly Dependabot keeps npm, both Cargo crates, and Actions
current; do not jump to TypeScript 7.

## Test

```bash
pnpm lint && pnpm build
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib
cargo test --locked --manifest-path apps/converter/Cargo.toml
```

```bash
pnpm verify:deps     # cargo-deny, the RustSec advisory database and licenses
```

`apps/converter/tests/http_contract.rs` is the HTTP contract. CI runs all of the
above on every push and pull request.

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
sidecar signatures, the Info.plist keys and the bundled licences. Add
`--notarized` to also check both stapled tickets and Gatekeeper.

## Conversion backend

The conversion service lives in [`apps/converter/`](apps/converter/). The app
spawns it on loopback, and it is the only service. See
[`docs/STATUS.md`](docs/STATUS.md).

- [`docs/STATUS.md`](docs/STATUS.md): state, critical path, milestones, traps, verify commands
- [`apps/converter/README.md`](apps/converter/README.md): backend setup and verification
- [`docs/archive/`](docs/archive/README.md): closed plans and verification evidence

## License

MIT. Red Hat Display and DM Sans are SIL OFL 1.1; TRJN DaVinci is licensed. See
[`apps/desktop/src/ui/fonts/THIRD_PARTY_NOTICES.md`](apps/desktop/src/ui/fonts/THIRD_PARTY_NOTICES.md).
The converter's runtime notices are in
[`apps/converter/THIRD_PARTY_NOTICES.md`](apps/converter/THIRD_PARTY_NOTICES.md).
