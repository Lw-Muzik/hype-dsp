//! macOS system-wide capture via Core Audio process taps (macOS 14.4+).
//!
//! Creates a **global tap** that mutes every process *except* HypeMuzik (so there
//! is no feedback from our own output), wraps it in a private aggregate device,
//! and runs an IO callback that pushes the tapped audio into a lock-free ring.
//! [`SystemTapSource`] reads from that ring as a normal [`AudioSource`], so the
//! existing chain processes the whole system's audio and plays it back — a true
//! system-wide EQ, with no driver to install.
//!
//! Requires the user to grant the **audio capture** permission
//! (`NSAudioCaptureUsageDescription`), and the app must be code-signed for the
//! grant to persist. See `docs/audio-driver.md`.
//!
//! NOTE: intricate Core Audio FFI against `objc2-core-audio` 0.3 — compile
//! -verified; its runtime behavior must be validated on a signed build with the
//! permission granted.

#![cfg(target_os = "macos")]

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use objc2::runtime::ProtocolObject;
use objc2::AllocAnyThread;
use objc2_core_audio::{
    kAudioDevicePropertyDeviceIsAlive, kAudioHardwarePropertyDefaultOutputDevice,
    kAudioHardwarePropertyTranslatePIDToProcessObject, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject, kAudioTapPropertyFormat,
    kAudioTapPropertyUID, AudioDeviceCreateIOProcID, AudioDeviceIOProcID, AudioDeviceStart,
    AudioDeviceStop, AudioHardwareCreateAggregateDevice, AudioHardwareCreateProcessTap,
    AudioHardwareDestroyAggregateDevice, AudioHardwareDestroyProcessTap,
    AudioObjectAddPropertyListener, AudioObjectGetPropertyData, AudioObjectID,
    AudioObjectPropertyAddress, AudioObjectRemovePropertyListener, CATapDescription,
    CATapMuteBehavior,
};
use objc2_core_audio_types::{AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp};
use objc2_core_foundation::{CFDictionary, CFRetained, CFString};
use objc2_foundation::{NSMutableArray, NSMutableDictionary, NSNumber, NSObject, NSString};
use rtrb::RingBuffer;

use crate::error::AudioError;
use crate::resampler::StereoResampler;
use crate::{AudioSource, StreamFormat};

/// Monotonic counter giving every tap+aggregate we create a process-unique UID.
///
/// CoreAudio keys aggregate devices by [`kAudioAggregateDeviceUIDKey`]; teardown
/// (`AudioHardwareDestroyAggregateDevice`) is **asynchronous**, so reusing a fixed
/// UID lets a fresh create race the still-pending destroy of the previous one and
/// fail with `kAudioHardwareIllegalOperationError` (`'nope'`, status 1852797029).
/// A unique UID per creation makes back-to-back rebuilds collision-free — the
/// same approach the per-app capture path uses (`HypeMuzikAppTap-{tap_id}`).
static TAP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Upper bound on the `f32` samples we will read out of a single tap callback
/// buffer. Real tap buffers are a few thousand frames; this clamp is a hard
/// defence against a corrupt `mDataByteSize` (e.g. under memory pressure) turning
/// `slice::from_raw_parts` into a wild, process-killing out-of-bounds read.
const MAX_TAP_SAMPLES: usize = 1 << 20; // 1,048,576 f32 = 4 MiB

/// `f32` sample count implied by a Core Audio `mDataByteSize`, clamped to
/// [`MAX_TAP_SAMPLES`]. Pure so it is unit-tested without Core Audio.
#[inline]
fn samples_in(byte_size: u32) -> usize {
    (byte_size as usize / size_of::<f32>()).min(MAX_TAP_SAMPLES)
}

