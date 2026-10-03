# Tool-Kit UI

The design system. One import path, one token layer, one specimen page.

```tsx
import { Button, Panel, Label, Stack } from "@ui";
```

Run `pnpm dev` and open `http://localhost:1420/?gallery` to see every primitive
in every state, on either side of the theme.

## The rule that makes this a library

**Nothing under `src/ui` may import from the app.** No `@/domains`, no `@/app`,
no `@/platform`, no `@tauri-apps/*`. ESLint enforces it.

The library knows about React and CSS. It does not know that Tool-Kit converts
documents, that jobs have five statuses, or that a Tauri bridge exists. When a
component needs something from the product, it takes it as a prop.

`StatusDot` is the worked example. It could have imported the app's `Status`
union and switched on it. Instead it declares its own `DotTone`, and the caller
maps `"processing" -> "live"`. One line at the call site buys a library that
compiles with the app deleted.

That is the point. The day a web client or a second app appears, `src/ui`
becomes `packages/ui` by moving the folder and changing one alias in
`vite.config.ts`. Anything that reaches into the app breaks that, silently, and
you find out months later.

## The design language

This is the Tristin Esfandiari system, retuned for a macOS instrument panel.
Three colours, radius 0, no shadow, no gradient, no blur. Rules do the work
that a shadow would: a 1px rule bounds a rectangle, and neighbours share one
rule rather than each drawing their own. Four rules decide every question:

**1. Colour is signal, never decoration.** `--status-warning` is live,
`--status-success` is passed, `--status-danger` is failed. The control you
press is an ink fill, not a hue. Slate is a ground and a hover, never a
highlight. Icons, brand marks, and folder glyphs are never coloured. If a
colour is not carrying one of those meanings, it is wrong.

**2. Values light up, they don't appear.** Counts and timers hold their slot as
`tone="ghost"` glyphs and brighten when they carry meaning. `StatusDot` occupies
the same box at every status. A window with twenty jobs finishing out of order
must never reflow.

**3. macOS first.** Opaque surfaces that follow the macOS appearance, native
`<select>` and `<input type=checkbox>` underneath the restyled shells so the
platform's keyboard and VoiceOver behaviour survives, tabular figures wherever
a number ticks, dimming when the window loses key, and one focus ring: a 1px
`--focus-ring` outline at `3px` offset.

**4. Whitespace is a grammar, not a feel.** The 4px scale is in `tokens.css`.
Which step means what is here, and a wrong step reads as a bug rather than as a
taste difference. The gap is inversely proportional to the relationship:

| Step | px | Used for |
|---|---|---|
| `--s1` | 4 | atoms of one object: title to subtitle, dot to count |
| `--s2` | 8 | parts of one control: icon to label, two adjacent buttons |
| `--s3` | 12 | items in a list: queue rows, drop-well items, groups in a panel |
| `--s4` | 16 | panel body padding, panel to panel |
| `--s5` | 24 | column to window edge |
| `--s6` | 32 | empty-state optical padding only |
| `--s7` `--s8` | 48 64 | not used in chrome at all |

**Indent is a rung, not a new value.** `Tree` steps `--s3` a level, which is
both the "items in a list" rung and AppKit's measured `indentationPerLevel`, and
it stops at four:
`calc(var(--s2) + var(--s3) * min(var(--tree-depth), 4))`. Uncapped, a deep
folder walks its own name off the left pane's 240px floor, with no error
anywhere to say so.

**The outer margin is the largest gap on screen.** A column whose edge inset
equals its internal gaps has no frame and reads as content spilling to the
glass, so nested containers step *down* the ladder, never up. `.flow` insets at
`--s5` and gaps at `--s4`, and matching the two was the single biggest reason
the window used to read as cramped.

**Air is content-driven, never leftover.** Exactly one element per scroll column
may absorb slack, which in the run column is the queue. Anything else that
stretches manufactures a void, and a void inside a panel border reads as
"something failed to load" rather than as air. A reserved slot is not a void,
but it belongs between panels: `.job-msg` holds two lines open below the Job
panel, where the space a resolving scan will fill is indistinguishable from
column air. Inside the border the same 30px is a bug report.

### Typography

Three faces, four jobs, no overlap. Getting this right matters more than any
colour.

| Face | Component | Job |
|---|---|---|
| Red Hat Display 500, uppercase, `0.22em` | `<Label>` | Every section heading, tab, field label, and button |
| TRJN DaVinci, `0.01em` | `<Display>` | Panel titles and headline numbers, `--t-lg` and up |
| DM Sans | `<Text>` | All prose |
| Red Hat Display, tabular figures | `<Meta>` | Paths, counts, timers, identifiers |

The tracked caps label is the signature. It carries more of the identity than
the palette does. `<Display>` deliberately has no size below `lg`: DaVinci is
drawn for display sizes and muddies at UI scale.

**DM Sans has no `tnum`**, and its digits are proportional, so a figure set in
it shifts every time it ticks. Every number that changes on screen goes on
`<Meta>` or another `--font-subhead` element with
`font-variant-numeric: tabular-nums`. Prose keeps DM Sans.

