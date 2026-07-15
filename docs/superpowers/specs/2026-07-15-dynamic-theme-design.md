# Dynamic theme — design

Status: approved, not yet implemented
Date: 2026-07-15

The app is dark-only today. This adds a user-selectable theme with four values —
**System**, **Dynamic**, **Light**, **Dark** — where Dynamic paints the current
track's cover art, blurred, behind the whole window. The blur radius is a user
control.

## Why this is mostly a CSS problem

`src/styles/index.css` already defines semantic tokens through Tailwind v4's
`@theme`, and components were disciplined about using them: across the entire
`src/` tree there is exactly **one** raw hex in a component
(`Surround3DPanel.tsx:82`). Tailwind v4 emits utilities that reference
`var(--color-*)`, so redefining those variables under a selector re-points every
utility at once:

```css
@theme { --color-surface: #0a0b0e; }        /* dark = the default */
:root[data-theme="light"] { --color-surface: #f4f5f7; }
```

No component has to change to gain a theme. Two constraints make this work:

* The block must stay plain `@theme`. `@theme inline` bakes values into the
  utilities and would silently break every override.
* `:root[data-theme=…]` (0,1,1) outranks `:root` (0,1,0), so the cascade lands
  the right way round without `!important`.

Rejected: separate per-theme stylesheets (duplicates the palette, drifts), and
applying colours from JS (loses the cascade, guarantees a flash on launch).

## Theme resolution

The store holds the user's choice; a resolver maps it to a concrete theme, so
`data-theme` only ever carries `dynamic | light | dark`:

```
system  → prefersDark ? dark : light
dynamic → dynamic
light   → light
dark    → dark
```

The resolver is a **pure function**, `resolveTheme(choice, prefersDark)`. The
`matchMedia` subscription lives in a thin adapter beside it. That is not
ceremony: there is no vitest config in this repo, so tests run in the default
**node** environment with no `matchMedia` and no jsdom, and the house convention
is to export pure functions and test them directly (`engine.test.ts` imports
`ytmusicItem`). Keeping the rule pure means the live-OS-flip case is testable
without pulling jsdom into the project.

Dynamic is **dark-based**: its scrim is dark and its palette derives from the
dark one. A light-based dynamic theme would need a second set of contrast
guarantees for no clear gain, and every music player that ships this effect
treats it as a dark surface.

`index.html` gets a small inline script that reads `localStorage` and sets
`data-theme` before first paint. Without it, a Light user watches the window
flash near-black on every launch.

## The backdrop

`App.tsx:69` — the single root container — gains `relative isolate`, and
`<ThemeBackdrop/>` mounts as its first child at `absolute inset-0 -z-10`.

`isolate` forces the root to be a stacking context. Within one, painting order
is: the context element's own background, then **negative z-index children**,
then in-flow non-positioned descendants. So the backdrop lands above the root's
background and below every piece of chrome, and `Sidebar`, `TopBar`,
`NowPlayingBar` and `main` need **no changes at all** — they reveal the art
purely by having translucent surface tokens in Dynamic.

```
body                 --color-canvas          (always opaque; see below)
 root (bg-surface)
   ThemeBackdrop     art + scrim + grain     (-z-10)
   Sidebar           bg-surface-raised       (alpha in Dynamic → art glows)
   TopBar / Player   bg-surface              (alpha in Dynamic)
   main              no background           (art directly behind content)
```

`--color-canvas` is a new token, used only by `body`. It exists because Dynamic
gives `--color-surface` an alpha; `body` would otherwise become translucent and
expose the browser canvas (white) behind the app.

