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
//! The WASAPI capture→DSP→render loop below is **implemented and compile-verified
//! for `x86_64-pc-windows-msvc`** (the project's primary gate, run on the macOS
//! dev host via `cargo xwin`). It is **untested on a real Windows device** — the
//! real-time COM/threading paths have never been exercised against live audio —
//! and it still requires the **bundled virtual audio driver** (a separate signing
//! / packaging effort, see `docs/system-eq.md`) to function: without that device
//! present, [`available`] returns `false` and [`WindowsSystemEq::start`] returns a
//! clear, actionable error instead of trying to capture a device that isn't there.
//!
//! Two parts make this work; only the loop can be built/tested off-Windows:
//!   - **The driver + installer** (signed package registering the virtual device):
//!     a separate effort.
//!   - **The WASAPI capture→DSP→render loop** below: implemented here, mirroring
//!     `system_eq_linux.rs`'s capture→DSP→render structure but with WASAPI instead
//!     of `parec`/`pacat`.
//!
//! ### Default-device switch — approach taken
//! Making the virtual device the *default* render endpoint (so apps render into it
//! automatically) needs `IPolicyConfig::SetDefaultEndpoint`, an **undocumented**
//! COM interface the public `windows` crate does not expose. We declare it
//! ourselves with the `windows::core::interface` macro using the well-known
//! `CLSID_PolicyConfigClient` / `IID_IPolicyConfig` and the documented vtable
//! order (the ten preceding methods are declared as opaque slots so
//! `SetDefaultEndpoint` lands at the correct vtable offset). If creating the
//! `IPolicyConfig` object fails at runtime (e.g. a future Windows build drops the
//! interface), we **gracefully skip auto-switching** and run anyway — the user can
//! set the HypeMuzik device as the default output manually, as many EQ apps
//! require. Whatever we switched is restored on stop.

#![cfg(target_os = "windows")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use arc_swap::ArcSwap;
use hm_core::EngineState;
use hm_dsp::ProcessChain;
use windows::core::{interface, IUnknown, IUnknown_Vtbl, GUID, HRESULT, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, PROPERTYKEY, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioCaptureClient, IAudioClient, IAudioRenderClient, IMMDevice,
    IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK, DEVICE_STATE_ACTIVE,
    WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
};
use windows::Win32::Media::KernelStreaming::WAVE_FORMAT_EXTENSIBLE;
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW,
    WaitForSingleObject,
};

use crate::error::AudioError;
use crate::resampler::StereoResampler;
use crate::system_eq_shared::process_block;

/// Substring that identifies our bundled virtual output device among the render
/// endpoints (the installer registers it under this friendly name).
pub const VIRTUAL_DEVICE_NAME: &str = "HypeMuzik";

/// We always present a stereo float pipeline to the DSP chain; non-stereo device
/// mixes are down/up-mixed to this on capture and fanned back out on render.
const DSP_CHANNELS: usize = 2;

/// `PKEY_Device_FriendlyName` — `{a45c254e-df1c-4efd-8020-67d146a850e0}`, pid 14.
/// Not re-exported by the `windows` crate's audio module, so spell it out.
const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 14,
};

// ---------------------------------------------------------------------------
// IPolicyConfig — undocumented default-endpoint control (see module docs).
// ---------------------------------------------------------------------------

/// `CLSID_PolicyConfigClient` — the COM class that implements [`IPolicyConfig`].
const CLSID_POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