### Themes

Two sides, one declaration. Every role token is a `light-dark()` pair in
`tokens.css`, so `color-scheme` decides which side a value resolves to and no
token is written twice. The app sets `color-scheme: light dark` and follows the
macOS appearance: **bone** in light, **ink** in dark.

`[data-theme="bone"]` and `[data-theme="ink"]` force a side by setting
`color-scheme` alone. That is for review, in the gallery. The app never sets
it.

## Files

```
src/ui/
  base.css            reset, element defaults, focus ring, .ui-selectable
  tokens.css          every colour, size, face, space, and duration
  fonts.css           @font-face for the three self-hosted families
  cx.ts               class-name join
  index.ts            the entire public surface
  primitives/         grouped by concept, not one file per component
    Text              Label · Display · Text · Meta
    Path              Path (a file path as crumbs)
    Button            Button
    Surface           Panel · Well · Divider
    Badge             Badge · StatusDot
    Field             Field · Input · Select · Switch
    Layout            Stack · Row · Spacer
    Meter             Meter
    Segmented         Segmented
    Sheet             Sheet (a modal card on a native <dialog>)
    Toast             Toast (a status bar at the foot of the window)
    Tree              Tree · TreeRow, with treeKeys.ts beside them
  gallery/            the specimen page
scripts/lint-tokens.sh  the token gate, run by `pnpm lint`
```

Primitives are grouped by concept because they are read and changed together.
Each `.tsx` imports its own `.css`.

`Input` takes an optional `label` and `hint` and wraps itself in `Field` when
either is given, so most settings need one element. Pass neither when the
control shares a row with something else and its own `Field` would force it
onto a line of its own.

**Stacked panels share a rule.** Two panels in a column each draw a 1px border,
which reads as a 2px seam. The call site pulls the second up with
`margin-block-start: -1px`.

## Loading order

Cascade layers decide, not import order. `@layer ui, app;` is declared at the
top of both `base.css` and `App.css`, every file under `primitives/` wraps its
rules in `@layer ui`, and `App.css` wraps its own in `@layer app`. An app rule
therefore beats a library rule of any specificity, whichever file Vite emits
first. `Gallery.css` is deliberately unlayered, so the specimen page wins over
both.

`src/shell/App.css` still does **not** `@import "../ui/base.css"`. `index.ts`
already loads it, and importing from both places makes Vite emit the whole token
layer into two chunks with two competing `:root` blocks.

## Extending it

1. **Reach for a token before writing a value.** A literal colour or a `7px` in
   a component is the bug. If no token fits, add one to `tokens.css` and say
   what it means.
2. **Add the specimen in the same commit.** A primitive with no entry in
   `gallery/Gallery.tsx` cannot be reviewed, so it does not exist.
3. **Check all three theme settings** before calling it done: system, bone and
   ink. The switch is in the gallery's top bar.
4. **Spacing comes from `--s1`..`--s8` only**, on the grammar above. The 4px
   grid has one exception, and it is not a spacing value: chrome measured
   against something macOS draws itself. The traffic lights sit at a fixed
   logical offset and do not zoom with the page, so `.bar-lights` reserves
   `calc(64px / var(--zoom, 1))` and `.bar` divides its top inset the same way.
   `useZoom` publishes `--zoom` on `:root`. Nothing else may hold a literal.
5. **Keep native elements underneath.** A restyled `<select>` keeps type-ahead
   and VoiceOver for free. A div pretending to be one does not.
6. **`pnpm lint:tokens` is the guard**, and it runs inside `pnpm lint`. It fails
   on a hex colour, an `rgba(`, a `box-shadow`, a `border-radius`, a
   `backdrop-filter`, a literal font family, or a deleted token name anywhere
   under `src/`. Only `tokens.css` and `fonts.css` are exempt.

## What is deliberately absent

No Tailwind, no Radix, no CVA, no `clsx`, no `tailwind-merge`. The look is a
token layer and a type ramp, and none of those libraries supply either. Adding
them would mean a build-system change to restyle a dozen components we already
own.

**No tree library, and three lost the argument.** `Tree` is hand-rolled.
react-arborist sets `aria-expanded` on leaves, which makes every file announce
as a folder nobody has opened, and it brings its own transitive dependencies.
react-complex-tree owns the render tree and its own custom properties, so the
token layer stops deciding how a row looks. react-aria-components is Apache-2.0
against this package's MIT, and it emits treegrid roles, which changes what
VoiceOver says about every row. What the library actually needed was list
navigation, and this repo had already hand-rolled a roving tabindex twice, in
`RunView` and in `Tabs`. The keyboard rules live in `treeKeys.ts` and are tested
there.

**No dialog library either.** `Sheet` is a native `<dialog>` with
`showModal()`, which gives the focus trap, Escape through `cancel`, inertness,
and the top layer with no dependency at all.

If accessible overlay behaviour is needed later (a real combobox, a listbox that
has to float), the cheapest correct answer is **Base UI** or **React Aria
Components**: both are headless, both work with this CSS, and neither drags a
utility framework in behind it. That escape hatch is for overlays. It is not a
reason to import list navigation.
