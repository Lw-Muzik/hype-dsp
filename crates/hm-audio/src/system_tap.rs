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
    kAudioHardwarePropertyTranslatePIDToProcessObject, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject, kAudioTapPropertyFormat,
    kAudioTapPropertyUID, AudioDeviceCreateIOProcID, AudioDeviceIOProcID, AudioDeviceStart,
    AudioDeviceStop, AudioHardwareCreateAggregateDevice, AudioHardwareCreateProcessTap,
    AudioHardwareDestroyAggregateDevice, AudioHardwareDestroyProcessTap,
    AudioObjectGetPropertyData, AudioObjectID, AudioObjectPropertyAddress, CATapDescription,
    CATapMuteBehavior,
};
use objc2_core_audio_types::{AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp};
use objc2_core_foundation::{CFDictionary, CFRetained, CFString};
use objc2_foundation::{NSMutableArray, NSMutableDictionary, NSNumber, NSObject, NSString};
use rtrb::RingBuffer;

use crate::error::AudioError;
use crate::{AudioSource, StreamFormat};

/// IO callback context (heap-owned, freed on drop).
struct TapContext {
    producer: rtrb::Producer<f32>,
    dbg_calls: AtomicU32,
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

    // --- DIAGNOSTIC (temporary): log the first few callbacks' buffer layout ---
    if crate::diag::first_n(&ctx.dbg_calls, 6) {
        let f32s = first.mDataByteSize as usize / size_of::<f32>();
        let s = std::slice::from_raw_parts(first.mData as *const f32, f32s);
        let peak = s.iter().fold(0.0_f32, |m, v| m.max(v.abs()));
        let b1ch = if n_buffers >= 2 {
            buffers[1].mNumberChannels as i32
        } else {
            -1
        };
        crate::diag::log(&format!(
            "io_proc: n_buffers={} buf0.ch={} buf0.bytes={} buf1.ch={} buf0_peak={:.4}",
            n_buffers, first.mNumberChannels, first.mDataByteSize, b1ch, peak,
        ));
    }

    // Verified layout: a single interleaved packed-float stereo buffer
    // (n_buffers=1, mChannelsPerFrame=2). We still handle a planar fallback, but
    // in BOTH paths L and R are pushed as a pair only when two ring slots are
    // free — so a full ring drops whole frames and never desyncs the channels.
    if n_buffers >= 2 {
        // Defensive: non-interleaved planes (not observed from the tap).
        let frames = first.mDataByteSize as usize / size_of::<f32>();
        let left = std::slice::from_raw_parts(first.mData as *const f32, frames);
        let right_buf = &buffers[1];
        let right = if right_buf.mData.is_null() {
            left
        } else {
            let rn = (right_buf.mDataByteSize as usize / size_of::<f32>()).min(frames);
            std::slice::from_raw_parts(right_buf.mData as *const f32, rn)
        };
        for i in 0..frames {
            if ctx.producer.slots() < 2 {
                break; // ring full: drop remaining frames, stay channel-aligned
            }
            let l = left[i];
            let r = right.get(i).copied().unwrap_or(l);
            let _ = ctx.producer.push(l);
            let _ = ctx.producer.push(r);
        }
    } else {
        // Single buffer: interleaved by channel count.
        let in_ch = first.mNumberChannels.max(1) as usize;
        let count = first.mDataByteSize as usize / size_of::<f32>();
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
    pub fn new(device_rate: u32) -> Result<Self, AudioError> {
        crate::diag::log(&format!(
            "=== SystemTapSource::new(device_rate={device_rate}) ==="
        ));
        let own = own_process_object()?;

        // Global tap that mutes everything except us.
        let exclude = NSMutableArray::<NSNumber>::new();
        exclude.addObject(&NSNumber::new_u32(own));
        let description = unsafe {
            let d = CATapDescription::initStereoGlobalTapButExcludeProcesses(
                CATapDescription::alloc(),
                &exclude,
            );
            d.setMuteBehavior(CATapMuteBehavior::Muted);
            d.setName(&NSString::from_str("HypeMuzik System Tap"));
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

        let aggregate_id = match create_aggregate(&uid) {
            Ok(id) => id,
            Err(e) => {
                unsafe { AudioHardwareDestroyProcessTap(tap_id) };
                return Err(e);
            }
        };

        let capacity = (device_rate.max(8_000) as usize) * 2 * 2;
        let (producer, consumer) = RingBuffer::<f32>::new(capacity);
        let ctx = Box::into_raw(Box::new(TapContext {
            producer,
            dbg_calls: AtomicU32::new(0),
        }));

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

        let source = Self {
            consumer,
            position_frames: Arc::new(AtomicU64::new(0)),
            tap_id,
            aggregate_id,
            proc_id,
            ctx,
            started: AtomicBool::new(false),
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

fn create_aggregate(tap_uid: &str) -> Result<AudioObjectID, AudioError> {
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
    let dict = NSMutableDictionary::<NSString, NSObject>::new();
    unsafe {
        tap_entry.setObject_forKey(
            &NSString::from_str(tap_uid),
            ProtocolObject::from_ref(&*k_subtap),
        );
        taps.addObject(&tap_entry);
        dict.setObject_forKey(
            &NSString::from_str("HypeMuzikSystemTap"),
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
    fn start(&mut self, _format: StreamFormat) -> Result<(), AudioError> {
        Ok(())
    }

    fn read(&mut self, out: &mut [f32], channels: usize) -> usize {
        if channels == 0 {
            return 0;
        }
        let frames = out.len() / channels;
        let mut produced = 0;
        for f in 0..frames {
            let base = f * channels;
            if self.consumer.slots() >= 2 {
                let l = self.consumer.pop().unwrap_or(0.0);
                let r = self.consumer.pop().unwrap_or(0.0);
                produced += 1;
                if channels == 1 {
                    out[base] = 0.5 * (l + r);
                } else {
                    out[base] = l;
                    out[base + 1] = r;
                    for ch in out.iter_mut().take(base + channels).skip(base + 2) {
                        *ch = 0.0;
                    }
                }
            } else {
                for ch in out.iter_mut().take(base + channels).skip(base) {
                    *ch = 0.0;
                }
            }
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
