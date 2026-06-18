//! macOS per-app mixer via Core Audio.
//!
//! **Phase 1 (this module): enumeration.** Lists the processes currently
//! producing audio output (`kAudioHardwarePropertyProcessObjectList` filtered by
//! `kAudioProcessPropertyIsRunningOutput`) and remembers a desired
//! volume/mute per app. The actual attenuation engine (a muted process tap that
//! re-renders the app at a scaled gain — Phases 2–3) plugs into
//! [`MacosSessionController::apply`] later; until then `set_volume`/`set_muted`
//! only record intent so the UI reflects it.
//!
//! Intricate Core Audio FFI (`objc2-core-audio` 0.3): compile-verified; runtime
//! behavior is validated on a signed build with the audio-capture permission.

#![cfg(target_os = "macos")]

use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::NonNull;
use std::sync::Mutex;

use objc2_core_audio::{
    kAudioHardwarePropertyProcessObjectList, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject, kAudioProcessPropertyBundleID,
    kAudioProcessPropertyIsRunningOutput, kAudioProcessPropertyPID, AudioObjectGetPropertyData,
    AudioObjectGetPropertyDataSize, AudioObjectID, AudioObjectPropertyAddress,
};
use objc2_core_foundation::{CFRetained, CFString};

use hm_core::AppSession;

use crate::error::PlatformError;
use crate::SessionController;

/// A global-scope property address on the main element.
fn addr(selector: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    }
}

/// Read a fixed-size POD property (e.g. `u32`, `i32`) from an audio object.
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

/// Read a `CFString` property and return it as a Rust `String`.
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
    // The Get call returns a +1 (retained) CFString we now own.
    let s = unsafe { CFRetained::from_raw(NonNull::new(ptr as *mut CFString)?) };
    Some(s.to_string())
}

/// Every process object the system knows about.
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

/// Derive a human-friendly name from a bundle id, e.g.
/// `com.apple.Music` → `Music`, `com.google.Chrome` → `Chrome`.
fn friendly_name(bundle_id: &str) -> String {
    bundle_id
        .rsplit('.')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(bundle_id)
        .to_string()
}

/// A process currently emitting audio.
struct AudioApp {
    /// Stable id: the bundle id when known, else `pid:<pid>`.
    id: String,
    name: String,
}

/// Enumerate processes that are currently producing audio output.
fn running_audio_apps() -> Vec<AudioApp> {
    let mut apps = Vec::new();
    for obj in process_object_list() {
        // Only processes actively outputting audio are "sessions".
        if get_scalar::<u32>(obj, kAudioProcessPropertyIsRunningOutput) != Some(1) {
            continue;
        }
        let bundle = get_cfstring(obj, kAudioProcessPropertyBundleID);
        let pid = get_scalar::<i32>(obj, kAudioProcessPropertyPID).unwrap_or(0);
        let (id, name) = match bundle {
            Some(b) if !b.is_empty() => (b.clone(), friendly_name(&b)),
            _ => (format!("pid:{pid}"), format!("PID {pid}")),
        };
        apps.push(AudioApp { id, name });
    }
    apps
}

/// Desired per-app state the user has set (remembered across enumeration).
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

/// macOS per-app mixer controller (Phase 1: enumeration + remembered intent).
#[derive(Default)]
pub struct MacosSessionController {
    desired: Mutex<HashMap<String, Desired>>,
}

impl MacosSessionController {
    pub fn new() -> Self {
        Self::default()
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
        running_audio_apps()
            .into_iter()
            .map(|app| {
                let d = desired.get(&app.id).copied().unwrap_or_default();
                AppSession {
                    id: app.id,
                    name: app.name,
                    icon: None,
                    volume: d.volume,
                    muted: d.muted,
                }
            })
            .collect()
    }

    fn set_volume(&self, id: &str, gain: f32) -> Result<(), PlatformError> {
        let mut desired = self.desired.lock().expect("mixer state poisoned");
        desired.entry(id.to_string()).or_default().volume = gain.clamp(0.0, 1.0);
        // Phase 2–3 will apply this through a per-app tap-and-re-render engine.
        Ok(())
    }

    fn set_muted(&self, id: &str, muted: bool) -> Result<(), PlatformError> {
        let mut desired = self.desired.lock().expect("mixer state poisoned");
        desired.entry(id.to_string()).or_default().muted = muted;
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
    fn enumeration_does_not_panic_and_remembers_intent() {
        // Enumeration must be safe to call even with no audio playing.
        let ctrl = MacosSessionController::new();
        let _ = ctrl.list_sessions();
        ctrl.set_volume("com.example.app", 0.5).unwrap();
        ctrl.set_muted("com.example.app", true).unwrap();
        let d = ctrl.desired.lock().unwrap();
        let s = d.get("com.example.app").copied().unwrap();
        assert_eq!(s.volume, 0.5);
        assert!(s.muted);
    }
}
