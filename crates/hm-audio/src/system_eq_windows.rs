//! System-wide EQ on Windows via a bundled virtual audio device.
//!
//! Windows has **no pure user-space way** to intercept-and-replace system audio:
//! WASAPI loopback captures the mix but can't silence the original, so replaying
//! the EQ'd copy would double the sound. The chosen approach mirrors VB-Cable /
//! FxSound — ship a **virtual audio output device**, make it the default so apps
//! render into it, then this process:
//!
//!   1. WASAPI **loopback-captures** the virtual device's render stream;
//!   2. runs the samples through the shared DSP [`ProcessChain`] (live params);
//!   3. **renders** the result to the real output device.
//!
//! The originals never reach the speakers (they go to the virtual device), so
//! there's no doubling — the same re-routing model as the Linux virtual sink and
//! the macOS muted tap.
//!
//! ## Status
//! Two parts make this work, and only one can be built/tested off-Windows:
//!   - **The driver + installer** (signed package that registers the virtual
//!     device): a separate signing/packaging effort — see `docs/system-eq.md`.
//!   - **The WASAPI capture→DSP→render loop** below: a focused chunk to wire
//!     against the installed driver on a real Windows box.
//!
//! Until both land, [`available`] reports whether the virtual device is present
//! and [`WindowsSystemEq::start`] returns a clear, actionable error rather than
//! shipping untested real-time FFI. The implementation plan is documented inline
//! so it can be completed deterministically.

#![cfg(target_os = "windows")]

use std::sync::Arc;

use arc_swap::ArcSwap;
use hm_core::EngineState;

use crate::error::AudioError;

/// Substring that identifies our bundled virtual output device among the render
/// endpoints (the installer registers it under this friendly name).
pub const VIRTUAL_DEVICE_NAME: &str = "HypeMuzik";

/// Whether the bundled HypeMuzik virtual audio device is installed.
///
/// TODO(windows-driver): enumerate active render endpoints
/// (`IMMDeviceEnumerator::EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)`),
/// read each endpoint's `PKEY_Device_FriendlyName`, and return true when one
/// contains [`VIRTUAL_DEVICE_NAME`]. Returns false until the driver ships, so
/// the UI shows an "install the audio driver" prompt instead of a dead toggle.
pub fn available() -> bool {
    false
}

/// A running Windows system-wide EQ pipeline (loopback capture → DSP → render).
/// Dropping it stops the audio threads and releases the WASAPI clients.
pub struct WindowsSystemEq {
    _state: Arc<ArcSwap<EngineState>>,
}

impl WindowsSystemEq {
    /// Start capturing the virtual device, processing, and rendering to the real
    /// output. `state` carries the engine's live EQ/effects/power/volume params.
    ///
    /// TODO(windows-driver): implement with the `windows` crate (WASAPI):
    ///   1. `CoInitializeEx`; create `IMMDeviceEnumerator`.
    ///   2. Capture client: get the virtual device (friendly name match),
    ///      `Activate::<IAudioClient>()`, `Initialize` with
    ///      `AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK`
    ///      at its mix format, `SetEventHandle`, `GetService::<IAudioCaptureClient>`.
    ///   3. Render client: get the real device (default render minus ours),
    ///      `Initialize` shared event-driven, `GetService::<IAudioRenderClient>`.
    ///   4. Worker thread: on the capture event, `GetBuffer`/`ReleaseBuffer`,
    ///      convert to interleaved f32 stereo (honor the mix format / resample to
    ///      the render format with [`crate::resampler::StereoResampler`]), run
    ///      `ProcessChain::standard(rate, 2)` (`set_params`/`process`) gated on
    ///      `state.power`, then write to the render `IAudioRenderClient` buffer.
    ///      `MMCSS` ("Pro Audio") the thread for low latency.
    ///   5. Store the threads/clients on `self`; stop + release them in `Drop`.
    ///
    /// Mirrors `system_eq_linux.rs`'s capture→DSP→render loop, but with WASAPI
    /// instead of `parec`/`pacat`.
    pub fn start(state: Arc<ArcSwap<EngineState>>) -> Result<Self, AudioError> {
        let _ = state;
        Err(AudioError::Unavailable(
            "Windows system-wide EQ needs the HypeMuzik virtual audio device — \
             install it from Settings (see docs/system-eq.md)."
                .into(),
        ))
    }
}
