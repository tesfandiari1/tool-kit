# Design system port, 2026-09-12

`apps/desktop/src/ui` was rebuilt on the Tristin Esfandiari Design System. This
is the record of what shipped and why, so the next agent does not re-litigate
it. The source export lived in `design/` until commit `e636d2f`:
`design/tristin-ds/` held the token CSS, the component sources and `readme.md`,
which states every rule. Read it with `git show e636d2f:design/tristin-ds/readme.md`.

## The system

Three colours: slate `#69809C`, ink `#2B2B2B` for text and rules, bone
`#EFE3D6` for the ground, each with steps. Radius 0, no shadow, no gradient, no
blur. Rectangles bounded by 1px rules, and neighbours share a rule. TRJN
DaVinci for display at one weight, Red Hat Display 500 tracked caps for labels
and buttons, DM Sans for body. Muted status colours, never traffic lights.
Hover is the only motion, 180ms.

## Decisions as shipped

1. **Token names are the system's own.** The old vocabulary was deleted, not
   aliased, so a stale name fails the gate instead of rendering the wrong
   colour.
2. **The theme follows the macOS appearance.** One role block using
   `light-dark()` on `color-scheme: light dark`, bone in light and ink in dark.
   There is no second theme block to keep in sync. `[data-theme="bone"|"ink"]`
   forces a side, and only the gallery sets it.
3. **Four faces, four jobs.** DaVinci headings, Red Hat Display labels and
   meta, DM Sans body, `ui-monospace` in the source editor and in `.md
   code/pre`. No Google Fonts import: the CSP is `font-src 'self'`.
4. **`Mono` became `Meta`.** The face is no longer monospace and the old name
   would lie.
5. **Button variants** are `primary | secondary | ghost`, default
   `secondary`. Primary is a solid the Run button needs, which the system has
   only as an outline on dark. `busy` is an outline in `--status-warning`.
6. **Badge tones** are `neutral | info | success | warning | danger`, and
   `square` became `count`. A 6px `currentColor` square appears only under a
   status tone, because a neutral swatch is decoration.
7. **`Toast` is a primitive**, not an App.css composite, because the gallery
   needs a specimen and the sheet overlay needs the same bar.
8. **`Input` absorbed `TextInput`.** `Input` takes optional `label`, `hint` and
   `error` and wraps itself in `Field` when one is given. Two names for one
   control was the whole defect.
9. **Cascade layers replace import order.** `@layer ui, app;` is declared in
   both `base.css` and `App.css`, so an app override wins whatever order the
   bundler picks. `Gallery.css` is deliberately unlayered.
10. **`scripts/lint-tokens.sh` is the guard**, wired into `pnpm lint`. It fails
    on a hex colour, `rgba(`, `box-shadow`, `border-radius`, `backdrop-filter`,
    a literal `font-family:` and every deleted token name, anywhere under
    `src/` except `ui/tokens.css` and `ui/fonts.css`.
11. **Status colours are lifted on ink.** `--status-danger` `#8C4A48` on ink is
    2.15:1, so each status token takes a lighter step on the dark side.
12. **Every ticking figure sits on Red Hat Display** with
    `font-variant-numeric: tabular-nums`. Measured with fonttools: Red Hat
    Display has `tnum`, DM Sans does not and its digits are proportional (`1`
    342 units against `0` 656), so a DM Sans timer jitters.

### Invented tokens

The system is a website system with no dark side, so five roles had no
precedent. Each carries a one-line comment in `tokens.css`.

| Token | Why |
|---|---|
| `--ink-lift`, `--ink-sunk` | the dark ground is one flat ink and needs a raised and a sunk step |
| `--text-ghost` | UI.md rule 2 needs a slot that is held but not read |
| `--selection-text` | the system sets only the wash, and ink on `--blue-deep` is unreadable |
| `--scrim` | a modal needs a ground that reads as behind, on both sides |
| `--toast-bg`, `--toast-fg` | the toast is a dark bar in both themes, and on ink it needs a lifted step or it vanishes |

## Token map

Left column is gone. Nothing under `src/` may name it.

| Old | New |
|---|---|
| `--surface`, `--surface-solid` | `--surface-page` |
| `--surface-raised` | `--surface-lift` |
| `--surface-well` | `--surface-sunk` |
| `--surface-scrim` | `--scrim` |
| `--ink` | `--text-body` |
| `--ink-2` | `--text-soft` |
| `--ink-3` | `--text-mute` |
| `--ink-ghost` | `--text-ghost` |
| `--rule-lit` | `--rule` |
| `--rule-strong` | deleted, hover rule steps are gone |
| `--focus` | `--focus-ring` |
| `--select` | `--selection` |
| `--live`, `--pass`, `--fault` | `--status-warning`, `--status-success`, `--status-danger` |
| `--accent` | `--btn-primary-bg` on a fill, `--text-body` on a mark, `--focus-ring` on the caret |
| `--accent-hover` | `--btn-primary-hover-bg` or `--link-hover` |
| `--accent-ink` | `--btn-primary-fg` |
| `--accent-quiet` | `--link-hover` |
| `--weight-medium`, `--weight-normal` | `--weight-subhead`, or nothing |
| `--font-mono` | `--font-subhead` on a label, `--font-code` in the editor |
| `--track-mono` | `--track-label` |
| `--r-control`, `--r-panel`, `--r-pill` | deleted, radius is 0 |
| `--warm-*`, `--cobalt-*`, `--amber`, `--green-*`, `--red-500` | deleted, the raw palette is `--blue/--ink/--bone` |

## Component map

| System | `@ui` | What changed |
|---|---|---|
| Button, IconButton | `Button`, `iconOnly` | four variants, caps on `--font-subhead`, radius 0, one press rule |
| Rule | `Divider` | `--rule`, soft variant on `--rule-soft` |
| SectionLabel, FieldLabel | `Label`, `Field` | one shared caps rule in `Text.css`, `LabelTone` is `default \| strong` |
| Input, Select, Switch | `Input`, `Select`, `Switch` | radius 0, square switch track and thumb, `Input` absorbed `TextInput` |
| Radio, RadioGroup | `Segmented` | ruled rectangles sharing a left rule, selected inverts |
| Tabs | `Tabs` | text on one rule, active `::after` 2px `--text-body` |
| Card | `Panel`, `Well` | 1px rule, no fill, no radius; `PanelTone` deleted |
| Badge | `Badge`, `StatusDot` | five tones, `count`, 6px square marks |
| Dialog | `Sheet` | `--scrim`, `--surface-lift` card, no radius, no entrance keyframes |
| Toast | `Toast` | a primitive, `role="status"`, 4px status edge on the left |
| Meter | `Meter` | `--rule-soft` track, `--status-warning` fill, square ends |
| Tooltip, Tag, Checkbox, Wordmark, PhotoBand | none | add when a view asks |

## Fonts

| Family | Role | File |
|---|---|---|
| TRJN DaVinci Text | display | `src/ui/fonts/TRJN-DaVinci-Text.woff2` |
| Red Hat Display | labels, buttons, meta, every ticking figure | `src/ui/fonts/RedHatDisplay-Variable.ttf` |
| DM Sans | body and prose | `src/ui/fonts/DMSans-Variable.ttf` |
| `ui-monospace` | source editor, `.md code/pre` | system face, no file |

Instrument Serif and JetBrains Mono were removed. The variable faces ship as
TTFs; woff2 conversion is a later optimisation. `THIRD_PARTY_NOTICES.md` lists
the two OFL families, and `verify-release.sh` still finds both notice files at
their old paths.
