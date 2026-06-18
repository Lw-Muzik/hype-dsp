//! macOS per-app mixer via Core Audio process taps (macOS 14.4+).
//!
//! Per-app **volume/mute** without a system extension:
//!
//! - **Enumerate** processes producing output and resolve each to its real app
//!   (responsible PID → `NSRunningApplication`), so e.g. a browser's audio
//!   helper shows as the browser.
//! - **Attenuate** an app by creating a *muted* mixdown process tap over all of
//!   its processes (so its direct output is silenced) wrapped in a private
//!   aggregate device whose output sub-device is the real default output; an IO
//!   callback re-renders the tapped audio at the chosen gain. Mute = gain 0.
//!   Restoring 100%/unmuted tears the engine down, so the app plays normally.
//!
//! Intricate Core Audio FFI (`objc2-core-audio` 0.3), modeled on
//! [`crate`]'s sibling system-tap. Compile-verified; runtime behavior must be
//! validated on a signed build with the audio-capture permission.

#![cfg(target_os = "macos")]

use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::AllocAnyThread;
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSBitmapImageRepPropertyKey, NSRunningApplication,
};
use objc2_core_audio::{
    kAudioAggregateDeviceIsPrivateKey, kAudioAggregateDeviceMainSubDeviceKey,
    kAudioAggregateDeviceNameKey, kAudioAggregateDeviceSubDeviceListKey,
    kAudioAggregateDeviceTapListKey, kAudioAggregateDeviceUIDKey,
    kAudioDevicePropertyDeviceUID, kAudioHardwarePropertyDefaultOutputDevice,
    kAudioHardwarePropertyProcessObjectList, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject, kAudioProcessPropertyBundleID,
    kAudioProcessPropertyIsRunningOutput, kAudioProcessPropertyPID, kAudioSubDeviceUIDKey,
    kAudioSubTapUIDKey, kAudioTapPropertyUID, AudioDeviceCreateIOProcID, AudioDeviceIOProcID,
    AudioDeviceStart, AudioDeviceStop, AudioHardwareCreateAggregateDevice,
    AudioHardwareCreateProcessTap, AudioHardwareDestroyAggregateDevice,
    AudioHardwareDestroyProcessTap, AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize,
    AudioObjectID, AudioObjectPropertyAddress, CATapDescription, CATapMuteBehavior,
};
use objc2_core_audio_types::{AudioBufferList, AudioTimeStamp};
use objc2_core_foundation::{CFDictionary, CFRetained, CFString};
use objc2_foundation::{
    NSDictionary, NSMutableArray, NSMutableDictionary, NSNumber, NSObject, NSString,
};

use hm_core::AppSession;

use crate::error::PlatformError;
use crate::SessionController;

// ---------------------------------------------------------------- FFI helpers

fn addr(selector: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    }
}

fn get_scalar<T: Copy + Default>(obj: AudioObjectID, selector: u32) -> Option<T> {
    let address = addr(selector);
    let mut value = T::default();
    let mut size = size_of::<T>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            obj,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new(&mut value as *mut T as *mut c_void)?,
        )
    };
    (status == 0).then_some(value)
}

fn get_cfstring(obj: AudioObjectID, selector: u32) -> Option<String> {
    let address = addr(selector);
    let mut ptr: *const CFString = std::ptr::null();
    let mut size = size_of::<*const CFString>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            obj,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new(&mut ptr as *mut *const CFString as *mut c_void)?,
        )
    };
    if status != 0 || ptr.is_null() {
        return None;
    }
    let s = unsafe { CFRetained::from_raw(NonNull::new(ptr as *mut CFString)?) };
    Some(s.to_string())
}

fn process_object_list() -> Vec<AudioObjectID> {
    let address = addr(kAudioHardwarePropertyProcessObjectList);
    let system = kAudioObjectSystemObject as AudioObjectID;
    let mut size: u32 = 0;
    let status = unsafe {
        AudioObjectGetPropertyDataSize(
            system,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
        )
    };
    if status != 0 || size == 0 {
        return Vec::new();
    }
    let count = size as usize / size_of::<AudioObjectID>();
    let mut ids = vec![0 as AudioObjectID; count];
    let mut got = size;
    let status = unsafe {
        AudioObjectGetPropertyData(
            system,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut got),
            NonNull::new(ids.as_mut_ptr() as *mut c_void).unwrap(),
        )
    };
    if status != 0 {
        return Vec::new();
    }
    ids.truncate(got as usize / size_of::<AudioObjectID>());
    ids
}