/// Allocation-free liveness + first-callback telemetry for the capture io_proc.
///
/// The io_proc runs on a real-time Core Audio thread, where file I/O, string
/// formatting, and heap allocation are all forbidden — any of them can stall the
/// callback or, under memory pressure, abort the process across the
/// `extern "C-unwind"` boundary. So the io_proc only ever does atomic stores into
/// this struct; a *normal* thread (the engine's tap watchdog) reads them and logs.
///
/// [`beat`](Self::beat) is the capture-side heartbeat: the engine watchdog watches
/// it to catch a starved/dead capture proc that would drain the ring and play
/// silence while the *output* heartbeat still looks healthy (leaving every other
/// app muted with no other symptom).
#[derive(Debug, Default)]
pub struct CaptureTelemetry {
    /// Bumped once per io_proc callback — the capture-side liveness heartbeat.
    beat: AtomicU64,
    /// Bumped by the engine each time it (re)builds a tap, so the watchdog can log
    /// the first-callback layout exactly once per tap instance.
    generation: AtomicU64,
    /// Cleared on each (re)build; set by the io_proc once it has recorded the
    /// first callback's buffer layout into the fields below.
    layout_ready: AtomicBool,
    first_buffers: AtomicU32,
    first_channels: AtomicU32,
    first_bytes: AtomicU32,
    first_peak_bits: AtomicU32,
}

impl CaptureTelemetry {
    /// The capture heartbeat. Monotonic while the tap runs; a frozen value means
    /// the capture io_proc has stalled or died.
    pub fn beat(&self) -> u64 {
        self.beat.load(Ordering::Relaxed)
    }

    /// Current tap generation (bumped on every (re)build).
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    /// Mark the start of a fresh tap instance: re-arm first-callback capture and
    /// bump the generation. Called (off the RT thread) just before building a tap.
    pub fn begin_generation(&self) {
        self.layout_ready.store(false, Ordering::Relaxed);
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// The first callback's `(n_buffers, channels, bytes, peak)` once recorded, for
    /// one-shot diagnostics from a non-RT thread. `None` until the io_proc has run.
    pub fn first_layout(&self) -> Option<(u32, u32, u32, f32)> {
        self.layout_ready.load(Ordering::Relaxed).then(|| {
            (
                self.first_buffers.load(Ordering::Relaxed),
                self.first_channels.load(Ordering::Relaxed),
                self.first_bytes.load(Ordering::Relaxed),
                f32::from_bits(self.first_peak_bits.load(Ordering::Relaxed)),
            )
        })
    }
}

/// IO callback context (heap-owned, freed on drop).
struct TapContext {
    producer: rtrb::Producer<f32>,
    /// Shared with the engine + watchdog; the io_proc only ever *stores* into it.
    tel: Arc<CaptureTelemetry>,
}

/// A live system-audio source backed by a Core Audio tap + aggregate device.
pub struct SystemTapSource {
    consumer: rtrb::Consumer<f32>,
    position_frames: Arc<AtomicU64>,
    tap_id: AudioObjectID,
    aggregate_id: AudioObjectID,
    proc_id: AudioDeviceIOProcID,
    ctx: *mut TapContext,
    started: AtomicBool,
    /// Tap capture rate (Hz), read from the tap format at construction.
    capture_rate: u32,
    /// Converts the tap's capture rate to the output device rate.
    resampler: StereoResampler,
}

// The CoreAudio object IDs and the boxed context are only touched on
// create/drop, making this safe to hand to the engine control thread.
unsafe impl Send for SystemTapSource {}

fn addr(selector: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    }
}

/// Translate this process's PID to its Core Audio process AudioObjectID.
fn own_process_object() -> Result<AudioObjectID, AudioError> {
    let pid: i32 = std::process::id() as i32;
    let address = addr(kAudioHardwarePropertyTranslatePIDToProcessObject);
    let mut object: AudioObjectID = 0;
    let mut size = size_of::<AudioObjectID>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject as AudioObjectID,
            NonNull::from(&address),
            size_of::<i32>() as u32,
            &pid as *const i32 as *const c_void,
            NonNull::from(&mut size),
            NonNull::new(&mut object as *mut AudioObjectID as *mut c_void).unwrap(),
        )
    };
    if status != 0 {
        return Err(AudioError::Unavailable(format!(
            "could not resolve own audio process (status {status})"
        )));
    }
    Ok(object)
}

fn tap_uid_string(tap_id: AudioObjectID) -> Result<String, AudioError> {
    let address = addr(kAudioTapPropertyUID);
    let mut uid: *const CFString = std::ptr::null();
    let mut size = size_of::<*const CFString>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            tap_id,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new(&mut uid as *mut *const CFString as *mut c_void).unwrap(),
        )
    };
    if status != 0 || uid.is_null() {
        return Err(AudioError::Unavailable(format!(
            "could not read tap UID (status {status})"
        )));
    }
    let uid = unsafe { CFRetained::from_raw(NonNull::new(uid as *mut CFString).unwrap()) };
    Ok(uid.to_string())
}