/// `IPolicyConfig` (undocumented). Only [`IPolicyConfig::SetDefaultEndpoint`] is
/// called; the ten methods ahead of it in the vtable are declared as opaque slots
/// (raw pointers, never invoked) purely so `SetDefaultEndpoint` resolves to its
/// correct offset. Vtable order is the well-known `PolicyConfig.h` layout.
#[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
unsafe trait IPolicyConfig: IUnknown {
    // 0: GetMixFormat(PCWSTR, WAVEFORMATEX**)
    unsafe fn get_mix_format(&self, name: PCWSTR, format: *mut *mut WAVEFORMATEX) -> HRESULT;
    // 1: GetDeviceFormat(PCWSTR, INT, WAVEFORMATEX**)
    unsafe fn get_device_format(
        &self,
        name: PCWSTR,
        default: i32,
        format: *mut *mut WAVEFORMATEX,
    ) -> HRESULT;
    // 2: ResetDeviceFormat(PCWSTR)
    unsafe fn reset_device_format(&self, name: PCWSTR) -> HRESULT;
    // 3: SetDeviceFormat(PCWSTR, WAVEFORMATEX*, WAVEFORMATEX*)
    unsafe fn set_device_format(
        &self,
        name: PCWSTR,
        endpoint_format: *const WAVEFORMATEX,
        mix_format: *const WAVEFORMATEX,
    ) -> HRESULT;
    // 4: GetProcessingPeriod(PCWSTR, INT, PINT64, PINT64)
    unsafe fn get_processing_period(
        &self,
        name: PCWSTR,
        default: i32,
        default_period: *mut i64,
        min_period: *mut i64,
    ) -> HRESULT;
    // 5: SetProcessingPeriod(PCWSTR, PINT64)
    unsafe fn set_processing_period(&self, name: PCWSTR, period: *const i64) -> HRESULT;
    // 6: GetShareMode(PCWSTR, *mut DeviceShareMode)
    unsafe fn get_share_mode(&self, name: PCWSTR, mode: *mut i32) -> HRESULT;
    // 7: SetShareMode(PCWSTR, *mut DeviceShareMode)
    unsafe fn set_share_mode(&self, name: PCWSTR, mode: *const i32) -> HRESULT;
    // 8: GetPropertyValue(PCWSTR, const PROPERTYKEY&, PROPVARIANT*)
    unsafe fn get_property_value(
        &self,
        name: PCWSTR,
        key: *const PROPERTYKEY,
        value: *mut PROPVARIANT,
    ) -> HRESULT;
    // 9: SetPropertyValue(PCWSTR, const PROPERTYKEY&, PROPVARIANT*)
    unsafe fn set_property_value(
        &self,
        name: PCWSTR,
        key: *const PROPERTYKEY,
        value: *const PROPVARIANT,
    ) -> HRESULT;
    // 10: SetDefaultEndpoint(PCWSTR, ERole) — the one we actually use.
    unsafe fn set_default_endpoint(&self, device_id: PCWSTR, role: i32) -> HRESULT;
    // 11: SetEndpointVisibility(PCWSTR, INT)
    unsafe fn set_endpoint_visibility(&self, name: PCWSTR, visible: i32) -> HRESULT;
}

// ---------------------------------------------------------------------------
// COM lifetime guard
// ---------------------------------------------------------------------------

/// RAII guard that balances a successful `CoInitializeEx` with `CoUninitialize`
/// on the *same* thread. WASAPI requires COM to be initialized; we own that on
/// the threads we control and undo it when the guard drops.
struct ComGuard {
    /// `CoUninitialize` must balance every *successful* `CoInitializeEx` —
    /// both `S_OK` and `S_FALSE` (already initialized) succeed and bump the
    /// apartment ref-count, so both must be balanced. Only a hard failure
    /// (e.g. `RPC_E_CHANGED_MODE`) means we did not initialize and must not undo.
    initialized: bool,
}

impl ComGuard {
    /// Initialize COM (multithreaded) for the current thread.
    fn new() -> Self {
        // SAFETY: standard COM init; `RPC_E_CHANGED_MODE` is non-fatal (we just
        // won't balance it). HRESULT::is_ok() covers both S_OK and S_FALSE.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        ComGuard {
            initialized: hr.is_ok(),
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: balances our own successful CoInitializeEx on this thread.
            unsafe { CoUninitialize() };
        }
    }
}

