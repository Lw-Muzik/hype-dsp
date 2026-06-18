# System-wide audio capture & processing — requirements

> **Status: design / not built.** True system-wide processing (EQ'ing other
> apps) requires routing system audio through something HypeMuzik controls. It
> **cannot be silently auto-enabled** — macOS requires either an explicit audio
> capture permission grant (process taps) or installing a signed driver and
> switching the output device. `hm-platform`'s `VirtualDeviceSource` is the stub
> for this; the driver-free dev stand-in is file/library/radio playback and
> input (mic) capture.

## What "system-wide" actually needs

To intercept the audio of arbitrary apps (Spotify, a browser, a game), their
output must flow through a tap or device we own, where we can process it and
write it back. macOS offers two real paths.

### macOS — Option A (recommended on 14.4+): Core Audio process taps

Since **macOS 14.2 / 14.4**, the public **Core Audio tap** API lets an app tap
the system (or specific processes) at the post-mix bus and — crucially — **mute
the original output** so the app can re-render processed audio inline. This is
exactly a system-wide EQ, with **no driver to build, sign, or install**. Modern
EQ apps (e.g. iQualize) work this way.

Flow: `CATapDescription` (system tap, mute behavior = muted) →
`AudioHardwareCreateProcessTap` → `AudioHardwareCreateAggregateDevice` with the
tap in `kAudioAggregateDeviceTapListKey` (private aggregate that also includes
the real output sub-device) → an IO callback on the aggregate reads the tapped
(now-muted-at-source) audio, runs it through the `ProcessChain`, and writes it
to the output sub-device.

Requirements:
- macOS 14.4+ (the user's machine is 26.x — fully supported).
- `NSAudioCaptureUsageDescription` in `Info.plist` + a one-time **user
  permission prompt** (audio capture / TCC). Cannot be bypassed silently.
- The app must be **code-signed** (Developer ID) for the permission to stick.
- Native Core Audio code (C/Obj-C/Swift) — in this Rust/Tauri app, via
  `coreaudio-sys`/`objc2` FFI or a small Swift sidecar. The API is powerful but
  sparsely documented; Apple's `AudioCap` sample is the reference.

This is the path to implement for HypeMuzik on modern macOS. It replaces the
`LoopbackCaptureSource` stand-in with a real `SystemTapSource` feeding the
existing chain.

### macOS — Option B (legacy / broad compatibility): virtual audio driver

The classic approach (eqMac, BlackHole, Loopback): ship a **user-space
AudioServerPlugin** (based on Apple's NullAudio sample) — a loopback device
installed under `/Library/Audio/Plug-Ins/HAL/`. The user (or app) sets it as the
system default output; the driver diverts system audio to its input stream,
which the app reads, processes, and sends to the real device. Runs in user space
(not a kext), but still needs **Developer ID signing + notarization** and an
**elevated installer**, and the user must switch their output device. Use this
only to support macOS < 14.4.

> `ScreenCaptureKit` (macOS 13+) can *capture* system/process audio but cannot
> re-insert processing into the playback path, so it is not usable for an inline
> EQ on its own.
>
> `cpal` cannot tap system **output** on macOS; its stand-in captures an
> **input** device (mic). Output interception needs Option A or B above.

### Windows

- An **APO (Audio Processing Object)** inserted into the system effects pipeline,
  or a **virtual audio driver** (e.g. a WDM/AVStream or APO-based device) the
  user selects as the default output.
- Driver packages must be **signed** (EV cert + Microsoft attestation/WHQL for
  broad install); installed via an elevated installer.
- `cpal` **WASAPI loopback** *can* capture the current default output device
  without a driver — this is the real, driver-free Windows capture stand-in.

## How it slots into the architecture

```rust
pub trait AudioSource: Send {
    fn start(&mut self, format: StreamFormat) -> Result<(), AudioError>;
    fn read(&mut self, out: &mut [f32], channels: usize) -> usize;
    fn stop(&mut self);
}
```

A production `VirtualDeviceSource` implements `AudioSource` by reading frames
from the installed virtual device's tap. Until the driver is installed, the stub
returns `AudioError::Unavailable` and the UI shows an install/CTA state — never a
crash, never fabricated audio.

## Installer / distribution work (out of session)

- Build + sign the platform driver/plugin in its own native toolchain.
- Ship a privileged installer that places and registers it, with uninstall.
- Detect install state at runtime and surface it in the UI (the trait already
  models the `Unavailable` case).