fn tap_format(tap_id: AudioObjectID) -> Result<AudioStreamBasicDescription, AudioError> {
    let address = addr(kAudioTapPropertyFormat);
    let mut asbd: AudioStreamBasicDescription = unsafe { std::mem::zeroed() };
    let mut size = size_of::<AudioStreamBasicDescription>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            tap_id,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new(&mut asbd as *mut _ as *mut c_void).unwrap(),
        )
    };
    if status != 0 {
        return Err(AudioError::Unavailable(format!(
            "could not read tap format (status {status})"
        )));
    }
    Ok(asbd)
}

/// IO callback: push the tapped input buffer (interleaved f32) into the ring.
///
/// REAL-TIME HOT PATH. This runs on a Core Audio io thread, so it must not
/// allocate, take locks, do I/O, format strings, or panic — a panic unwinding
/// into Core Audio's C frames would abort the whole process (which, under memory
/// pressure, is exactly how the tap used to crash). Every array access below is
/// length-checked or `.get()`-guarded, every `from_raw_parts` length is clamped by
/// [`samples_in`], and there are no `unwrap`s: no code path here can panic.
unsafe extern "C-unwind" fn io_proc(
    _device: AudioObjectID,
    _now: NonNull<AudioTimeStamp>,
    input_data: NonNull<AudioBufferList>,
    _input_time: NonNull<AudioTimeStamp>,
    _output_data: NonNull<AudioBufferList>,
    _output_time: NonNull<AudioTimeStamp>,
    client_data: *mut c_void,
) -> i32 {
    if client_data.is_null() {
        return 0;
    }
    let ctx = &mut *(client_data as *mut TapContext);
    // Capture-side liveness heartbeat: one relaxed add, RT-safe. The engine
    // watchdog watches this to notice a starved/dead capture proc that would
    // otherwise leave the system muted with a still-healthy output heartbeat.
    ctx.tel.beat.fetch_add(1, Ordering::Relaxed);

    let list = input_data.as_ref();
    let n_buffers = list.mNumberBuffers as usize;
    if n_buffers == 0 {
        return 0;
    }
    // mBuffers is a variable-length array; only `[AudioBuffer; 1]` is declared.
    let buffers = std::slice::from_raw_parts(list.mBuffers.as_ptr(), n_buffers);
    let first = &buffers[0];
    if first.mData.is_null() {
        return 0;
    }

    // First-callback layout telemetry — recorded into preallocated atomics only
    // (a non-RT thread logs it later). No format!/log/alloc on this hot path.
    if !ctx.tel.layout_ready.load(Ordering::Relaxed) {
        let n = samples_in(first.mDataByteSize);
        let s = std::slice::from_raw_parts(first.mData as *const f32, n);
        let peak = s.iter().fold(0.0_f32, |m, v| m.max(v.abs()));
        ctx.tel.first_buffers.store(n_buffers as u32, Ordering::Relaxed);
        ctx.tel
            .first_channels
            .store(first.mNumberChannels, Ordering::Relaxed);
        ctx.tel
            .first_bytes
            .store(first.mDataByteSize, Ordering::Relaxed);
        ctx.tel
            .first_peak_bits
            .store(peak.to_bits(), Ordering::Relaxed);
        ctx.tel.layout_ready.store(true, Ordering::Relaxed);
    }

    // Verified layout: a single interleaved packed-float stereo buffer
    // (n_buffers=1, mChannelsPerFrame=2). We still handle a planar fallback, but
    // in BOTH paths L and R are pushed as a pair only when two ring slots are
    // free — so a full ring drops whole frames and never desyncs the channels.
    if n_buffers >= 2 {
        // Defensive: non-interleaved planes (not observed from the tap).
        let frames = samples_in(first.mDataByteSize);
        let left = std::slice::from_raw_parts(first.mData as *const f32, frames);
        let right_buf = &buffers[1];
        let right = if right_buf.mData.is_null() {
            left
        } else {
            let rn = samples_in(right_buf.mDataByteSize).min(frames);
            std::slice::from_raw_parts(right_buf.mData as *const f32, rn)
        };
        for (i, &l) in left.iter().enumerate() {
            if ctx.producer.slots() < 2 {
                break; // ring full: drop remaining frames, stay channel-aligned
            }
            let r = right.get(i).copied().unwrap_or(l);
            let _ = ctx.producer.push(l);
            let _ = ctx.producer.push(r);
        }
    } else {
        // Single buffer: interleaved by channel count.
        let in_ch = first.mNumberChannels.max(1) as usize;
        let count = samples_in(first.mDataByteSize);
        let samples = std::slice::from_raw_parts(first.mData as *const f32, count);
        for frame in samples.chunks(in_ch) {
            if ctx.producer.slots() < 2 {
                break; // ring full: drop remaining frames, stay channel-aligned
            }
            let l = frame.first().copied().unwrap_or(0.0);
            let r = frame.get(1).copied().unwrap_or(l);
            let _ = ctx.producer.push(l);
            let _ = ctx.producer.push(r);
        }
    }
    0
}

