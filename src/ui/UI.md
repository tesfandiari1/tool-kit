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
panel. Three rules decide every question:

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
    Button            Button
    Surface           Panel · CellGrid · Cell · Well · Divider
    Badge             Badge · Status · StatusDot
    Field             Field · Input · TextInput · Select · Switch
    Layout            Stack · Row · Spacer
    Meter             Meter
    Segmented         Segmented
    Disclosure        Disclosure
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
4. **Spacing comes from `--s1`..`--s8` only.** The 4px grid has no exceptions.
5. **Keep native elements underneath.** A restyled `<select>` keeps type-ahead
   and VoiceOver for free. A div pretending to be one does not.

## What is deliberately absent

No Tailwind, no Radix, no CVA, no `clsx`, no `tailwind-merge`. The look is a
token layer and a type ramp, and none of those libraries supply either. Adding
them would mean a build-system change to restyle a dozen components we already
own.

If accessible overlay behaviour is needed later (a real combobox, a modal with
focus trapping), the cheapest correct answer is **Base UI** or **React Aria
Components**: both are headless, both work with this CSS, and neither drags a
utility framework in behind it.
