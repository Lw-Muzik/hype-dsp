# Production virtual audio driver — requirements

> **Status: design / not built.** True system-wide audio capture requires a
> signed, OS-installed virtual audio device. None is produced in this codebase.
> `hm-platform`'s `VirtualDeviceSource` is a stub that reports `Unavailable`;
> the app degrades to file playback and default-device/loopback capture (see
> `hm-audio`) as the driver-free development stand-in.

## What "system-wide capture" actually needs

To intercept the audio of arbitrary apps (Spotify, a browser, a game), the OS
must route their output through a device we control. That device is a kernel- or
HAL-level component the OS loads at boot, and modern OSes refuse to load
unsigned ones.

### macOS

- **CoreAudio AudioServerPlugin (HAL plugin)** implementing a virtual output
  device, installed under `/Library/Audio/Plug-Ins/HAL/`.
- The plugin presents a virtual device the user (or the app) selects as the
  system output; audio written to it is mirrored to a tap we read from.
- **Code signing + notarization** with an Apple Developer ID; hardened runtime.
  Installation requires admin rights (a privileged helper or installer pkg).
- A lighter alternative for *capture only* (not routing) is **ScreenCaptureKit**
  (macOS 13+), which can tap system/process audio with user consent — usable for
  monitoring but not for inserting processing into the playback path.
- Note: `cpal` cannot tap system **output** on macOS natively; its loopback
  stand-in captures an **input** device. Output loopback needs one of the above.

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
