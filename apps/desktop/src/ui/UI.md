# Tool-Kit UI

The design system. One import path, one token layer, one specimen page.

```tsx
import { Button, Panel, Label, Stack } from "@ui";
```

Run `pnpm dev` and open `http://localhost:1420/?gallery` to see every primitive
in every state, in both themes.

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

Derived from AssemblyAI's published theme, retuned for a macOS instrument
panel. Four rules decide every question:

**1. Colour is signal, never decoration.** Amber is live, green is passed, red
is failed, cobalt is the control you press. Icons, brand marks, and folder
glyphs are never coloured. If a colour is not carrying one of those five
meanings, it is wrong.

**2. Values light up, they don't appear.** Counts and timers hold their slot as
`tone="ghost"` glyphs and brighten when they carry meaning. `StatusDot` occupies
the same box at every status. A window with twenty jobs finishing out of order
must never reflow.

**3. macOS first.** Real vibrancy under a scrim, SF metrics for prose, HIG focus
rings, tabular numerals wherever a number ticks, and native `<select>` and
`<input type=checkbox>` underneath the restyled shells so the platform's
keyboard and VoiceOver behaviour survives.

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
| `--s7` `--s8` | 40 48 | not used in chrome at all |

**Indent is a rung, not a new value.** `Tree` steps `--s3` a level, which is
both the "items in a list" rung and AppKit's measured `indentationPerLevel`, and
it stops at four:
`calc(var(--s2) + var(--s3) * min(var(--tree-depth), 4))`. Uncapped, a deep
folder walks its own name off the left pane's 300px floor, with no error
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

Three families, three jobs, no overlap. Getting this right matters more than
any colour.

| Family | Component | Job |
|---|---|---|
| JetBrains Mono, uppercase, `0.09em` | `<Label>` | Every section heading, tab, field label, and button |
| Instrument Serif, `-0.02em` | `<Display>` | Panel titles and headline numbers, `--t-lg` and up |
| SF Pro | `<Text>` | All prose |
| JetBrains Mono | `<Mono>` | Paths, counts, timers, identifiers |

The mono uppercase label is the signature. It carries more of the identity than
the palette does. `<Display>` deliberately has no size below `lg`: Instrument
Serif is drawn for display sizes and muddies at UI scale.

### Themes

`:root` is **graphite**, the warm charcoal desktop default.
`[data-theme="paper"]` is **paper**, AssemblyAI's cream, for any future web
surface. Only surfaces, ink, and accent contrast differ. Type, space, radius,
and motion are shared, so a component styled once works in both.

## Files

```
src/ui/
  base.css            reset, element defaults, focus ring, .ui-selectable
  tokens.css          every colour, size, font, radius, and duration
  fonts.css           @font-face for the two self-hosted OFL families
  cx.ts               class-name join
  index.ts            the entire public surface
  primitives/         grouped by concept, not one file per component
    Text              Label · Display · Text · Mono
    Path              Path (a file path as crumbs)
    Button            Button
    Surface           Panel · CellGrid · Cell · Well · Divider
    Badge             Badge · Status · StatusDot
    Field             Field · Input · TextInput · Select · Switch
    Layout            Stack · Row · Spacer
    Meter             Meter
    Segmented         Segmented
    Disclosure        Disclosure
    Sheet             Sheet (a modal card on a native <dialog>)
    Tree              Tree · TreeRow, with treeKeys.ts beside them
  gallery/            the specimen page
```

Primitives are grouped by concept because they are read and changed together.
Each `.tsx` imports its own `.css`.

### Two pairs that are easy to confuse

**`Status` vs `StatusDot`.** `StatusDot` draws its own dot. `Status` is a
fixed-width slot that tints whatever glyph you put in it. Use `Status` wherever
the glyph's *shape* carries meaning the colour cannot, such as a spinner for
work in flight. `RunView` uses `Status` for exactly that reason.

**`Input` vs `TextInput`.** `TextInput` is `Field` + `Input`, which is what most
settings want. Reach for the bare `Input` when the control shares a row with
something else (a Save button, a unit suffix) and `TextInput`'s built-in `Field`
would force it onto its own line.

## Loading order

`src/shell/App.css` deliberately does **not** `@import "../ui/base.css"`. `index.ts`
already loads it, and importing from both places makes Vite emit the whole 16KB
token layer into two chunks with two competing `:root` blocks.

That means `App.tsx` must import `@ui` **before** `./App.css`, so the library's
`:root` lands first and the app's overrides win. Keep that order.

## Extending it

1. **Reach for a token before writing a value.** A literal colour or a `7px` in
   a component is the bug. If no token fits, add one to `tokens.css` and say
   what it means.
2. **Add the specimen in the same commit.** A primitive with no entry in
   `gallery/Gallery.tsx` cannot be reviewed, so it does not exist.
3. **Check both themes** before calling it done. The toggle is in the gallery's
   top bar.
4. **Spacing comes from `--s1`..`--s8` only**, on the grammar above. The 4px
   grid has one exception, and it is not a spacing value: chrome measured
   against something macOS draws itself. The traffic lights sit at a fixed
   logical offset and do not zoom with the page, so `.bar-lights` reserves
   `calc(64px / var(--zoom, 1))` and `.bar` divides its top inset the same way.
   `useZoom` publishes `--zoom` on `:root`. Nothing else may hold a literal.
5. **Keep native elements underneath.** A restyled `<select>` keeps type-ahead
   and VoiceOver for free. A div pretending to be one does not.

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