// ---------------------------------------------------------------------------
// Public surface
// ---------------------------------------------------------------------------

/// Whether the bundled HypeMuzik virtual audio device is installed.
///
/// Enumerates active render endpoints and returns `true` when one's friendly
/// name contains [`VIRTUAL_DEVICE_NAME`]. Safe to call with no audio devices
/// present (returns `false`, never panics or leaks). COM is initialized for the
/// call and balanced on return.
pub fn available() -> bool {
    let _com = ComGuard::new();
    // SAFETY: all calls are checked; failures collapse to `false` via the helper.
    unsafe { virtual_device_present().unwrap_or(false) }
}

/// `true` if any active render endpoint's friendly name contains the virtual
/// device marker. Returns `Err` on any COM failure so the caller can map it to a
/// safe `false`.
unsafe fn virtual_device_present() -> windows::core::Result<bool> {
    let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
    let collection = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
    let count = collection.GetCount()?;
    for i in 0..count {
        let Ok(device) = collection.Item(i) else {
            continue;
        };
        if let Some(name) = device_friendly_name(&device) {
            if name.contains(VIRTUAL_DEVICE_NAME) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Read an endpoint's `PKEY_Device_FriendlyName`, or `None` if unavailable.
unsafe fn device_friendly_name(device: &IMMDevice) -> Option<String> {
    let store = device.OpenPropertyStore(STGM_READ).ok()?;
    let value = store.GetValue(&PKEY_DEVICE_FRIENDLY_NAME).ok()?;
    // PROPVARIANT -> Rust String via the crate's helper (empty on type mismatch).
    let s = value.to_string();
    (!s.is_empty()).then_some(s)
}

/// Find the active render endpoint whose friendly name contains the virtual
/// device marker, returning its (device, device-id) pair.
unsafe fn find_virtual_device(
    enumerator: &IMMDeviceEnumerator,
) -> windows::core::Result<Option<(IMMDevice, String)>> {
    let collection = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
    let count = collection.GetCount()?;
    for i in 0..count {
        let Ok(device) = collection.Item(i) else {
            continue;
        };
        if let Some(name) = device_friendly_name(&device) {
            if name.contains(VIRTUAL_DEVICE_NAME) {
                let id = device_id(&device)?;
                return Ok(Some((device, id)));
            }
        }
    }
    Ok(None)
}

/// The endpoint's COM device-id string (used for `IPolicyConfig` switching).
unsafe fn device_id(device: &IMMDevice) -> windows::core::Result<String> {
    let pwstr = device.GetId()?;
    let s = pwstr.to_string().unwrap_or_default();
    // GetId allocates with CoTaskMemAlloc; free it now that we've copied it.
    CoTaskMemFree(Some(pwstr.0 as *const _));
    Ok(s)
}

/// A running Windows system-wide EQ pipeline (loopback capture → DSP → render).
/// Dropping it stops the worker thread, releases the WASAPI clients, and restores
/// the previous default render endpoint if we switched it.
pub struct WindowsSystemEq {
    running: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    /// `Some(previous_default_id)` only if we switched the default endpoint and
    /// must restore it on stop. `None` if auto-switching was skipped.
    restore_default: Option<String>,
}

impl WindowsSystemEq {
    /// Start capturing the virtual device, processing, and rendering to the real
    /// output. `state` carries the engine's live EQ/effects/power/volume params.
    pub fn start(state: Arc<ArcSwap<EngineState>>) -> Result<Self, AudioError> {
        if !available() {
            return Err(AudioError::Unavailable(
                "Windows system-wide EQ needs the HypeMuzik virtual audio device — \
                 install it from Settings (see docs/system-eq.md)."
                    .into(),
            ));
        }

        // The default-endpoint switch is best-effort and reversible; the worker
        // does all the heavy lifting on its own COM-initialized thread so the
        // WASAPI clients (which are apartment-affine) live and die there.
        let running = Arc::new(AtomicBool::new(true));

        // Switch the default render endpoint to our virtual device so apps route
        // into it. Best-effort: on failure we run anyway and tell the user to set
        // it manually. Done on this (caller) thread under its own COM guard.
        let restore_default = {
            let _com = ComGuard::new();
            // SAFETY: all COM calls checked; any failure yields `None` (skip).
            unsafe { switch_default_to_virtual() }
        };

        let run = running.clone();
        let worker = std::thread::Builder::new()
            .name("hm-system-eq-win".into())
            .spawn(move || {
                // COM is per-thread; the worker owns its own apartment for the
                // lifetime of the WASAPI clients it creates.
                let _com = ComGuard::new();
                // SAFETY: the worker confines all COM/WASAPI use to this thread.
                if let Err(e) = unsafe { run_pipeline(&state, &run) } {
                    crate::diag::log(&format!("system-eq(win) worker exited: {e}"));
                }
            })
            .map_err(|e| {
                // Roll back the default switch if the worker never started.
                if let Some(ref prev) = restore_default {
                    let _com = ComGuard::new();
                    // SAFETY: best-effort restore on the caller thread.
                    unsafe { set_default_endpoint(prev) };
                }
                AudioError::Stream(format!("system EQ worker: {e}"))
            })?;

        Ok(Self {
            running,
            worker: Some(worker),
            restore_default,
        })
    }
}

impl Drop for WindowsSystemEq {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
        // Restore the previous default endpoint (if we switched it) so apps snap
        // back to the real device. Best-effort, on this thread under its own COM.
        if let Some(prev) = self.restore_default.take() {
            let _com = ComGuard::new();
            // SAFETY: best-effort restore; failure is logged-by-omission, never panics.
            unsafe { set_default_endpoint(&prev) };
        }
    }
}

// ---------------------------------------------------------------------------
// Default-endpoint switching (IPolicyConfig)
// ---------------------------------------------------------------------------

/// Switch the default render endpoint to the virtual device, returning the
/// previous default's id so it can be restored. Returns `None` (and switches
/// nothing) if the device, the previous default, or `IPolicyConfig` is
/// unavailable — the pipeline then runs with the user setting the default
/// manually.
unsafe fn switch_default_to_virtual() -> Option<String> {
    let enumerator: IMMDeviceEnumerator =
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;

    // Our virtual device's id.
    let (_virtual_device, virtual_id) = find_virtual_device(&enumerator).ok()??;

    // The current default render endpoint — restore target. If it's already the
    // virtual device, there's nothing to switch (and nothing to restore).
    let prev_device = enumerator
        .GetDefaultAudioEndpoint(eRender, eConsole)
        .ok()?;
    let prev_id = device_id(&prev_device).ok()?;
    if prev_id == virtual_id {
        return None;
    }

    if set_default_endpoint(&virtual_id) {
        Some(prev_id)
    } else {
        None
    }
}

/// Set the default render endpoint to `device_id` via `IPolicyConfig` for all
/// three roles (console, multimedia, communications). Returns `true` on success.
/// Never panics; any COM failure yields `false`.
unsafe fn set_default_endpoint(device_id: &str) -> bool {
    let policy: IPolicyConfig =
        match CoCreateInstance(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) {
            Ok(p) => p,
            Err(_) => return false,
        };
    let wide = to_wide(device_id);
    let id = PCWSTR(wide.as_ptr());
    // eConsole = 0, eMultimedia = 1, eCommunications = 2. Set all so every app
    // category follows. Success of the console role is enough to report success.
    let ok = policy.set_default_endpoint(id, 0).is_ok();
    let _ = policy.set_default_endpoint(id, 1);
    let _ = policy.set_default_endpoint(id, 2);
    ok
}

/// UTF-16, NUL-terminated copy of `s` for `PCWSTR` arguments.
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

// ---------------------------------------------------------------------------
// Capture → DSP → render worker
// ---------------------------------------------------------------------------

/// A WASAPI client paired with the format it negotiated. The mix format is owned
/// by the OS (CoTaskMemAlloc); we copy out the fields we need then free it.
struct ClientFormat {
    channels: usize,
    sample_rate: u32,
    /// `true` when the device mix is 32-bit IEEE float (the fast path); `false`
    /// means 16-bit PCM integer, which we convert by hand.
    is_float: bool,
    bits_per_sample: u16,
}

/// Stand up the capture (loopback on the virtual device) and render (real default
/// output) clients, then run the steady-state loop until `run` clears or a fatal
/// error occurs. All COM objects live on the calling (worker) thread.
unsafe fn run_pipeline(
    state: &Arc<ArcSwap<EngineState>>,
    run: &Arc<AtomicBool>,
) -> Result<(), AudioError> {
    let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
        .map_err(|e| AudioError::Host(format!("device enumerator: {e}")))?;

    // Capture endpoint: our virtual device (loopback).
    let (virtual_device, virtual_id) = find_virtual_device(&enumerator)
        .map_err(|e| AudioError::Host(format!("enumerate endpoints: {e}")))?
        .ok_or_else(|| {
            AudioError::Unavailable("HypeMuzik virtual device not present".into())
        })?;

    // Render endpoint: the *real* default output. If the default is still our
    // virtual device (switch failed or pending), there's nothing safe to render
    // to without feeding back — bail with an actionable message.
    let render_device = enumerator
        .GetDefaultAudioEndpoint(eRender, eConsole)
        .map_err(|e| AudioError::Host(format!("default render endpoint: {e}")))?;
    let render_id =
        device_id(&render_device).map_err(|e| AudioError::Host(format!("render id: {e}")))?;
    if render_id == virtual_id {
        return Err(AudioError::Unavailable(
            "the real output is still the HypeMuzik device — set a real default \
             output so the processed audio has somewhere to go."
                .into(),
        ));
    }

    // --- Capture client: loopback + event-driven on the virtual device. ---
    let capture_client: IAudioClient = virtual_device
        .Activate(CLSCTX_ALL, None)
        .map_err(|e| AudioError::Stream(format!("activate capture client: {e}")))?;
    let capture_mix = capture_client
        .GetMixFormat()
        .map_err(|e| AudioError::Stream(format!("capture mix format: {e}")))?;
    let capture_fmt = read_format(capture_mix);
    // ~50 ms shared buffer; loopback ignores periodicity (0).
    let buffer_duration = 50 * 10_000_i64; // REFERENCE_TIME (100-ns units).
    capture_client
        .Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            buffer_duration,
            0,
            capture_mix as *const _,
            None,
        )
        .map_err(|e| AudioError::Stream(format!("init capture client: {e}")))?;
    CoTaskMemFree(Some(capture_mix as *const _));

    let capture_event =
        CreateEventW(None, false, false, PCWSTR::null()).map_err(|e| {
            AudioError::Stream(format!("capture event: {e}"))
        })?;
    let event_guard = HandleGuard(capture_event);
    capture_client
        .SetEventHandle(capture_event)
        .map_err(|e| AudioError::Stream(format!("set capture event: {e}")))?;
    let capture: IAudioCaptureClient = capture_client
        .GetService()
        .map_err(|e| AudioError::Stream(format!("capture service: {e}")))?;

    // --- Render client: shared event-driven on the real default output. ---
    let render_client: IAudioClient = render_device
        .Activate(CLSCTX_ALL, None)
        .map_err(|e| AudioError::Stream(format!("activate render client: {e}")))?;
    let render_mix = render_client
        .GetMixFormat()
        .map_err(|e| AudioError::Stream(format!("render mix format: {e}")))?;
    let render_fmt = read_format(render_mix);
    render_client
        .Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            buffer_duration,
            0,
            render_mix as *const _,
            None,
        )
        .map_err(|e| AudioError::Stream(format!("init render client: {e}")))?;
    CoTaskMemFree(Some(render_mix as *const _));

    let render_event =
        CreateEventW(None, false, false, PCWSTR::null()).map_err(|e| {
            AudioError::Stream(format!("render event: {e}"))
        })?;
    let render_event_guard = HandleGuard(render_event);
    render_client
        .SetEventHandle(render_event)
        .map_err(|e| AudioError::Stream(format!("set render event: {e}")))?;
    let render: IAudioRenderClient = render_client
        .GetService()
        .map_err(|e| AudioError::Stream(format!("render service: {e}")))?;
    let render_buffer_frames = render_client
        .GetBufferSize()
        .map_err(|e| AudioError::Stream(format!("render buffer size: {e}")))?;

    // --- MMCSS boost: tell the scheduler this is pro-audio work. ---
    let mut mmcss_task_index = 0u32;
    let mmcss = AvSetMmThreadCharacteristicsW(
        windows::core::w!("Pro Audio"),
        &mut mmcss_task_index,
    )
    .ok();
    let _mmcss_guard = mmcss.map(MmcssGuard);

    // --- Pre-sized scratch + persistent DSP chain (no per-block allocation). ---
    let mut chain = ProcessChain::standard(render_fmt.sample_rate as f32, DSP_CHANNELS);
    let mut resampler = StereoResampler::new();
    let needs_resample = capture_fmt.sample_rate != render_fmt.sample_rate;
    if needs_resample {
        resampler.set_ratio(capture_fmt.sample_rate, render_fmt.sample_rate);
    }
    // Reusable buffers: captured stereo (pre-resample) and render stereo blocks.
    let mut captured: Vec<(f32, f32)> = Vec::with_capacity(8192);
    let mut render_block: Vec<f32> = Vec::with_capacity(8192);

    capture_client
        .Start()
        .map_err(|e| AudioError::Stream(format!("start capture: {e}")))?;
    render_client
        .Start()
        .map_err(|e| AudioError::Stream(format!("start render: {e}")))?;
    let _capture_stop = StopGuard(&capture_client);
    let _render_stop = StopGuard(&render_client);

    // Steady state: wait on the capture event, drain all queued packets into the
    // stereo `captured` buffer, resample if needed, DSP, then push to the render
    // device's available space. Exits cleanly when `run` clears or a pipe breaks.
    while run.load(Ordering::Relaxed) {
        // 100 ms timeout so a stalled source still lets us re-check `run`.
        let wait = WaitForSingleObject(capture_event, 100);
        if wait != WAIT_OBJECT_0 {
            continue;
        }

        captured.clear();
        // Drain every queued capture packet.
        loop {
            let packet_frames = match capture.GetNextPacketSize() {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut frames: u32 = 0;
            let mut flags: u32 = 0;
            if capture
                .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                .is_err()
            {
                break;
            }
            let silent = (flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
            append_capture(&mut captured, data, frames as usize, &capture_fmt, silent);
            let _ = capture.ReleaseBuffer(frames);
            let _ = packet_frames; // size already reflected by `frames`
        }

        if captured.is_empty() {
            continue;
        }

        // Resample capture-rate stereo to the render rate (1:1 is a passthrough).
        render_block.clear();
        if needs_resample {
            let mut idx = 0usize;
            // Linear resampler emits ~ (out_rate/in_rate) * input frames; pull
            // until the input is exhausted (resampler holds the last frame).
            let out_estimate = (captured.len() as f64 * render_fmt.sample_rate as f64
                / capture_fmt.sample_rate as f64)
                .ceil() as usize;
            for _ in 0..out_estimate {
                match resampler.next_frame(|| {
                    let v = captured.get(idx).copied();
                    idx += 1;
                    v
                }) {
                    Some((l, r)) => {
                        render_block.push(l);
                        render_block.push(r);
                    }
                    None => break,
                }
            }
        } else {
            for &(l, r) in &captured {
                render_block.push(l);
                render_block.push(r);
            }
        }

        // Shared DSP step (master volume + power-gated ProcessChain).
        let st = state.load();
        process_block(&mut chain, &mut render_block, DSP_CHANNELS, &st);

        // Write to the render device's currently-available space, frame-by-frame
        // into the device mix format. Drop overflow (better than blocking the RT
        // thread); underflow is naturally handled by the event pacing.
        write_render(
            &render,
            &render_client,
            render_buffer_frames,
            &render_block,
            &render_fmt,
        );
    }

    // Guards (`StopGuard`, `HandleGuard`, `MmcssGuard`) tear everything down here.
    drop(event_guard);
    drop(render_event_guard);
    Ok(())
}

/// Append `frames` of device-format capture data, converted to interleaved
/// stereo f32, onto `out`. Honors the SILENT flag (emits zeros). Down/up-mixes
/// to stereo: mono duplicates, >2ch takes the first two.
unsafe fn append_capture(
    out: &mut Vec<(f32, f32)>,
    data: *mut u8,
    frames: usize,
    fmt: &ClientFormat,
    silent: bool,
) {
    if silent || data.is_null() {
        out.extend(std::iter::repeat_n((0.0f32, 0.0f32), frames));
        return;
    }
    let ch = fmt.channels.max(1);
    if fmt.is_float {
        let samples = std::slice::from_raw_parts(data as *const f32, frames * ch);
        for f in 0..frames {
            let base = f * ch;
            let l = samples[base];
            let r = if ch >= 2 { samples[base + 1] } else { l };
            out.push((l, r));
        }
    } else if fmt.bits_per_sample == 16 {
        let samples = std::slice::from_raw_parts(data as *const i16, frames * ch);
        let scale = 1.0 / 32768.0;
        for f in 0..frames {
            let base = f * ch;
            let l = samples[base] as f32 * scale;
            let r = if ch >= 2 {
                samples[base + 1] as f32 * scale
            } else {
                l
            };
            out.push((l, r));
        }
    } else {
        // Unknown depth: treat as silence rather than mis-reading memory.
        out.extend(std::iter::repeat_n((0.0f32, 0.0f32), frames));
    }
}

/// Write interleaved-stereo `block` into the render device's available buffer
/// space, converting to the device mix format. Only as many frames as currently
/// fit are written (the rest are dropped to keep the RT thread non-blocking).
unsafe fn write_render(
    render: &IAudioRenderClient,
    client: &IAudioClient,
    buffer_frames: u32,
    block: &[f32],
    fmt: &ClientFormat,
) {
    let avail_frames = block.len() / DSP_CHANNELS;
    if avail_frames == 0 {
        return;
    }
    let padding = client.GetCurrentPadding().unwrap_or(0);
    let free = buffer_frames.saturating_sub(padding) as usize;
    let to_write = avail_frames.min(free);
    if to_write == 0 {
        return;
    }
    let Ok(ptr) = render.GetBuffer(to_write as u32) else {
        return;
    };
    let ch = fmt.channels.max(1);
    if fmt.is_float {
        let dst = std::slice::from_raw_parts_mut(ptr as *mut f32, to_write * ch);
        for f in 0..to_write {
            let l = block[f * DSP_CHANNELS];
            let r = block[f * DSP_CHANNELS + 1];
            let base = f * ch;
            dst[base] = l;
            if ch >= 2 {
                dst[base + 1] = r;
            }
            // Extra channels (center/LFE/surround) get silence — we only have a
            // stereo bus; a richer downmix matrix is a future refinement.
            for c in 2..ch {
                dst[base + c] = 0.0;
            }
        }
    } else if fmt.bits_per_sample == 16 {
        let dst = std::slice::from_raw_parts_mut(ptr as *mut i16, to_write * ch);
        for f in 0..to_write {
            let l = (block[f * DSP_CHANNELS].clamp(-1.0, 1.0) * 32767.0) as i16;
            let r = (block[f * DSP_CHANNELS + 1].clamp(-1.0, 1.0) * 32767.0) as i16;
            let base = f * ch;
            dst[base] = l;
            if ch >= 2 {
                dst[base + 1] = r;
            }
            for c in 2..ch {
                dst[base + c] = 0;
            }
        }
    } else {
        // Unknown depth: release nothing meaningful, but we must still release the
        // buffer we acquired. Write silence.
        let _ = render.ReleaseBuffer(to_write as u32, 0);
        return;
    }
    let _ = render.ReleaseBuffer(to_write as u32, 0);
}

/// Copy the fields we need out of a `WAVEFORMATEX[EXTENSIBLE]` mix format. The
/// pointer remains owned by the caller (freed via `CoTaskMemFree`).
///
/// `WAVEFORMATEX[EXTENSIBLE]` are `#[repr(packed)]`, so every field is read with
/// `read_unaligned` through a raw pointer — taking `&field` on a packed struct is
/// undefined behaviour even if never dereferenced.
unsafe fn read_format(format: *const WAVEFORMATEX) -> ClientFormat {
    let format_tag = std::ptr::addr_of!((*format).wFormatTag).read_unaligned();
    let channels = std::ptr::addr_of!((*format).nChannels).read_unaligned();
    let sample_rate = std::ptr::addr_of!((*format).nSamplesPerSec).read_unaligned();
    let bits_per_sample = std::ptr::addr_of!((*format).wBitsPerSample).read_unaligned();
    let cb_size = std::ptr::addr_of!((*format).cbSize).read_unaligned();

    let mut is_float = format_tag == WAVE_FORMAT_IEEE_FLOAT as u16;
    // WAVEFORMATEXTENSIBLE carries the real subtype when wFormatTag is EXTENSIBLE.
    let extensible_extra =
        std::mem::size_of::<WAVEFORMATEXTENSIBLE>() - std::mem::size_of::<WAVEFORMATEX>();
    if format_tag == WAVE_FORMAT_EXTENSIBLE as u16 && cb_size as usize >= extensible_extra {
        let wfex = format as *const WAVEFORMATEXTENSIBLE;
        let sub_format = std::ptr::addr_of!((*wfex).SubFormat).read_unaligned();
        is_float = sub_format == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT;
    }
    ClientFormat {
        channels: channels as usize,
        sample_rate,
        is_float,
        bits_per_sample,
    }
}

// ---------------------------------------------------------------------------
// RAII guards for the worker's OS handles / clients
// ---------------------------------------------------------------------------

/// Closes an event `HANDLE` on drop.
struct HandleGuard(HANDLE);
impl Drop for HandleGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: the handle came from CreateEventW and is owned here.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

/// Stops an `IAudioClient` on drop (idempotent; errors ignored).
struct StopGuard<'a>(&'a IAudioClient);
impl Drop for StopGuard<'_> {
    fn drop(&mut self) {
        // SAFETY: Stop is safe to call on a started or stopped client.
        let _ = unsafe { self.0.Stop() };
    }
}

/// Reverts the MMCSS thread characteristics on drop.
struct MmcssGuard(HANDLE);
impl Drop for MmcssGuard {
    fn drop(&mut self) {
        // SAFETY: balances our AvSetMmThreadCharacteristicsW on this thread.
        let _ = unsafe { AvRevertMmThreadCharacteristics(self.0) };
    }
}

// SAFETY: the worker confines all COM/WASAPI objects to its own thread; only the
// `running` AtomicBool and the `Arc<ArcSwap<EngineState>>` cross the boundary,
// and both are `Send`/`Sync`. `WindowsSystemEq` itself holds no COM pointers.
unsafe impl Send for WindowsSystemEq {}
