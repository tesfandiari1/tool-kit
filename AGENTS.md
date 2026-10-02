## Learned User Preferences

- Steal host, folder, and onboarding patterns from Yaak, OpenWork, Apple Setup Assistant, and Obsidian; do not fork those products or copy Electron, Tailwind/shadcn, Zustand, or TanStack Query/Router.
- Product target is a local markdown workspace editor (library-first), not a batch converter; the title-bar nav is Library/Run/History and Settings is a sheet over the window at ⌘,; follow `docs/north-star.md`.
- Workspace root plus projects as folders (Claude/ChatGPT desktop pattern); documents are durable in the project on import; export is copy-out only, never move.
- No backend retention of customer files; conversion uses ephemeral local workers and discards scratch after each run; local vs cloud conversion mode is user-configurable in onboarding and settings.
- Conversion HTTP and secrets stay in Tauri; never browser-fetch the remote service or put the bearer token in the webview.
- Prefer type-aware ESLint (`strictTypeChecked`, exhaustive switches, official React hooks) and Vitest on plan/billing math over Biome, Prettier, Playwright, or coverage theater.
- Stage explicit paths only; never `git add .` or `git add -A`. Parallel sessions start from `docs/STATUS.md`.
- Stay on latest stable dependencies except the documented pins; do not bump TypeScript 7 just to be current.
- Operational milestones in `docs/STATUS.md`; product target in `docs/north-star.md`. Do not add separate phase execution-plan docs unless they encode critical patterns.
- One window phase. The window opens at `tauri.conf.json`'s size, grows once into `ONBOARDING` or `WORKSPACE`, and is then resizable inside `WORKSPACE`'s bounds at the size the user last left it. Every number lives in `src/shell/geometry.ts`.
- Keep job UX progress (Tauri poll → `job-updated`) separate from operator monitoring. M7 carried the metrics work and is cancelled, but the split still holds. See `docs/archive/MONITORING_AND_PROGRESS.md`.
- Use Clippy with a small production deny-set and test exceptions in each crate's `clippy.toml`. Do not enable `pedantic` or blanket `unwrap_used`.

## Learned Workspace Facts

- Frontend layout, under `apps/desktop/`: `src/app` (types, commands, OpenAPI client), `src/platform/host.ts` (only Tauri/host imports), `src/domains/{library,onboarding,run,settings,history,thread}`, `src/shell` (App + its hooks), `src/ui` (design system); workspace commands in `src-tauri/src/workspace.rs`.
- `pnpm generate:api` writes `apps/desktop/src/app/api/schema.ts` from `contract/http/openapi.yaml`; import the generated schema only from `src/app/api`.
- Repo layout: `apps/{desktop,converter}`, `crates/worker-protocol`, `workers/{vision,audio}` (Swift, macOS only), `contract/http`, `deploy/docker`. The image builds from the repo root because the converter path-depends on `crates/worker-protocol`.
- Conversion backend lives in `apps/converter/`. Milestones and session state live in `docs/STATUS.md`; product direction in `docs/north-star.md`. Desktop HTTP to that service is M6. Backend sessions own `apps/converter/**`. Desktop sessions own `apps/desktop/**`.
- `docs/north-star.md` defines the local-first pivot: workspace folder, projects as real directories, `.toolkit/index.db` as rebuildable index, tool-run provenance, no durable backend job retention in product mode.
- AnyDoc is a Rust crate inside the single converter container, not a separate service. PDF stays on the isolated `pdf-inspector` child worker.
- Toolchain pins live in `rust-toolchain.toml`, `.node-version`, and `package.json` `packageManager`. TypeScript stays on 6.x (7 has no compiler API and breaks typescript-eslint). `security-framework` needs the non-default `OSX_10_15` feature. `pdf-inspector` stays `=1.25.2`. Do not take `libc` 1.0. Do not create a root Cargo workspace.
- CI and weekly Dependabot cover npm, both Cargo crates, and Actions. Desktop clippy/tests run on macOS because of `macos-private-api` and `security-framework`.
- `pnpm check` runs `tsc --noEmit`, ESLint, and Vitest; CI runs `pnpm test` before `pnpm build`.
