# HypeMuzik (desktop)

A cross-platform desktop **audio enhancement** app — system-wide equalizer,
volume booster, virtual surround, per-headphone correction, and a media player —
built with **Tauri 2 · React 19 · TypeScript · Tailwind v4**, with a real,
test-backed DSP engine in Rust.

> Sibling to the HypeMuzik Flutter mobile player. This is the desktop product.

## Status

**Phase 0 — scaffold.** The app window opens with the full shell (sidebar, top
bar with power toggle / master volume / idle meters, six feature routes). The
Cargo workspace and React app build clean; DSP, audio engine, media, mixer, and
licensing arrive in later phases. See [`docs/architecture.md`](docs/architecture.md)
for the plan.

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