impl SystemTapSource {
    /// Build the tap + aggregate device and start capture. May trigger the
    /// audio-capture permission prompt on first use.
    ///
    /// `tel` is the shared capture telemetry: its heartbeat is bumped by the io
    /// proc and watched by the engine watchdog, and a fresh generation is started
    /// here so first-callback diagnostics are re-armed for this new tap instance.
    pub fn new(device_rate: u32, tel: Arc<CaptureTelemetry>) -> Result<Self, AudioError> {
        crate::diag::log(&format!(
            "=== SystemTapSource::new(device_rate={device_rate}) ==="
        ));
        // Re-arm first-callback capture for this (re)built tap (non-RT).
        tel.begin_generation();
        let own = own_process_object()?;
        // Process-unique suffix: distinct UID per (re)build so a fresh aggregate
        // never collides with the async teardown of the previous one.
        let seq = TAP_SEQ.fetch_add(1, Ordering::Relaxed);

        // Global tap that mutes everything except us.
        let exclude = NSMutableArray::<NSNumber>::new();
        exclude.addObject(&NSNumber::new_u32(own));
        let description = unsafe {
            let d = CATapDescription::initStereoGlobalTapButExcludeProcesses(
                CATapDescription::alloc(),
                &exclude,
            );
            d.setMuteBehavior(CATapMuteBehavior::Muted);
            d.setName(&NSString::from_str(&format!("HypeMuzik System Tap {seq}")));
            d
        };

        let mut tap_id: AudioObjectID = 0;
        let status =
            unsafe { AudioHardwareCreateProcessTap(Some(&description), &mut tap_id as *mut _) };
        if status != 0 || tap_id == 0 {
            return Err(AudioError::Unavailable(format!(
                "system audio capture was denied or failed (status {status}). Grant the \
                 audio-capture permission and run a signed build."
            )));
        }

        let uid = tap_uid_string(tap_id)?;
        let fmt = tap_format(tap_id)?;
        crate::diag::log(&format!(
            "tap_format: sr={} ch/frame={} bytes/frame={} bytes/packet={} frames/packet={} \
             bits/ch={} flags={:#010x} (NonInterleaved bit 0x20={})",
            fmt.mSampleRate,
            fmt.mChannelsPerFrame,
            fmt.mBytesPerFrame,
            fmt.mBytesPerPacket,
            fmt.mFramesPerPacket,
            fmt.mBitsPerChannel,
            fmt.mFormatFlags,
            (fmt.mFormatFlags & 0x20) != 0,
        ));

        let aggregate_id = match create_aggregate(&uid, seq) {
            Ok(id) => id,
            Err(e) => {
                unsafe { AudioHardwareDestroyProcessTap(tap_id) };
                return Err(e);
            }
        };

        let capacity = (device_rate.max(8_000) as usize) * 2 * 2;
        let (producer, consumer) = RingBuffer::<f32>::new(capacity);
        let ctx = Box::into_raw(Box::new(TapContext { producer, tel }));

        let mut proc_id: AudioDeviceIOProcID = None;
        let status = unsafe {
            AudioDeviceCreateIOProcID(
                aggregate_id,
                Some(io_proc),
                ctx as *mut c_void,
                NonNull::from(&mut proc_id),
            )
        };
        if status != 0 {
            unsafe {
                drop(Box::from_raw(ctx));
                AudioHardwareDestroyAggregateDevice(aggregate_id);
                AudioHardwareDestroyProcessTap(tap_id);
            }
            return Err(AudioError::Stream(format!(
                "IO proc creation failed ({status})"
            )));
        }

        let capture_rate = if fmt.mSampleRate > 0.0 {
            fmt.mSampleRate as u32
        } else {
            device_rate
        };
        let source = Self {
            consumer,
            position_frames: Arc::new(AtomicU64::new(0)),
            tap_id,
            aggregate_id,
            proc_id,
            ctx,
            started: AtomicBool::new(false),
            capture_rate,
            resampler: StereoResampler::new(),
        };

        let status = unsafe { AudioDeviceStart(aggregate_id, proc_id) };
        if status != 0 {
            return Err(AudioError::Stream(format!(
                "could not start tap device ({status})"
            )));
        }
        source.started.store(true, Ordering::Relaxed);
        crate::diag::log("SystemTapSource: AudioDeviceStart OK — tap running");
        Ok(source)
    }
}

