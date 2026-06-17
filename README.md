# HypeMuzik (desktop)

A cross-platform desktop **audio enhancement** app — system-wide equalizer,
volume booster, virtual surround, per-headphone correction, and a media player —
built with **Tauri 2 · React 19 · TypeScript · Tailwind v4**, with a real,
test-backed DSP engine in Rust.

> Sibling to the HypeMuzik Flutter mobile player. This is the desktop product.

## Status

**Feature-complete (Phases 0–7).** A runnable app with a real, test-backed DSP
chain and live UI:

- **DSP engine** (`hm-dsp`) — 31-band graphic EQ, bass boost, spatializer
  (crossfeed/widening), per-headphone correction, makeup gain, and a look-ahead
  brickwall limiter, in the fixed chain
  `HeadphoneCorrection → GraphicEq → BassBoost → Spatializer → Gain → Limiter`.
- **Real-time audio** (`hm-audio`) — cpal output engine with lock-free parameter
  passing, real meters + a 64-band FFT spectrum, seek/pause/progress.
- **Equalizer** — 31-band editor with a live response curve over the spectrum,
  12 built-in genre presets + custom presets (SQLite).
- **Headphone profiles** — 37 genuine AutoEq (oratory1990) curves, searchable.
- **Player** — multi-format decode (mp3/flac/aac/wav/ogg) via symphonia, a
  scanned library, and playlists.
- **Radio** — radio-browser directory + favorites + **live streaming** through
  the chain.
- **Mixer** — per-app volume (native on Windows; a graceful notice on macOS).
- **Licensing** — an explicitly-marked local trial/activation **mock**.

See [`docs/architecture.md`](docs/architecture.md) for the design and the
honest boundaries below.

## Prerequisites

- **Rust** (stable) and **Cargo**
- **Node** 20+ and **pnpm** (`corepack enable pnpm`)
- macOS: Xcode Command Line Tools. Windows: MSVC build tools + WebView2.

## Develop

```bash
pnpm install
pnpm tauri dev      # launches the app with hot-reload UI
```

## Build

```bash
pnpm build          # typecheck + bundle the frontend → dist/
pnpm tauri build    # produce a packaged desktop app
```

## Test

```bash
cargo test          # workspace Rust tests (DSP null tests, device enumeration)
```

## Layout

```
crates/        hm-core · hm-dsp · hm-audio · hm-media · hm-platform
src-tauri/     Tauri app: commands, events, wiring
src/           React app: app/ features/ components/ stores/ lib/ styles/
docs/          architecture · audio-driver · browser-extension
```

## Honest boundaries

- **System-wide capture** needs a signed virtual audio driver, which is not
  shipped here — see [`docs/audio-driver.md`](docs/audio-driver.md). The
  driver-free dev path is file playback and default-device/loopback capture.
- **Licensing** is an explicitly-marked local mock; the production contract is in
  [`docs/architecture.md`](docs/architecture.md).
- Meters and spectrum render only **real** engine data — never synthesized.