fn is_running_output(obj: AudioObjectID) -> bool {
    get_scalar::<u32>(obj, kAudioProcessPropertyIsRunningOutput) == Some(1)
}

fn default_output_uid() -> Option<String> {
    let dev: AudioObjectID =
        get_scalar(kAudioObjectSystemObject as AudioObjectID, kAudioHardwarePropertyDefaultOutputDevice)?;
    if dev == 0 {
        return None;
    }
    get_cfstring(dev, kAudioDevicePropertyDeviceUID)
}

/// The PID macOS holds *responsible* for `pid` — for a browser/XPC audio helper
/// this is the parent app, giving a clean name. Private but long-stable libsystem
/// symbol (used by SoundSource, Background Music, etc.); falls back to `pid`.
fn responsible_pid(pid: i32) -> i32 {
    extern "C" {
        fn responsibility_get_pid_responsible_for_pid(pid: i32) -> i32;
    }
    let r = unsafe { responsibility_get_pid_responsible_for_pid(pid) };
    if r > 0 {
        r
    } else {
        pid
    }
}

fn friendly_name(bundle_id: &str) -> String {
    bundle_id
        .rsplit('.')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(bundle_id)
        .to_string()
}

/// Resolve a PID to `(stable id, display name)` via `NSRunningApplication`.
fn running_app_identity(pid: i32) -> Option<(String, String)> {
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?;
    let bundle = app.bundleIdentifier().map(|s| s.to_string());
    let name = app.localizedName().map(|s| s.to_string());
    let id = bundle
        .clone()
        .unwrap_or_else(|| format!("pid:{pid}"));
    let name = name
        .or(bundle)
        .unwrap_or_else(|| format!("PID {pid}"));
    Some((id, name))
}

/// The app icon for `pid` as a PNG `data:` URI, via `NSRunningApplication`.
fn icon_data_uri(pid: i32) -> Option<String> {
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?;
    let image = app.icon()?;
    let tiff = image.TIFFRepresentation()?;
    let rep = NSBitmapImageRep::imageRepWithData(&tiff)?;
    let props = NSDictionary::<NSBitmapImageRepPropertyKey, AnyObject>::new();
    let png = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &props) }?;
    Some(crate::util::png_data_uri(&png.to_vec()))
}

/// `(stable id, display name)` for an audio process object: responsible app
/// first, then the process's own app, then its bundle id, then the PID.
fn app_identity(obj: AudioObjectID) -> (String, String) {
    let pid = get_scalar::<i32>(obj, kAudioProcessPropertyPID).unwrap_or(0);
    if let Some(id) =
        running_app_identity(responsible_pid(pid)).or_else(|| running_app_identity(pid))
    {
        return id;
    }
    match get_cfstring(obj, kAudioProcessPropertyBundleID).filter(|b| !b.is_empty()) {
        Some(b) => {
            let name = friendly_name(&b);
            (b, name)
        }
        None => (format!("pid:{pid}"), format!("PID {pid}")),
    }
}

/// All currently-outputting process objects whose resolved app id is `id`.
fn process_objects_for_id(id: &str) -> Vec<AudioObjectID> {
    process_object_list()
        .into_iter()
        .filter(|&o| is_running_output(o) && app_identity(o).0 == id)
        .collect()
}

fn tap_uid_string(tap_id: AudioObjectID) -> Option<String> {
    get_cfstring(tap_id, kAudioTapPropertyUID)
}

// ---------------------------------------------------------------- per-app engine

/// IO-callback context: the live gain (f32 bits) the app is re-rendered at.
struct TapCtx {
    gain: AtomicU32,
}

