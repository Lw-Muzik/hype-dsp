# HypeMuzik — Architecture

HypeMuzik is a cross-platform desktop **audio enhancement** app: a system-wide
equalizer, volume booster, virtual surround processor, and media player. It runs
incoming audio through a real DSP chain and exposes a polished, real-time control
UI built with Tauri 2 + React 19 + TypeScript + Tailwind v4.

## Signal path

```
[ source ] -> [ DSP ProcessChain ] -> [ sink (output device) ]
   |                |                        ^
   |          params snapshot (lock-free)    |
   |          meters + spectrum  ------------> emitted to UI (~30-60 fps)
```

- **Sources** (`hm-audio`): file/decoder playback, default-device/loopback
  capture, and a documented virtual-device stub — all behind the `AudioSource`
  trait.
- **DSP** (`hm-dsp`): an ordered `ProcessChain` of `AudioProcessor` stages,
  fixed order `HeadphoneCorrection → GraphicEq → BassBoost → Spatializer → Gain
  → Limiter`. Pure, no I/O, unit-tested.
- **Sink** (`hm-audio`): the selected `cpal` output device.
- **Telemetry**: per-block peak/RMS and a throttled FFT magnitude vector pushed
  to the UI over a Tauri channel.

## The hard constraint — system-wide capture

True system-wide capture (intercepting Spotify, a browser, a game) requires a
**signed virtual audio device**: a CoreAudio HAL/AudioServer plugin on macOS, or
an APO / virtual audio driver on Windows. These must be built, code-signed,
notarized, and installed out of band. We do **not** ship or fake one. Instead:

- The capture surface lives behind the `AudioSource` trait.
- Two real sources ship: file/decoder playback and default-device/loopback
  capture (the legitimate, driver-free dev stand-in).
- A `VirtualDeviceSource` stub returns a clear `Unavailable` state, documented in
  [`audio-driver.md`](./audio-driver.md).

## Workspace layout

A Cargo workspace keeps audio/DSP logic in standalone, independently-testable
crates, separate from the Tauri app.

| Crate | Responsibility |
|---|---|
| `hm-core` | Shared serde types, presets/profiles, persistence, the licensing seam. No I/O. |
| `hm-dsp` | Pure DSP: biquads, the `ProcessChain`, tests. No I/O, no Tauri. |
| `hm-audio` | `cpal` device I/O, `AudioSource`/`AudioSink`, the real-time engine. |
| `hm-media` | Local player (library/playlist) and internet radio, both through the chain. |
| `hm-platform` | Per-app mixer + virtual-driver stubs, OS `cfg`-gated. |
| `src-tauri` | Tauri commands/events; wires the crates to the React UI. |

The React app lives in `src/` (`app/` shell + routing, `features/` per view,
`components/` primitives, `stores/` Zustand mirrors, `lib/` typed IPC + types,
`styles/` Tailwind tokens).

## Real-time audio rules (non-negotiable)

The `cpal` output callback must never allocate, lock a mutex, log, or do I/O.

- Parameter changes flow UI → typed `invoke` → Tauri command → write into an
  `ArcSwap<EngineParams>`. The audio thread reads the latest snapshot at the top
  of each block; no lock is shared with the callback.
- Meters/spectrum are computed in/after the callback into a bounded SPSC ring,
  drained on a normal thread, and emitted to the UI at ~30–60 fps.

## IPC contract

- Commands are verbs: `engine_set_power`, `eq_apply_preset`,
  `mixer_set_session_volume`. App-defined commands are always callable from the
  webview; the ACL in `src-tauri/capabilities/` governs only core/plugin
  commands.
- Every command returns a serde-serializable value or `hm_core::IpcError
  { code, message }`.
- Streaming data (meters, spectrum, transport progress) flows over a Tauri
  channel/event, never command polling.
- Canonical TS interfaces in `src/lib/types.ts` mirror the serde payloads
  exactly; components call typed wrappers in `src/lib/ipc.ts`, never `invoke`
  directly.

## Persistence

- **SQLite** (via `rusqlite`, bundled) for presets, headphone profiles, library,
  and favorites.
- **`tauri-plugin-store`** for simple settings. No browser storage
  (`localStorage`/`sessionStorage`) is used for app data.

## Licensing — production contract (the shipped impl is a mock)

The app gates a trial/activation flow behind the `LicenseService` trait
(`hm-core`). The shipped implementation is an **explicitly-marked local mock**
that persists trial/license state to disk. There is **no** real DRM, key
cryptography, or activation server, and the app must not imply otherwise.

A production backend would implement the same trait against:

- An **activation service** (HTTPS) that validates a signed license key and
  returns entitlement plus device-binding info.
- **Asymmetric verification** on-device (the app holds a public key; the server
  signs license tokens) so entitlement can be checked offline within a grace
  window.
- A **device registry** (activate/deactivate seats), and a **payment provider**
  (e.g. Stripe) driving license issuance.

The UI depends only on `LicenseStatus`, so swapping the mock for the real
service requires no front-end change.

## Build phases

0. **Scaffold** (this milestone) — workspace, Tauri shell, nav, empty views,
   green `cargo check` + `pnpm build`.
1. DSP core (`GraphicEq`, `Gain`, `Limiter`, `ProcessChain`) + tests.
2. Audio path + live power/master-volume control + real meters.
3. EQ UI + presets (SQLite) + spectrum analyzer.
4. Remaining DSP + headphone profiles (AutoEq dataset + picker).
5. Media: player (library/playlist) + radio (directory/stream/favorites).
6. Mixer + licensing mock + capture stand-in + `audio-driver.md`.
7. Polish: error/empty/loading states, settings, accessibility, docs.