fn create_aggregate(tap_uid: &str, seq: u64) -> Result<AudioObjectID, AudioError> {
    use objc2_core_audio::{
        kAudioAggregateDeviceIsPrivateKey, kAudioAggregateDeviceNameKey,
        kAudioAggregateDeviceTapListKey, kAudioAggregateDeviceUIDKey, kAudioSubTapUIDKey,
    };

    fn key(c: &std::ffi::CStr) -> objc2::rc::Retained<NSString> {
        NSString::from_str(c.to_str().unwrap_or_default())
    }

    // taps = [ { "uid": <tap uid> } ]
    let k_subtap = key(kAudioSubTapUIDKey);
    let tap_entry = NSMutableDictionary::<NSString, NSObject>::new();
    let taps = NSMutableArray::<NSObject>::new();
    let k_uid = key(kAudioAggregateDeviceUIDKey);
    let k_name = key(kAudioAggregateDeviceNameKey);
    let k_private = key(kAudioAggregateDeviceIsPrivateKey);
    let k_taps = key(kAudioAggregateDeviceTapListKey);
    // Process-unique UID so a rebuild never collides with the previous
    // aggregate's asynchronous teardown (which yields status 'nope').
    let agg_uid = format!("HypeMuzikSystemTap-{}-{}", std::process::id(), seq);
    let dict = NSMutableDictionary::<NSString, NSObject>::new();
    unsafe {
        tap_entry.setObject_forKey(
            &NSString::from_str(tap_uid),
            ProtocolObject::from_ref(&*k_subtap),
        );
        taps.addObject(&tap_entry);
        dict.setObject_forKey(
            &NSString::from_str(&agg_uid),
            ProtocolObject::from_ref(&*k_uid),
        );
        dict.setObject_forKey(
            &NSString::from_str("HypeMuzik System Tap"),
            ProtocolObject::from_ref(&*k_name),
        );
        dict.setObject_forKey(
            &NSNumber::new_bool(true),
            ProtocolObject::from_ref(&*k_private),
        );
        dict.setObject_forKey(&taps, ProtocolObject::from_ref(&*k_taps));
    }

    // NSDictionary is toll-free bridged to CFDictionary.
    let cf: &CFDictionary =
        unsafe { &*(objc2::rc::Retained::as_ptr(&dict) as *const CFDictionary) };
    let mut aggregate_id: AudioObjectID = 0;
    let status =
        unsafe { AudioHardwareCreateAggregateDevice(cf, NonNull::from(&mut aggregate_id)) };
    if status != 0 || aggregate_id == 0 {
        return Err(AudioError::Unavailable(format!(
            "could not create the tap aggregate device (status {status})"
        )));
    }
    Ok(aggregate_id)
}

impl Drop for SystemTapSource {
    fn drop(&mut self) {
        unsafe {
            if self.started.load(Ordering::Relaxed) {
                AudioDeviceStop(self.aggregate_id, self.proc_id);
            }
            AudioHardwareDestroyAggregateDevice(self.aggregate_id);
            AudioHardwareDestroyProcessTap(self.tap_id);
            if !self.ctx.is_null() {
                drop(Box::from_raw(self.ctx));
            }
        }
    }
}

impl AudioSource for SystemTapSource {
    fn start(&mut self, format: StreamFormat) -> Result<(), AudioError> {
        let out_rate = format.sample_rate.max(1);
        self.resampler.set_ratio(self.capture_rate, out_rate);
        crate::diag::log(&format!(
            "SystemTapSource::start: capture_rate={} out_rate={}",
            self.capture_rate, out_rate
        ));
        Ok(())
    }