/// IO proc: re-render the tapped (muted) app into the device output at `gain`.
unsafe extern "C-unwind" fn io_proc(
    _device: AudioObjectID,
    _now: NonNull<AudioTimeStamp>,
    input_data: NonNull<AudioBufferList>,
    _input_time: NonNull<AudioTimeStamp>,
    output_data: NonNull<AudioBufferList>,
    _output_time: NonNull<AudioTimeStamp>,
    client_data: *mut c_void,
) -> i32 {
    if client_data.is_null() {
        return 0;
    }
    let ctx = &*(client_data as *mut TapCtx);
    let gain = f32::from_bits(ctx.gain.load(Ordering::Relaxed));

    let inputs = input_data.as_ref();
    let outputs = output_data.as_ref();
    if outputs.mNumberBuffers == 0 {
        return 0;
    }
    let out_list = output_data.as_ptr();
    let out_buffers = std::slice::from_raw_parts_mut(
        (*out_list).mBuffers.as_mut_ptr(),
        (*out_list).mNumberBuffers as usize,
    );
    let out = &mut out_buffers[0];
    let out_ch = out.mNumberChannels.max(1) as usize;
    let out_count = out.mDataByteSize as usize / size_of::<f32>();
    if out.mData.is_null() || out_count == 0 {
        return 0;
    }
    let out_samples = std::slice::from_raw_parts_mut(out.mData as *mut f32, out_count);
    // Start from silence so any unfilled frames/channels are quiet.
    out_samples.iter_mut().for_each(|s| *s = 0.0);

    if inputs.mNumberBuffers == 0 {
        return 0;
    }
    let in_buffers =
        std::slice::from_raw_parts(inputs.mBuffers.as_ptr(), inputs.mNumberBuffers as usize);
    let inp = &in_buffers[0];
    if inp.mData.is_null() {
        return 0;
    }
    let in_ch = inp.mNumberChannels.max(1) as usize;
    let in_count = inp.mDataByteSize as usize / size_of::<f32>();
    let in_samples = std::slice::from_raw_parts(inp.mData as *const f32, in_count);

    let out_frames = out_count / out_ch;
    let in_frames = in_count / in_ch;
    let frames = out_frames.min(in_frames);
    for f in 0..frames {
        let l = in_samples[f * in_ch];
        let r = in_samples[f * in_ch + (in_ch.min(2) - 1)];
        out_samples[f * out_ch] = l * gain;
        if out_ch >= 2 {
            out_samples[f * out_ch + 1] = r * gain;
        }
    }
    0
}

/// One running attenuation engine for an app (tap + aggregate + IO proc).
struct AppTap {
    tap_id: AudioObjectID,
    aggregate_id: AudioObjectID,
    proc_id: AudioDeviceIOProcID,
    ctx: *mut TapCtx,
    started: bool,
}

// Core Audio object IDs + the boxed ctx are only touched on create/drop/set.
unsafe impl Send for AppTap {}