**No `backdrop-filter` anywhere.** The shell is a flex layout of non-overlapping
regions and `main` scrolls inside its own box (`App.tsx:76`), so nothing ever
scrolls beneath translucent chrome. A flat translucent colour over
already-blurred art reads as frosted glass without a second blur pass. This also
sidesteps a decade-old WebKit bug where `backdrop-filter` leaves artifacts on
rounded corners ([158807](https://bugs.webkit.org/show_bug.cgi?id=158807)) —
relevant, since this app ships on WKWebView and WebKitGTK.

### Dynamic surface tokens

Dynamic inherits every dark value except these. Only the two *chrome* surfaces
gain alpha; overlays stay opaque, because a translucent dropdown panel over
moving content is unreadable — the token names already draw that line.

| token | Dynamic value | why |
| --- | --- | --- |
| `--color-canvas` | `#0a0b0e` (opaque) | `body`'s base, never translucent |
| `--color-surface` | `rgb(10 11 14 / 0.55)` | TopBar, player bar → art glows |
| `--color-surface-raised` | `rgb(20 22 28 / 0.55)` | Sidebar → art glows |
| `--color-surface-overlay` | `#1b1e26` (unchanged) | dropdowns, dialogs stay legible |
| `--color-border` / `-strong` | unchanged | |
| text tiers | see the contrast table below | |

### Layers

`ThemeBackdrop` renders two crossfading art wrappers plus two static layers:

```html
<!-- A and B: promoted wrappers. Blur rasterises into THIS texture and is reused
     across the whole crossfade; only opacity animates here. -->
<div class="backdrop" style="will-change:transform; opacity:…; transition:opacity 600ms linear">
  <!-- un-promoted child, oversized so the Gaussian fade is off-screen -->
  <div class="art" style="inset:calc(var(--hm-backdrop-blur) * -3);
                          filter:blur(var(--hm-backdrop-blur)) saturate(1.5)"></div>
</div>
<div class="backdrop"><div class="art">…</div></div>   <!-- the other of A/B -->

<div class="scrim"></div>   <!-- rgb(10 11 14 / 0.72), above both art layers -->
<div class="grain"></div>   <!-- unscaled, unblurred, topmost -->
```

Scrim and grain sit above *both* art layers and never animate, so a track change
only touches the two opacities.

Four details here are load-bearing, and each one is a mistake we already made
once on paper:

**`inset: -3σ`, not `scale-110`.** The length in `filter: blur(N)` is a Gaussian
*standard deviation*, not a radius, so `blur(48px)` bleeds ~144px and samples
transparent pixels beyond the element, fading the edges. `scale-110` on a
1440×900 layer buys only ~72px — not enough — and `transform` applies *after*
`filter`, so it magnifies the fade band instead of hiding it. Oversizing by 3σ
puts the fade genuinely off-screen. (`LyricsView.tsx:242` gets away with
`scale-150` only because 150% happens to exceed its 3σ.)

**`saturate` after `blur`.** Filter functions apply left to right. Blur averages
neighbouring colours toward grey; saturating afterwards restores the chroma that
averaging removed. Saturating first would boost colours that are about to be
averaged away. This is why the filter is written inline rather than assembled
from Tailwind classes, which compose in a fixed order.

**`will-change: transform` on the wrapper, never on the blurred child.**
Animating opacity on a blurred element forces the GPU to re-blur the texture
every frame — Chrome measures ~90ms/frame. Promoting the parent lets the blur
rasterise once into the parent's texture and be reused for the whole crossfade.

**The grain layer is a separate, unscaled, unblurred sibling.** If it rides on
the oversized art layer it gets magnified and stops working as per-pixel dither.

### Blur control

`--hm-backdrop-blur` is set inline on the root from the store, so dragging the
slider retargets one CSS variable — no React re-render of the image.

Range **8–96px, default 48**. The floor is not zero: at zero the raw cover fills
the window behind the library, and covers routinely carry their own text and
borders which collide with UI text. ~8px destroys that high-frequency detail
while still reading as the cover. The ceiling is a cost decision — the art layer
is `viewport + 6σ`, so 96px already means ~2.9M pixels to blur.

`THEME_LIMITS = { min, max, default, step }` is exported alongside the store,
matching the existing `VISUALIZER_LIMITS` / `SIDEBAR_LIMITS` shape.

### Track changes

`nowPlayingMeta.cover` is `null` for a beat after every track change while tags
decode or a cloud cover is fetched, so a naive backdrop flashes empty between
tracks. The component keeps the previous art until a new cover actually arrives,
then A/B ping-pongs two layers and crossfades opacity over **600ms linear**.

Linear, not eased — an eased crossfade dips visibly in the middle. 600ms is
longer than Material's <400ms guidance, which targets functional transitions; an
ambient backdrop is not one.

Fallbacks: no embedded art → the deterministic gradient from `lib/cover.ts`,
the same one `Artwork` already shows, so the backdrop matches the artwork on
screen. Nothing playing → plain surface, no backdrop.

## Contrast

Every value below is computed, not eyeballed, against the **worst possible cover
art** (pure white). The check is a unit test, so the palette cannot regress.

Two knobs (art opacity *and* scrim) multiply: `0.32 × (1 − 0.62)` would crush
peak white to 31/255, squeezing a maximally-smooth blurred gradient into ~31
levels — the textbook banding generator. So the art layer stays at opacity 1.0
and **the scrim is the only darkening step**: `rgb(10 11 14 / 0.72)`, leaving
the art 71 of 255 levels.

That still lifts the effective background, and `--color-text-muted` / `-faint`
are tuned for near-black — they fail against bright art at *any* scrim (muted is
only 4.25:1 even at 0.80). Rather than darken until the art disappears, Dynamic
**redefines its own text tokens**, which is exactly what a separate theme is
for:

| token | Dynamic value | vs white art | vs black art |
| --- | --- | --- | --- |
| `--color-text` | `#eceef2` (unchanged) | 7.04:1 | 17.25:1 |
| `--color-text-muted` | `#c5cbd6` (brightened) | 5.02:1 | 12.30:1 |
| `--color-text-faint` | `#a7aeba` (brightened) | 3.66:1 | 8.97:1 |

Chrome is safe by construction: the sidebar's translucent surface over the
worst-case backdrop still gives 11.35:1 for body text.

71 levels of smooth gradient will band on an 8-bit display, so the grain layer
is required, not decorative — KDE and Windows Acrylic pair blur with noise for
this reason. A pre-rendered `feTurbulence` tile (`fractalNoise`, `baseFrequency
0.65`, `numOctaves 3`, `stitchTiles="stitch"` — without `stitch` the tile seams
show) as a static data-URI background, `background-size: 182px`, `opacity 0.04`,
`mix-blend-mode: overlay`. It is baked once, never a live filter.

## The accent split

`--color-accent` is overloaded: 24 uses as a fill (`bg-accent`) and 52 as text
on a surface (`text-accent` ×19, `text-accent-strong` ×33). Those need opposite
treatment under a light background. Worse, `bg-accent` pairs with `text-surface`
in 8 places — which works only because `surface` is near-black today. Flip it to
near-white and every amber button becomes unreadable.

Fix: add **`--color-on-accent`**, the text colour that sits on an accent fill,
and point those 8 sites plus `Button` at it. The accent then flips per theme:

| | Dark | Light |
| --- | --- | --- |
| `--color-accent` (fill) | `#f5b40f` | `#8a6000` |
| `--color-accent-strong` (emphasis, hover) | `#ffca42` | `#6b4a00` |
| `--color-on-accent` | `#0a0b0e` | `#ffffff` |

Both directions keep their meaning: `hover:bg-accent-strong` is *brighter* on
dark and *darker* on light, which is "more emphasis" either way.
`text-accent` lands at 10.70:1 on dark and 5.13:1 on light; `on-accent` on a
light fill is 5.59:1.

**This fixes a live bug.** `Button.tsx:12` renders `primary` as
`bg-accent text-text` — near-white on amber, **1.58:1** — in the theme that
ships today. It becomes `bg-accent text-on-accent`, ≈10.7:1 on dark. Approved as
part of this work.

## Light palette

Not an inversion. In a light UI, raised surfaces go *lighter* (white lifts off a
grey base), which is the opposite of dark, where raised surfaces go lighter too
but from near-black.

| token | value | check |
| --- | --- | --- |
| `--color-canvas` / `--color-surface` | `#f4f5f7` | base, not pure white |
| `--color-surface-raised` | `#ffffff` | cards, sidebar |
| `--color-surface-overlay` | `#ffffff` | panels (rely on shadow) |
| `--color-border` / `-strong` | `#e3e5ea` / `#cdd1d9` | |
| `--color-text` | `#16181d` | 16.28:1 |
| `--color-text-muted` | `#5a616e` | 5.71:1 |
| `--color-text-faint` | `#767d8a` | 3.80:1 (decorative tier, ≥3:1) |
| `--color-accent-muted` | `#fdf3d9` | pale amber badge fill |

`:root` also sets `color-scheme: light|dark` per theme so form controls,
scrollbars and the window's own chrome follow.

## Accessibility

* `prefers-reduced-motion` — snap instead of crossfade. There is no shared hook;
  the existing pattern is a module-level `matchMedia` const
  (`AlbumDeck.tsx:21`, `LyricsView.tsx:10`). This adds a third copy, so promote
  it to `lib/reducedMotion.ts` and adopt it in all three.
* `prefers-reduced-transparency` — the media query built for exactly this
  pattern (macOS *Reduce transparency*). Drop the art layer and make surfaces
  opaque. Progressive enhancement: it is not yet Baseline, so it must only ever
  remove effects.
* Never animate the blur radius; only opacity between two pre-blurred layers.

## Persistence

`localStorage`, hand-rolled load/save with clamp-on-read, matching the house
pattern (`stores/ui.ts:23`, `stores/visualizer.ts:23`):

* `hm.theme` — `system | dynamic | light | dark`, unknown values fall back to
  `system`
* `hm.theme.blur` — number, clamped to `THEME_LIMITS`

Not `tauri-plugin-store`: it is registered at `lib.rs:216` but has no JS
dependency and zero call sites — it is dead. Not Rust either; nothing in Rust
reads this, and a round-trip would only add a flash on launch.

## Settings

A new feature-owned `ThemeCard`, dropped into `SettingsView`'s card grid like
`YtMusicCard` and the others. A segmented control for the four themes, and the
blur `Slider`, disabled unless the resolved theme is Dynamic.

The segmented control means generalising `LayoutToggle`, which is currently
hardcoded to `list | grid` with baked-in icons. Its markup is already the house
segmented pattern; it grows an item list and `LayoutToggle` becomes a caller.
`Slider` needs an explicit width class — passing `className` replaces its
`flex-1` default and a 0px track silently ignores drags.

## Files

| file | change |
| --- | --- |
| `src/styles/index.css` | `--color-canvas`, `--color-on-accent`; `[data-theme]` blocks for light/dynamic; grain tile |
| `src/stores/theme.ts` | new — choice, resolver, blur, `THEME_LIMITS`, persistence |
| `src/features/theme/ThemeBackdrop.tsx` | new — layers, crossfade, fallback |
| `src/features/settings/ThemeCard.tsx` | new |
| `src/lib/reducedMotion.ts` | new — promoted from two copies |
| `src/app/App.tsx` | `relative isolate` + mount `<ThemeBackdrop/>` |
| `src/components/LayoutToggle.tsx` | generalise to a segmented control |
| `src/components/Button.tsx` | `text-text` → `text-on-accent` (bug fix) |
| 8 call sites | `text-surface` on accent → `text-on-accent` |
| `index.html` | pre-paint theme script |
| `src/features/settings/SettingsView.tsx` | mount `ThemeCard` |
| `src/stores/theme.test.ts` | new — resolver, clamp, persistence |
| `src/styles/palette.test.ts` | new — WCAG assertions over every theme |

## Testing

Vitest (`pnpm test`, `vitest run`; 2 store tests exist today).

* **Palette contrast** — the checks in this document become assertions: every
  text tier against its surface in all three themes, plus Dynamic composited
  over synthetic white and black art. This is the test that matters; the first
  accent colour tried here failed at 4.26:1 and `text-faint` at 2.84:1, and
  neither was visible by eye.

  It must assert against **the real palette, not a copy**. The test reads
  `src/styles/index.css` from disk (plain `fs` — it runs in node) and extracts
  the `@theme` and `[data-theme=…]` blocks with a small regex over
  `--color-*: value` declarations. Re-declaring the hexes in the test would
  guard nothing: the copy and the stylesheet would drift, and the test would
  keep passing while the app regressed. The parser only needs to handle the
  declaration shapes this file actually uses; if it ever fails to find a token
  it must throw rather than skip, so a rename can't silently empty the suite.
* **Resolver** — `system` follows `matchMedia` and reacts to a live OS flip;
  unknown stored values fall back.
* **Blur clamp** — out-of-range and non-numeric stored values clamp.
* **Backdrop** — renders nothing unless Dynamic; holds previous art when `cover`
  goes `null`; falls back to the gradient when a track has no art.

Manual, on device: bright vs dark album art, a cover with text on it, track
changes, the blur slider through its range, all four themes, macOS *Reduce
transparency*, and a light-theme cold launch to confirm no dark flash.

## Deliberately excluded

* **Palette extraction.** Plexamp and Spotify derive a colour and render a
  gradient rather than blurring the bitmap, because covers carry text and
  borders. The Apple Music / YouTube Music approach — blur the bitmap — is what
  was asked for. A palette gradient also bands harder and would still need the
  grain layer.
* **An art-intensity slider.** Blur only, as agreed. The scrim is fixed at the
  value the contrast table depends on.
* **Downscaling the art to a thumbnail before blurring.** Blur cost tracks layer
  area, not source resolution, and the decode is shared with `Artwork`, so this
  buys only decode memory for one image at a time. Revisit if a pathological
  cover shows up.
* **Baking the blur to a canvas.** Would make the crossfade trivially cheap, but
  the `will-change`-on-wrapper fix already caches the blur, and baking would put
  a re-render between the slider and the screen.

## Risks

* **WebKitGTK below 2.46 blurs slowly** — CSS filters only became Skia-backed
  (and accelerated) in 2.46. This app has already been bitten here: the
  cross-arch audit logged WebKitGTK `shadowBlur` as a P0. If Linux drags, cap
  the blur ceiling per platform via the existing `lib/platform.ts`.
* `mix-blend-mode` differs subtly between WebKit and Blink; check the grain on
  both. If it misbehaves, drop the blend mode — plain low-opacity noise still
  dithers.
* Two promoted full-screen layers hold GPU memory for as long as Dynamic is
  active. Only Dynamic mounts the backdrop; the other themes render nothing.