    fn read(&mut self, out: &mut [f32], channels: usize) -> usize {
        if channels == 0 {
            return 0;
        }
        let frames = out.len() / channels;

        // Hold off until a little audio is buffered, so the resampler can prime
        // and we don't start on an empty ring.
        if self.consumer.slots() < 4 && self.position_frames.load(Ordering::Relaxed) == 0 {
            for s in out.iter_mut() {
                *s = 0.0;
            }
            return 0;
        }

        // Split-borrow the two fields the resampler needs so the pull closure can
        // capture the ring consumer independently of `&mut self.resampler`.
        let rs = &mut self.resampler;
        let consumer = &mut self.consumer;

        let mut produced = 0;
        for f in 0..frames {
            let base = f * channels;
            let (l, r) = rs
                .next_frame(|| {
                    if consumer.slots() >= 2 {
                        Some((
                            consumer.pop().unwrap_or(0.0),
                            consumer.pop().unwrap_or(0.0),
                        ))
                    } else {
                        None
                    }
                })
                .unwrap_or((0.0, 0.0));

            if channels == 1 {
                out[base] = 0.5 * (l + r);
            } else {
                out[base] = l;
                out[base + 1] = r;
                for ch in out.iter_mut().take(base + channels).skip(base + 2) {
                    *ch = 0.0;
                }
            }
            produced += 1;
        }
        self.position_frames
            .fetch_add(produced as u64, Ordering::Relaxed);
        produced
    }

    fn stop(&mut self) {}

    fn position(&self) -> usize {
        self.position_frames.load(Ordering::Relaxed) as usize
    }

    fn is_live(&self) -> bool {
        true
    }
}

/// Whether the system-tap path is available on this OS build (always true on
/// macOS; the runtime permission is requested when capture starts).
pub fn available() -> bool {
    true
}

/// The system's current default **output** device, or `None` if it can't be
/// resolved. The watchdog compares this against the device the tap was built on to
/// tell a real default-device change (rebuild required) from a mere output stall.
pub fn default_output_device_id() -> Option<AudioObjectID> {
    let address = addr(kAudioHardwarePropertyDefaultOutputDevice);
    let mut device: AudioObjectID = 0;
    let mut size = size_of::<AudioObjectID>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject as AudioObjectID,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new(&mut device as *mut AudioObjectID as *mut c_void).unwrap(),
        )
    };
    (status == 0 && device != 0).then_some(device)
}

/// Whether `device` currently reports itself alive
/// (`kAudioDevicePropertyDeviceIsAlive`). Used before tearing a tap down: an
/// *alive* device that merely stopped feeding the output callback is CPU
/// starvation, not death, so the watchdog must NOT rebuild (which would churn
/// coreaudiod and drop the EQ).
///
/// Fail-open: an unknown (`0`) device or a failed query returns `true`, so a
/// transient probe glitch never triggers a needless — and possibly muting —
/// teardown.
pub fn device_is_alive(device: AudioObjectID) -> bool {
    if device == 0 {
        return true;
    }
    let address = addr(kAudioDevicePropertyDeviceIsAlive);
    let mut alive: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            device,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new(&mut alive as *mut u32 as *mut c_void).unwrap(),
        )
    };
    status != 0 || alive != 0
}

/// Heap context for the default-output-device listener: a callback invoked from
/// a Core Audio notification thread. Boxed and handed to the registration as
/// `clientData`; reclaimed in [`DefaultOutputListener::drop`].
struct ListenerCtx {
    on_change: Box<dyn Fn() + Send>,
}

/// Core Audio property-listener proc. The HAL invokes a given (proc, clientData)
/// registration serially, so reading the shared `ListenerCtx` here is sound.
///
/// This runs on a Core Audio notification thread and unwinding into the HAL's C
/// frames would abort the process, so `on_change` MUST be allocation-free and
/// panic-free — the engine passes a closure that only flips an `AtomicBool`; the
/// watchdog thread does the actual (allocating) channel send.
unsafe extern "C-unwind" fn default_device_listener_proc(
    _object: AudioObjectID,
    _num_addresses: u32,
    _addresses: NonNull<AudioObjectPropertyAddress>,
    client_data: *mut c_void,
) -> i32 {
    if client_data.is_null() {
        return 0;
    }
    let ctx = &*(client_data as *const ListenerCtx);
    (ctx.on_change)();
    0
}

