# Tristin Esfandiari Design System

Brand and UI system for **Tristin Esfandiari Consulting** (esfandiari.dev). Tristin is a business systems and transformation consultant: an operator's mindset applied to how companies clarify what matters, improve how work gets done, and build systems that support sustainable growth. The brand pairs Roman inscriptional capitals with the founder's own architecture photography at blue hour. The idea is load-bearing structure: business systems built to stand without him.

## Sources

- `uploads/Tristin Brand Board 2.png`: the brand board (wordmark, palette, type roles, photography). Photographs were cropped from it into `assets/photos/`.
- `uploads/DESIGN.md`: design notes from the esfandiari.dev homepage build (tokens, masthead, section families, texture, decisions).
- `uploads/TRJN DaVinci Text.ttf`, `uploads/PPNeueMontreal-*.otf`: font files supplied by the user.
- Brief (chat): three-colour palette, DaVinci display caps with wide tracking, Red Hat Display subheads, DM Sans body, rigid two-column grid with 1px dividers, no radii, no shadows, no gradients, alternating bone / slate / dark-photo bands.

No codebase, Figma file or logo was provided, and none exists yet. The brand board is a designer's direction board (palette, type roles, photography), not finished branding. Wherever a mark would go, the name is set in TRJN DaVinci caps as a placeholder wordmark. See Iconography.

## Products

One surface: the **consulting website** (esfandiari.dev). Sections per DESIGN.md: hero, the offer, how we'd work together, selected work, writing, book a call. The UI kit in `ui_kits/website/` recreates it.

## Content fundamentals

- **Voice.** First person singular from Tristin ("I", "me") speaking to "you". Plain, declarative, matter-of-fact. Short paragraphs. Statements, not slogans.
- **Casing.** Wordmark and labels in tracked uppercase (`TRISTIN ESFANDIARI`, `SELECTED WORK`, `BOOK A CALL`). Headings in sentence case, mixed case, DaVinci. Body in normal sentence case.
- **Punctuation.** Zero em-dashes and en-dashes. Use a comma, a colon or a full stop. No exclamation marks.
- **Eyebrows.** One eyebrow per page at most (the role line, "Business Systems & Transformation Consultant"). Headlines carry the rest.
- **No emoji, ever.** No icons in copy.
- **Vocabulary.** Operator, systems, load-bearing, clarify, how work gets done, sustainable growth. Avoid startup jargon (synergy, unlock, supercharge, 10x).
- **Example lede.** "Tristin Esfandiari is a business strategy and systems consultant focused on making good businesses work better. He brings an operator's mindset to complex problems, helping companies clarify what matters, improve how work gets done, and build systems that support sustainable growth."
- **Buttons.** Two or three words, tracked caps: `BOOK A CALL`, `READ THE NOTE`, `SEE THE WORK`.
- **Section labels.** Small tracked caps in muted contrast, centred above the block: `TEXT LOGO`, `COLOR PALETTE`, `FONTS GUIDE`.

## Visual foundations

**Colour.** Three colours, no gradients. Slate blue `#69809C` is the accent and a full section ground (never a tint or a highlight colour). Near-black ink `#2B2B2B` carries text, rules and dark panels. Warm bone `#EFE3D6` is the light ground. Each has a lighter and deeper step (`--blue-deep`, `--blue-pale`, `--ink-soft`, `--ink-mute`, `--bone-lift`, `--bone-deep`, `--bone-dim`) for hierarchy, not for decoration. The masthead plate uses `--bone-lift` so it separates from a bone page. Photography is the fourth colour: dusk cityscapes, always under a flat dark wash (`--photo-wash`) plus light film grain so bone text sits on top. Files in `assets/photos/` are pre-treated; use `--photo-wash` again only when placing an untreated image from `clean/` or `src/`. Status colours are muted and derived (`--status-*`); no traffic-light red/green.

**Type.** TRJN DaVinci (Trajan-style caps) for the wordmark, all display headings and small labels. Wordmark tracked 0.28em, labels 0.22em, headings near-zero tracking. One weight; never set `font-weight`, never faux-bold. Red Hat Display 500 for subheads, ledes, engagement titles, buttons and meta. DM Sans 300 for body at 1.6 leading. Hierarchy comes from size and colour only.

**Layout.** Rigid two-column grid (the board splits roughly 59/41, the site uses `1fr auto 1fr` for the masthead and 12 columns for the facade grid). Blocks are strict rectangles separated by 1px hairline rules in the ink tone (`--rule`, or a 28% alpha version on dark and slate). Generous padding (`--section-pad-y: 96px`). Content is centred or left-aligned within its block, never justified, never right-aligned. Layout families used once each: full-bleed photo, prose plus rail, three ruled columns, facade grid, ruled index, split panel. Contrast comes from alternating bone, slate and dark-photo bands, not from ornament.

**Backgrounds.** Flat colour fields plus full-bleed photography. Paper grain is a subtle `background-image` texture on bone and dark surfaces (opacity ~6%), never a fixed overlay. No illustration, no patterns, no gradients except a single directional gradient on the hero photo for legibility.

**Imagery.** The founder's own photographs. Preferred: blue-hour architecture, towers, facades, bridges, lit windows, dusk skies. Cool sky, warm window light. Darkened with a flat wash, light grain. The library (`assets/photos/src/`) also holds daylight travel shots (cathedrals, temples, beaches, colour-saturated streets); avoid those unless washed heavily. Never bright daylight, never people close-up, never stock-office imagery. Selected work is a "facade grid": lit bays carry an image, dark bays are ink.