impl AppTap {
    fn new(processes: &[AudioObjectID], gain: f32) -> Result<Self, PlatformError> {
        let output_uid = default_output_uid()
            .ok_or_else(|| PlatformError::Unsupported("no default output device".into()))?;

        // Muted mixdown tap over the app's process(es).
        let procs = NSMutableArray::<NSNumber>::new();
        for &p in processes {
            procs.addObject(&NSNumber::new_u32(p));
        }
        let description = unsafe {
            let d = CATapDescription::initStereoMixdownOfProcesses(
                CATapDescription::alloc(),
                &procs,
            );
            d.setMuteBehavior(CATapMuteBehavior::Muted);
            d.setName(&NSString::from_str("HypeMuzik App Tap"));
            d
        };
        let mut tap_id: AudioObjectID = 0;
        let status =
            unsafe { AudioHardwareCreateProcessTap(Some(&description), &mut tap_id as *mut _) };
        if status != 0 || tap_id == 0 {
            return Err(PlatformError::Unsupported(format!(
                "could not create process tap (status {status}); grant audio capture and run a \
                 signed build"
            )));
        }
        let tap_uid = match tap_uid_string(tap_id) {
            Some(u) => u,
            None => {
                unsafe { AudioHardwareDestroyProcessTap(tap_id) };
                return Err(PlatformError::Unsupported("could not read tap UID".into()));
            }
        };

        let aggregate_id = match create_aggregate(&tap_uid, &output_uid, tap_id) {
            Ok(id) => id,
            Err(e) => {
                unsafe { AudioHardwareDestroyProcessTap(tap_id) };
                return Err(e);
            }
        };

        let ctx = Box::into_raw(Box::new(TapCtx {
            gain: AtomicU32::new(gain.to_bits()),
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
            return Err(PlatformError::Unsupported(format!(
                "IO proc creation failed ({status})"
            )));
        }

        let mut tap = Self {
            tap_id,
            aggregate_id,
            proc_id,
            ctx,
            started: false,
        };
        let status = unsafe { AudioDeviceStart(aggregate_id, proc_id) };
        if status != 0 {
            return Err(PlatformError::Unsupported(format!(
                "could not start app-tap device ({status})"
            )));
        }
        tap.started = true;
        Ok(tap)
    }

    fn set_gain(&self, gain: f32) {
        unsafe { (*self.ctx).gain.store(gain.to_bits(), Ordering::Relaxed) };
    }
}

impl Drop for AppTap {
    fn drop(&mut self) {
        unsafe {
            if self.started {
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

/// Build a private aggregate device: the muted tap as input, the real default
/// output as the (clock + main) output sub-device. Its UID is unique per tap.
fn create_aggregate(
    tap_uid: &str,
    output_uid: &str,
    tap_id: AudioObjectID,
) -> Result<AudioObjectID, PlatformError> {
    fn key(c: &std::ffi::CStr) -> Retained<NSString> {
        NSString::from_str(c.to_str().unwrap_or_default())
    }

    let k_uid = key(kAudioAggregateDeviceUIDKey);
    let k_name = key(kAudioAggregateDeviceNameKey);
    let k_private = key(kAudioAggregateDeviceIsPrivateKey);
    let k_taps = key(kAudioAggregateDeviceTapListKey);
    let k_subtap = key(kAudioSubTapUIDKey);
    let k_subdevs = key(kAudioAggregateDeviceSubDeviceListKey);
    let k_subdev_uid = key(kAudioSubDeviceUIDKey);
    let k_main = key(kAudioAggregateDeviceMainSubDeviceKey);

    let dict = NSMutableDictionary::<NSString, NSObject>::new();
    unsafe {
        // taps = [ { subTapUID: <tap uid> } ]
        let tap_entry = NSMutableDictionary::<NSString, NSObject>::new();
        tap_entry.setObject_forKey(
            &NSString::from_str(tap_uid),
            ProtocolObject::from_ref(&*k_subtap),
        );
        let taps = NSMutableArray::<NSObject>::new();
        taps.addObject(&tap_entry);

        // subdevices = [ { subDeviceUID: <output uid> } ]
        let sub_entry = NSMutableDictionary::<NSString, NSObject>::new();
        sub_entry.setObject_forKey(
            &NSString::from_str(output_uid),
            ProtocolObject::from_ref(&*k_subdev_uid),
        );
        let subs = NSMutableArray::<NSObject>::new();
        subs.addObject(&sub_entry);

        dict.setObject_forKey(
            &NSString::from_str(&format!("HypeMuzikAppTap-{tap_id}")),
            ProtocolObject::from_ref(&*k_uid),
        );
        dict.setObject_forKey(
            &NSString::from_str("HypeMuzik App Tap"),
            ProtocolObject::from_ref(&*k_name),
        );
        dict.setObject_forKey(&NSNumber::new_bool(true), ProtocolObject::from_ref(&*k_private));
        dict.setObject_forKey(
            &NSString::from_str(output_uid),
            ProtocolObject::from_ref(&*k_main),
        );
        dict.setObject_forKey(&subs, ProtocolObject::from_ref(&*k_subdevs));
        dict.setObject_forKey(&taps, ProtocolObject::from_ref(&*k_taps));
    }

    let cf: &CFDictionary =
        unsafe { &*(Retained::as_ptr(&dict) as *const CFDictionary) };
    let mut aggregate_id: AudioObjectID = 0;
    let status =
        unsafe { AudioHardwareCreateAggregateDevice(cf, NonNull::from(&mut aggregate_id)) };
    if status != 0 || aggregate_id == 0 {
        return Err(PlatformError::Unsupported(format!(
            "could not create app-tap aggregate device (status {status})"
        )));
    }
    Ok(aggregate_id)
}

// ---------------------------------------------------------------- controller

#[derive(Clone, Copy)]
struct Desired {
    volume: f32,
    muted: bool,
}

impl Default for Desired {
    fn default() -> Self {
        Self {
            volume: 1.0,
            muted: false,
        }
    }
}

impl Desired {
    /// Effective gain to re-render at (mute wins).
    fn gain(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            self.volume
        }
    }
    /// Whether this leaves the app untouched (no engine needed).
    fn is_passthrough(&self) -> bool {
        !self.muted && (self.volume - 1.0).abs() < f32::EPSILON
    }
}

/// macOS per-app mixer: enumeration + per-app tap-and-re-render attenuation.
#[derive(Default)]
pub struct MacosSessionController {
    desired: Mutex<HashMap<String, Desired>>,
    engines: Mutex<HashMap<String, AppTap>>,
    /// Per-app icon (PNG data URI), resolved once and cached.
    icon_cache: Mutex<HashMap<String, Option<String>>>,
}

impl MacosSessionController {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cached PNG-data-URI icon for an app id (resolved via its responsible app).
    fn icon_for(&self, id: &str, obj: AudioObjectID) -> Option<String> {
        let mut cache = self.icon_cache.lock().expect("mixer icons poisoned");
        if let Some(cached) = cache.get(id) {
            return cached.clone();
        }
        let pid = get_scalar::<i32>(obj, kAudioProcessPropertyPID).unwrap_or(0);
        let uri = icon_data_uri(responsible_pid(pid)).or_else(|| icon_data_uri(pid));
        cache.insert(id.to_string(), uri.clone());
        uri
    }

    /// Bring the running engine for `id` in line with its desired state.
    fn apply(&self, id: &str) {
        let want = self
            .desired
            .lock()
            .expect("mixer state poisoned")
            .get(id)
            .copied()
            .unwrap_or_default();

        let mut engines = self.engines.lock().expect("mixer engines poisoned");
        if want.is_passthrough() {
            engines.remove(id); // drop → tap destroyed → app plays normally
            return;
        }
        if let Some(engine) = engines.get(id) {
            engine.set_gain(want.gain());
            return;
        }
        let procs = process_objects_for_id(id);
        if procs.is_empty() {
            return; // app isn't producing audio right now; (re)applied when it is
        }
        if let Ok(engine) = AppTap::new(&procs, want.gain()) {
            engines.insert(id.to_string(), engine);
        }
    }
}

impl SessionController for MacosSessionController {
    fn supported(&self) -> bool {
        true
    }

    fn unavailable_reason(&self) -> Option<String> {
        None
    }

    fn list_sessions(&self) -> Vec<AppSession> {
        let desired = self.desired.lock().expect("mixer state poisoned");
        // Dedupe processes that resolve to the same app (e.g. helpers).
        let mut seen: HashMap<String, AppSession> = HashMap::new();
        for obj in process_object_list() {
            if !is_running_output(obj) {
                continue;
            }
            let (id, name) = app_identity(obj);
            if seen.contains_key(&id) {
                continue;
            }
            let d = desired.get(&id).copied().unwrap_or_default();
            let icon = self.icon_for(&id, obj);
            seen.insert(
                id.clone(),
                AppSession {
                    id,
                    name,
                    icon,
                    volume: d.volume,
                    muted: d.muted,
                },
            );
        }
        let mut sessions: Vec<AppSession> = seen.into_values().collect();
        sessions.sort_by_key(|s| s.name.to_lowercase());
        sessions
    }

    fn set_volume(&self, id: &str, gain: f32) -> Result<(), PlatformError> {
        self.desired
            .lock()
            .expect("mixer state poisoned")
            .entry(id.to_string())
            .or_default()
            .volume = gain.clamp(0.0, 1.0);
        self.apply(id);
        Ok(())
    }

    fn set_muted(&self, id: &str, muted: bool) -> Result<(), PlatformError> {
        self.desired
            .lock()
            .expect("mixer state poisoned")
            .entry(id.to_string())
            .or_default()
            .muted = muted;
        self.apply(id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_name_strips_bundle_prefix() {
        assert_eq!(friendly_name("com.apple.Music"), "Music");
        assert_eq!(friendly_name("com.google.Chrome"), "Chrome");
        assert_eq!(friendly_name("Spotify"), "Spotify");
    }

    #[test]
    fn desired_semantics() {
        assert!(Desired::default().is_passthrough());
        assert_eq!(Desired::default().gain(), 1.0);
        let muted = Desired {
            volume: 0.8,
            muted: true,
        };
        assert_eq!(muted.gain(), 0.0);
        assert!(!muted.is_passthrough());
    }

    #[test]
    fn enumeration_and_intent_are_safe() {
        // Exercises the real Core Audio enumeration + NSRunningApplication FFI.
        let ctrl = MacosSessionController::new();
        let _ = ctrl.list_sessions();
        // Setting 100%/unmuted must not create an engine (passthrough).
        ctrl.set_volume("com.example.none", 1.0).unwrap();
        assert!(ctrl.engines.lock().unwrap().is_empty());
        // A non-running app id can't build an engine but must not error.
        ctrl.set_muted("com.example.none", true).unwrap();
    }
}
