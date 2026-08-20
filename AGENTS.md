## Learned User Preferences

- Steal host and folder patterns from Yaak and OpenWork; do not fork those products or copy Electron, Tailwind/shadcn, Zustand, or TanStack Query/Router.
- Product target is a local markdown workspace editor (library-first), not a batch converter; follow `docs/north-star.md`.
- Workspace root plus projects as folders (Claude/ChatGPT desktop pattern); documents are durable in the project on import; export is copy-out only, never move.
- No backend retention of customer files; conversion uses ephemeral local workers and discards scratch after each run.
- Conversion HTTP and secrets stay in Tauri; never browser-fetch the remote service or put the bearer token in the webview.
- Prefer type-aware ESLint (`strictTypeChecked`, exhaustive switches, official React hooks) and Vitest on plan/billing math over Biome, Prettier, Playwright, or coverage theater.
- Stage explicit paths only; never `git add .` or `git add -A`. Parallel sessions start from `docs/STATUS.md`.
- Stay on latest stable dependencies except the documented pins; do not bump TypeScript 7 just to be current.
- Operational milestones in `docs/STATUS.md`; product target in `docs/north-star.md`. Do not add separate phase execution-plan docs unless they encode critical patterns.
- Launcher window should fit content height (`useFitWindow`); no page scroll in the compact launcher phase.
- Keep job UX progress (Tauri poll → `job-updated`) separate from operator monitoring. M7 carried the metrics work and is cancelled, but the split still holds. See `docs/archive/MONITORING_AND_PROGRESS.md`.
- Use Clippy with a small production deny-set and test exceptions in each crate's `clippy.toml`. Do not enable `pedantic` or blanket `unwrap_used`.

## Learned Workspace Facts

- Frontend layout, under `apps/desktop/`: `src/app` (types, commands, OpenAPI client), `src/platform/host.ts` (only Tauri/host imports), `src/domains/{run,settings,history,thread}`, `src/shell` (App + its hooks), `src/ui` (design system).
- `pnpm generate:api` writes `apps/desktop/src/app/api/schema.ts` from `contract/http/openapi.yaml`; import the generated schema only from `src/app/api`.
- Repo layout: `apps/{desktop,converter}`, `crates/worker-protocol`, `workers/vision` (Swift, macOS only), `contract/http`, `deploy/docker`. The image builds from the repo root because the converter path-depends on `crates/worker-protocol`.
- Conversion backend lives in `apps/converter/`. Milestones and session state live in `docs/STATUS.md`; product direction in `docs/north-star.md`. Desktop HTTP to that service is M6. Backend sessions own `apps/converter/**`. Desktop sessions own `apps/desktop/**`.
- `docs/north-star.md` defines the local-first pivot: workspace folder, projects as real directories, `.toolkit/index.db` as rebuildable index, tool-run provenance, no durable backend job retention in product mode.
- AnyDoc is a Rust crate inside the single converter container, not a separate service. PDF stays on the isolated `pdf-inspector` child worker.
- Toolchain pins live in `rust-toolchain.toml`, `.node-version`, and `package.json` `packageManager`. TypeScript stays on 6.x (7 has no compiler API and breaks typescript-eslint). `security-framework` needs the non-default `OSX_10_15` feature. `pdf-inspector` stays `=1.15.0`. Do not take `libc` 1.0. Do not create a root Cargo workspace.
- CI and weekly Dependabot cover npm, both Cargo crates, and Actions. Desktop clippy/tests run on macOS because of `macos-private-api` and `security-framework`.
- `pnpm check` runs `tsc --noEmit`, ESLint, and Vitest; CI runs `pnpm test` before `pnpm build`.

<!-- gitnexus:start -->
# GitNexus: code intelligence

This project is indexed by GitNexus as **tool-kit** (3393 symbols, 9474 relationships, 290 execution flows). Use the GitNexus MCP tools to understand code, assess impact, and navigate safely.

> Index stale? Run `node .gitnexus/run.cjs analyze` from the project root. It auto-selects an available runner. No `.gitnexus/run.cjs` yet? `npx gitnexus analyze` (npm 11 crash → `npm i -g gitnexus`; #1939).

## Always Do

- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `detect_changes()` before committing** to verify your changes only affect expected symbols and execution flows. For regression review, compare against the default branch: `detect_changes({scope: "compare", base_ref: "main"})`.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `query({search_query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol (callers, callees, and which execution flows it participates in), use `context({name: "symbolName"})`.
- For security review, `explain({target: "fileOrSymbol"})` lists taint findings (source→sink flows; needs `analyze --pdg`).

## Never Do

- NEVER edit a function, class, or method without first running `impact` on it.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis.
- NEVER rename symbols with find-and-replace. Use `rename`, which understands the call graph.
- NEVER commit changes without running `detect_changes()` to check affected scope.

## Resources

| Resource | Use for |
|----------|---------|
| `gitnexus://repo/tool-kit/context` | Codebase overview, check index freshness |
| `gitnexus://repo/tool-kit/clusters` | All functional areas |
| `gitnexus://repo/tool-kit/processes` | All execution flows |
| `gitnexus://repo/tool-kit/process/{name}` | Step-by-step execution trace |

## CLI

| Task | Read this skill file |
|------|---------------------|
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus/gitnexus-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->