**Corners, borders, shadows.** Radius 0 everywhere. Borders are 1px ink hairlines. No shadows on cards or buttons. The one exception is the floating masthead plate: `--shadow-plate`, tinted to the page ink.

**Cards.** A card is a rectangle bounded by 1px rules on a flat ground. No fill change, no radius, no shadow. Cards in a row share their dividing rule (no double lines).

**Buttons.** Rectangles, 44px tall, 24px side padding, Red Hat Display 500 in tracked caps at 12px. Primary: ink fill, bone text. Secondary: 1px ink outline. Inverse (on dark or slate): 1px bone outline. Hover: primary fills `--blue-deep`; outlines invert to a solid fill. Press: no scale, colour only.

**Hover and press.** Links go from ink to `--blue-deep`. Photo bays lift the wash slightly (opacity 0.55 to 0.4). Everything transitions over 180ms with `cubic-bezier(0.4,0,0.2,1)`. No scale, no bounce, no translate.

**Motion.** None beyond hover transitions. No keyframes, no scroll-driven animation, no parallax, no autoplay. Where a reveal is unavoidable, use `animation-timeline: view()` gated on `prefers-reduced-motion: no-preference`.

**Transparency and blur.** Only the photo wash (rgba ink) and rules on dark surfaces. No backdrop blur, no glass.

**Fixed elements.** The masthead is a bone-lift plate floating clear of the viewport edges, square corners, three zones on `1fr auto 1fr`. Nothing else is fixed.

**Forms.** Inputs are 44px rectangles with a 1px ink bottom or full border on a bone-lift ground, DM Sans, no radius. Focus: 1px `--blue-deep` outline offset 3px.

## Iconography

- There is **no logo or graphic mark**, and the brand board is not final branding. The `Wordmark` component sets `TRISTIN ESFANDIARI` in TRJN DaVinci caps, tracked 0.28em, as a stand-in. Replace it when a real mark exists. DESIGN.md mentions an unused SVG (`cadence.svg`) that was not supplied; it is not reconstructed here.
- **No icon system** appears in the brand board or site notes. The site runs without icons: navigation is text, buttons are text, sections are labelled in type.
- **No emoji.** Unicode characters are limited to typographic ones: the ampersand in the role line, an arrow (`→`) may follow a text link in Red Hat Display where a directional cue is needed.
- If an interface genuinely needs icons (a prototype with forms or navigation), use **Lucide** from CDN at 1.5px stroke, 16 to 20px, in the current text colour, and treat this as a substitution to flag.

## Fonts supplied vs used

- `TRJN DaVinci Text` (supplied, `.ttf`) drives `--font-display`. Licence caveat from DESIGN.md: CC BY 4.0 from onlinewebfonts, wants attribution; Trajan Pro or Cinzel are licensed substitutes.
- `Red Hat Display` and `DM Sans` are loaded from Google Fonts (`tokens/fonts.css`). No local files were supplied.
- `PP Neue Montreal` (13 files supplied) is declared as `--font-sans-alt` and `@font-face`d, but the brief assigns subheads to Red Hat Display and body to DM Sans, so it is not used by default. Swap `--font-subhead` / `--font-body` to it if that was the intent.

## Intentional additions

Brand-guidelines-only run: no component inventory was given, so a small standard set was authored to the brand. `Wordmark` and `SectionLabel` exist because the brand's identity lives in type, not in a mark. `PhotoBand` exists because the darkened photograph is a first-class surface.

## Components

Wordmark, SectionLabel, Rule, PhotoBand, Button, IconButton, FieldLabel, Input, Select, Checkbox, Radio, RadioGroup, Switch, Card, Badge, Tag, Tabs, Dialog, Toast, Tooltip. Each has `.jsx`, `.d.ts`, `.prompt.md` and a shared card per directory.

## Index

- `styles.css`: entry point. `@import`s everything in `tokens/`.
- `tokens/fonts.css`, `colors.css`, `typography.css`, `spacing.css`, `base.css`
- `assets/fonts/`: TRJN DaVinci, PP Neue Montreal.
- `assets/photos/`: 12 curated photographs from the founder's library, treated (flat wash rgba(20,20,22,.55) plus film grain) and sized to 1920px: `hero-tower-dusk`, `hero-city-dusk`, `hero-river-dusk`, `tower-corner-dusk`, `boulevard-lamps-dusk`, `column-sky-dusk`, `street-facades`, `canal-town`, `cathedral-tower`, `sky-clouds-dusk`, `hills-glow-night`, `arch-gate`. `assets/photos/clean/` holds the same twelve untreated. `assets/photos/src/` holds all 38 originals (webp) for future picks.
- `guidelines/`: specimen cards for Colors, Type, Spacing, Brand.
- `components/brand/`: Wordmark, SectionLabel, Rule, PhotoBand
- `components/actions/`: Button, IconButton
- `components/forms/`: FieldLabel, Input, Select, Checkbox, Radio (+ RadioGroup), Switch
- `components/surfaces/`: Card, Badge, Tag, Tabs
- `components/feedback/`: Dialog, Toast, Tooltip
- `ui_kits/website/`: homepage recreation (`index.html`, one JSX per section).
- `thumbnail.html`: project tile. `SKILL.md`: agent skill entry.