/// Fires its callback whenever the system's **default output device** changes, so
/// the engine can rebuild the tap on the new device *immediately* and
/// deterministically — rather than waiting for the heartbeat watchdog to notice
/// the resulting output-stream stall. Keep the watchdog too: it backstops changes
/// this listener can't see (e.g. a sample-rate change on the *same* device).
///
/// The listener is removed and its context freed on drop.
pub struct DefaultOutputListener {
    ctx: *mut ListenerCtx,
}

// The `ctx` pointer is touched only on create and on `&mut self` drop — never
// through a shared `&self` — so the handle is safe to both move and share across
// threads (the engine that owns it is `Send + Sync`). The Core Audio callback
// dereferences `ctx` independently; the HAL serialises those invocations.
unsafe impl Send for DefaultOutputListener {}
unsafe impl Sync for DefaultOutputListener {}

impl DefaultOutputListener {
    /// Register a default-output-device listener on the system object. `on_change`
    /// runs on a Core Audio notification thread, so it MUST be allocation-free and
    /// panic-free (see [`default_device_listener_proc`]) — flip an atomic only.
    /// Returns an error if registration fails; the caller can treat that as "no
    /// proactive recovery" and rely on the watchdog.
    pub fn new(on_change: Box<dyn Fn() + Send>) -> Result<Self, AudioError> {
        let ctx = Box::into_raw(Box::new(ListenerCtx { on_change }));
        let address = addr(kAudioHardwarePropertyDefaultOutputDevice);
        let status = unsafe {
            AudioObjectAddPropertyListener(
                kAudioObjectSystemObject as AudioObjectID,
                NonNull::from(&address),
                Some(default_device_listener_proc),
                ctx as *mut c_void,
            )
        };
        if status != 0 {
            unsafe { drop(Box::from_raw(ctx)) };
            return Err(AudioError::Unavailable(format!(
                "could not register default-output-device listener (status {status})"
            )));
        }
        Ok(Self { ctx })
    }
}

impl Drop for DefaultOutputListener {
    fn drop(&mut self) {
        let address = addr(kAudioHardwarePropertyDefaultOutputDevice);
        unsafe {
            // Must pass the same (proc, clientData) pair used to register.
            AudioObjectRemovePropertyListener(
                kAudioObjectSystemObject as AudioObjectID,
                NonNull::from(&address),
                Some(default_device_listener_proc),
                self.ctx as *mut c_void,
            );
            drop(Box::from_raw(self.ctx));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_in_divides_bytes_by_f32_width() {
        assert_eq!(samples_in(0), 0);
        assert_eq!(samples_in(4), 1);
        assert_eq!(samples_in(4096), 1024);
        // Non-multiples truncate toward zero (partial trailing float ignored).
        assert_eq!(samples_in(7), 1);
    }

    #[test]
    fn samples_in_clamps_a_corrupt_bytesize() {
        // A wild/corrupt mDataByteSize must never yield a slice length past the
        // hard cap — this is the guard against an out-of-bounds read killing us.
        assert_eq!(samples_in(u32::MAX), MAX_TAP_SAMPLES);
    }

    #[test]
    fn device_is_alive_fails_open_for_unknown_device() {
        // Device id 0 (not yet recorded) must be treated as alive so the watchdog
        // never tears down a tap it can't positively confirm is dead.
        assert!(device_is_alive(0));
    }

    #[test]
    fn capture_telemetry_generation_and_layout_roundtrip() {
        let tel = CaptureTelemetry::default();
        assert_eq!(tel.beat(), 0);
        assert_eq!(tel.generation(), 0);
        assert!(tel.first_layout().is_none());

        tel.begin_generation();
        assert_eq!(tel.generation(), 1);
        // begin_generation re-arms layout capture (stays None until the io proc runs).
        assert!(tel.first_layout().is_none());

        // Simulate what the io proc records (via the internal atomics).
        tel.first_buffers.store(1, Ordering::Relaxed);
        tel.first_channels.store(2, Ordering::Relaxed);
        tel.first_bytes.store(4096, Ordering::Relaxed);
        tel.first_peak_bits.store(0.5f32.to_bits(), Ordering::Relaxed);
        tel.layout_ready.store(true, Ordering::Relaxed);
        assert_eq!(tel.first_layout(), Some((1, 2, 4096, 0.5)));

        // A fresh generation clears the layout again.
        tel.begin_generation();
        assert_eq!(tel.generation(), 2);
        assert!(tel.first_layout().is_none());
    }
}
