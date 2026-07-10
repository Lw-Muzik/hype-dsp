//! Output-device management: list the system's output devices and switch the
//! **system default** output device (Internal Speakers, headphones, USB / BT /
//! HDMI / AirPlay, …), the way macOS Sound settings or Boom3D's "Select your
//! output device" do.
//!
//! Setting the system default output is the correct lever for this app: the
//! engine is hard-wired to the default output (`engine::output_setup` uses
//! cpal's `default_output_device`, and a `DefaultOutputListener` rebuilds the
//! system-EQ tap on default-device changes), so switching the default makes the
//! whole app follow — **no special entitlement is required** (unlike the tap /
//! capture features, this is a plain, sanctioned Core Audio setter).
//!
//! On macOS this talks to Core Audio (`AudioObjectGetPropertyData` /
//! `AudioObjectSetPropertyData` on `kAudioObjectSystemObject`), mirroring the
//! FFI style and panic-free rigor of [`crate::system_tap`]. On every other OS it
//! degrades to a names-only listing via cpal (`crate::device`) and reports that
//! switching the output isn't supported here.
//!
//! The Core Audio calls are compile-verified against `objc2-core-audio` 0.3;
//! their runtime behaviour (correct enumeration, that switching the default
//! actually moves audio, and the AirPods-Continuity revert handling) must be
//! validated on a real device.

use serde::{Deserialize, Serialize};

use crate::error::AudioError;

/// How a device is physically attached, used by the UI to pick an icon and to
/// coarsely hint speaker-vs-headphone. `Other` covers anything unrecognised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputTransport {
    BuiltIn,
    Usb,
    Bluetooth,
    Hdmi,
    DisplayPort,
    AirPlay,
    Aggregate,
    Virtual,
    Thunderbolt,
    Other,
}

// Core Audio `kAudioDeviceTransportType*` FourCC codes, defined locally so the
// mapping stays a pure function testable on any platform. A macOS-only test
// asserts these still equal the `objc2-core-audio` constants.
const TRANSPORT_BUILT_IN: u32 = 0x626c_746e; // 'bltn'
const TRANSPORT_USB: u32 = 0x7573_6220; // 'usb '
const TRANSPORT_BLUETOOTH: u32 = 0x626c_7565; // 'blue'
const TRANSPORT_BLUETOOTH_LE: u32 = 0x626c_6561; // 'blea'
const TRANSPORT_HDMI: u32 = 0x6864_6d69; // 'hdmi'
const TRANSPORT_DISPLAY_PORT: u32 = 0x6470_7274; // 'dprt'
const TRANSPORT_AIRPLAY: u32 = 0x6169_7270; // 'airp'
const TRANSPORT_AGGREGATE: u32 = 0x6772_7570; // 'grup'
const TRANSPORT_AUTO_AGGREGATE: u32 = 0x6667_7270; // 'fgrp'
const TRANSPORT_VIRTUAL: u32 = 0x7669_7274; // 'virt'
const TRANSPORT_THUNDERBOLT: u32 = 0x7468_756e; // 'thun'

impl OutputTransport {
    /// Map a Core Audio transport-type FourCC to a UI transport category. Pure —
    /// unit-tested without Core Audio.
    pub fn from_transport_code(code: u32) -> Self {
        match code {
            TRANSPORT_BUILT_IN => Self::BuiltIn,
            TRANSPORT_USB => Self::Usb,
            TRANSPORT_BLUETOOTH | TRANSPORT_BLUETOOTH_LE => Self::Bluetooth,
            TRANSPORT_HDMI => Self::Hdmi,
            TRANSPORT_DISPLAY_PORT => Self::DisplayPort,
            TRANSPORT_AIRPLAY => Self::AirPlay,
            TRANSPORT_AGGREGATE | TRANSPORT_AUTO_AGGREGATE => Self::Aggregate,
            TRANSPORT_VIRTUAL => Self::Virtual,
            TRANSPORT_THUNDERBOLT => Self::Thunderbolt,
            _ => Self::Other,
        }
    }
}

/// One selectable output device, as presented to the UI picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputDevice {
    /// Core Audio `AudioObjectID` (macOS). `0` on platforms without one.
    pub id: u32,
    /// Stable unique identifier used as the selection key. On macOS this is the
    /// device UID (`kAudioDevicePropertyDeviceUID`); elsewhere it's the name.
    pub uid: String,
    /// Human-readable device name.
    pub name: String,
    /// Physical transport, for the UI icon + speaker/headphone hint.
    pub transport: OutputTransport,
    /// Whether this is the current system default output.
    pub is_default: bool,
    /// Whether the device currently reports itself alive.
    pub is_alive: bool,
}

/// List the system's output devices (default first, then by name).
///
/// macOS enumerates Core Audio devices with at least one output channel;
/// every other platform returns a names-only cpal listing.
pub fn list_output_devices() -> Result<Vec<OutputDevice>, AudioError> {
    #[cfg(target_os = "macos")]
    {
        Ok(macos::list_output_devices())
    }
    #[cfg(not(target_os = "macos"))]
    {
        fallback::list_output_devices()
    }
}

/// Make `uid` the system default output device. Returns an error if the uid
/// doesn't resolve, the set fails, or (macOS) the change doesn't stick.
///
/// Unsupported on non-macOS platforms (there's no sanctioned cross-platform
/// default-output setter in cpal) — those return [`AudioError::Unavailable`].
pub fn set_default_output(uid: &str) -> Result<(), AudioError> {
    #[cfg(target_os = "macos")]
    {
        macos::set_default_output(uid)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = uid;
        Err(AudioError::Unavailable(
            "selecting the output device is only supported on macOS".into(),
        ))
    }
}

/// The UID of the current system default output device, if resolvable.
pub fn default_output_uid() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        macos::default_output_uid()
    }
    #[cfg(not(target_os = "macos"))]
    {
        fallback::default_output_uid()
    }
}

/// The `AudioObjectID` of the current system default output device (macOS).
/// `None` off macOS.
pub fn default_output_device_id() -> Option<u32> {
    #[cfg(target_os = "macos")]
    {
        macos::default_output_device_id()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Sum output channels across an `AudioBufferList`'s buffers. Pure so it's
/// unit-tested without Core Audio; only referenced by the macOS backend.
#[cfg(target_os = "macos")]
pub(crate) fn total_channels(channels_per_buffer: &[u32]) -> u32 {
    channels_per_buffer.iter().copied().sum()
}

/// Names-only cpal fallback for non-macOS platforms.
#[cfg(not(target_os = "macos"))]
mod fallback {
    use super::{AudioError, OutputDevice, OutputTransport};

    pub fn list_output_devices() -> Result<Vec<OutputDevice>, AudioError> {
        let devices = crate::device::list_output_devices()?;
        Ok(devices
            .into_iter()
            .map(|d| OutputDevice {
                id: 0,
                uid: d.name.clone(),
                name: d.name,
                transport: OutputTransport::Other,
                is_default: d.is_default,
                is_alive: true,
            })
            .collect())
    }

    pub fn default_output_uid() -> Option<String> {
        crate::device::list_output_devices()
            .ok()?
            .into_iter()
            .find(|d| d.is_default)
            .map(|d| d.name)
    }
}

/// Core Audio backend (macOS).
#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;
    use std::mem::{offset_of, size_of};
    use std::ptr::NonNull;
    use std::time::Duration;

    use objc2_core_audio::{
        kAudioDevicePropertyDeviceIsAlive, kAudioDevicePropertyDeviceUID,
        kAudioDevicePropertyStreamConfiguration, kAudioDevicePropertyTransportType,
        kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyDevices,
        kAudioObjectPropertyElementMain, kAudioObjectPropertyName,
        kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject,
        AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectID,
        AudioObjectPropertyAddress, AudioObjectSetPropertyData,
    };
    use objc2_core_audio_types::{AudioBuffer, AudioBufferList};
    use objc2_core_foundation::{CFRetained, CFString};

    use super::{total_channels, AudioError, OutputDevice, OutputTransport};

    /// A property address at a given selector + scope, element = main.
    fn addr(selector: u32, scope: u32) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress {
            mSelector: selector,
            mScope: scope,
            mElement: kAudioObjectPropertyElementMain,
        }
    }

    /// Enumerate every `AudioObjectID` on the system object, or an empty list on
    /// any failure (never panics).
    fn all_device_ids() -> Vec<AudioObjectID> {
        let address = addr(
            kAudioHardwarePropertyDevices,
            kAudioObjectPropertyScopeGlobal,
        );
        let mut size: u32 = 0;
        let status = unsafe {
            AudioObjectGetPropertyDataSize(
                kAudioObjectSystemObject as AudioObjectID,
                NonNull::from(&address),
                0,
                std::ptr::null(),
                NonNull::from(&mut size),
            )
        };
        let count = size as usize / size_of::<AudioObjectID>();
        if status != 0 || count == 0 {
            return Vec::new();
        }
        let mut ids = vec![0u32; count];
        let mut io_size = size;
        let status = unsafe {
            AudioObjectGetPropertyData(
                kAudioObjectSystemObject as AudioObjectID,
                NonNull::from(&address),
                0,
                std::ptr::null(),
                NonNull::from(&mut io_size),
                // `ids` is non-empty here, so the pointer is non-null.
                NonNull::new(ids.as_mut_ptr() as *mut c_void).unwrap(),
            )
        };
        if status != 0 {
            return Vec::new();
        }
        // The HAL may return fewer entries than the earlier size implied.
        let got = io_size as usize / size_of::<AudioObjectID>();
        ids.truncate(got.min(count));
        ids
    }

    /// The number of **output** channels a device exposes (0 = input-only).
    /// Reads the output-scope `StreamConfiguration` (an `AudioBufferList`) and
    /// sums each buffer's channel count.
    fn output_channel_count(device: AudioObjectID) -> u32 {
        let address = addr(
            kAudioDevicePropertyStreamConfiguration,
            kAudioObjectPropertyScopeOutput,
        );
        let mut size: u32 = 0;
        let status = unsafe {
            AudioObjectGetPropertyDataSize(
                device,
                NonNull::from(&address),
                0,
                std::ptr::null(),
                NonNull::from(&mut size),
            )
        };
        if status != 0 || (size as usize) < size_of::<u32>() {
            return 0;
        }
        // Back the buffer with `u64` words so it is pointer-aligned for the
        // `AudioBuffer.mData` pointers inside the `AudioBufferList`.
        let words = (size as usize).div_ceil(size_of::<u64>()).max(1);
        let mut backing = vec![0u64; words];
        let mut io_size = size;
        let status = unsafe {
            AudioObjectGetPropertyData(
                device,
                NonNull::from(&address),
                0,
                std::ptr::null(),
                NonNull::from(&mut io_size),
                // `backing` is non-empty, so the pointer is non-null.
                NonNull::new(backing.as_mut_ptr() as *mut c_void).unwrap(),
            )
        };
        if status != 0 {
            return 0;
        }
        let list = unsafe { &*(backing.as_ptr() as *const AudioBufferList) };
        // Cap the buffer count by what the returned bytes can actually hold, so a
        // bogus `mNumberBuffers` can never walk past the allocation.
        let header = offset_of!(AudioBufferList, mBuffers);
        let max_buffers = (io_size as usize).saturating_sub(header) / size_of::<AudioBuffer>();
        let n = (list.mNumberBuffers as usize).min(max_buffers);
        if n == 0 {
            return 0;
        }
        let buffers = unsafe { std::slice::from_raw_parts(list.mBuffers.as_ptr(), n) };
        let counts: Vec<u32> = buffers.iter().map(|b| b.mNumberChannels).collect();
        total_channels(&counts)
    }

    /// Read a CFString device property (global scope), or `None` on failure.
    fn read_cfstring(device: AudioObjectID, selector: u32) -> Option<String> {
        let address = addr(selector, kAudioObjectPropertyScopeGlobal);
        let mut cf: *const CFString = std::ptr::null();
        let mut size = size_of::<*const CFString>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                device,
                NonNull::from(&address),
                0,
                std::ptr::null(),
                NonNull::from(&mut size),
                NonNull::new(&mut cf as *mut *const CFString as *mut c_void).unwrap(),
            )
        };
        if status != 0 || cf.is_null() {
            return None;
        }
        // The getter returns a +1 retained CFStringRef the caller owns.
        let s = unsafe { CFRetained::from_raw(NonNull::new(cf as *mut CFString)?) };
        Some(s.to_string())
    }

    /// Read a `u32` device property at a given scope, or `None` on failure.
    fn read_u32(device: AudioObjectID, selector: u32, scope: u32) -> Option<u32> {
        let address = addr(selector, scope);
        let mut value: u32 = 0;
        let mut size = size_of::<u32>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                device,
                NonNull::from(&address),
                0,
                std::ptr::null(),
                NonNull::from(&mut size),
                NonNull::new(&mut value as *mut u32 as *mut c_void).unwrap(),
            )
        };
        (status == 0).then_some(value)
    }

    /// Whether the device reports itself alive. Fail-open: a failed query is
    /// treated as alive so a transient probe glitch never drops a real device.
    fn device_is_alive(device: AudioObjectID) -> bool {
        match read_u32(
            device,
            kAudioDevicePropertyDeviceIsAlive,
            kAudioObjectPropertyScopeGlobal,
        ) {
            Some(v) => v != 0,
            None => true,
        }
    }

    pub fn default_output_device_id() -> Option<AudioObjectID> {
        let address = addr(
            kAudioHardwarePropertyDefaultOutputDevice,
            kAudioObjectPropertyScopeGlobal,
        );
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

    pub fn default_output_uid() -> Option<String> {
        let id = default_output_device_id()?;
        read_cfstring(id, kAudioDevicePropertyDeviceUID)
    }

    /// Resolve a device UID to its `AudioObjectID` by scanning the device list.
    fn device_id_for_uid(uid: &str) -> Option<AudioObjectID> {
        all_device_ids()
            .into_iter()
            .find(|&id| read_cfstring(id, kAudioDevicePropertyDeviceUID).as_deref() == Some(uid))
    }

    pub fn list_output_devices() -> Vec<OutputDevice> {
        let default_id = default_output_device_id().unwrap_or(0);
        let mut out: Vec<OutputDevice> = Vec::new();
        for id in all_device_ids() {
            // Output devices only: drop input-only endpoints (mics, etc.).
            if output_channel_count(id) == 0 {
                continue;
            }
            let is_alive = device_is_alive(id);
            if !is_alive {
                continue; // skip dead devices (e.g. an unplugged aggregate)
            }
            // A stable UID is the selection key — skip anything without one.
            let Some(uid) = read_cfstring(id, kAudioDevicePropertyDeviceUID) else {
                continue;
            };
            let name = read_cfstring(id, kAudioObjectPropertyName).unwrap_or_else(|| uid.clone());
            let transport = read_u32(
                id,
                kAudioDevicePropertyTransportType,
                kAudioObjectPropertyScopeGlobal,
            )
            .map(OutputTransport::from_transport_code)
            .unwrap_or(OutputTransport::Other);
            out.push(OutputDevice {
                id,
                uid,
                name,
                transport,
                is_default: id == default_id,
                is_alive,
            });
        }
        // Default first, then case-insensitive by name.
        out.sort_by(|a, b| {
            b.is_default
                .cmp(&a.is_default)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        out
    }

    pub fn set_default_output(uid: &str) -> Result<(), AudioError> {
        let device_id = device_id_for_uid(uid).ok_or_else(|| {
            AudioError::DeviceNotFound(format!("no output device with uid {uid}"))
        })?;
        let address = addr(
            kAudioHardwarePropertyDefaultOutputDevice,
            kAudioObjectPropertyScopeGlobal,
        );

        // Set, then read the default back and VERIFY it stuck. The AirPods
        // Continuity revert bug (FB15113809) makes the HAL return noErr from the
        // set yet silently revert to the previous device — so we re-assert once
        // and fail if it still won't hold, instead of lying that the switch
        // worked. `AudioObjectSetPropertyData` only reads `in_data`, so passing a
        // `&mut` local is safe.
        for _attempt in 0..2 {
            let mut id = device_id;
            let status = unsafe {
                AudioObjectSetPropertyData(
                    kAudioObjectSystemObject as AudioObjectID,
                    NonNull::from(&address),
                    0,
                    std::ptr::null(),
                    size_of::<AudioObjectID>() as u32,
                    NonNull::new(&mut id as *mut AudioObjectID as *mut c_void).unwrap(),
                )
            };
            if status != 0 {
                return Err(AudioError::Stream(format!(
                    "could not set the default output device (status {status})"
                )));
            }
            // The switch isn't always synchronous; let the HAL settle, then check.
            std::thread::sleep(Duration::from_millis(60));
            if default_output_device_id() == Some(device_id) {
                return Ok(());
            }
        }
        Err(AudioError::Stream(
            "the system reverted the output device (a connected device may be forcing itself as \
             the default)"
                .into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_codes_map_to_expected_variants() {
        assert_eq!(
            OutputTransport::from_transport_code(TRANSPORT_BUILT_IN),
            OutputTransport::BuiltIn
        );
        assert_eq!(
            OutputTransport::from_transport_code(TRANSPORT_USB),
            OutputTransport::Usb
        );
        assert_eq!(
            OutputTransport::from_transport_code(TRANSPORT_BLUETOOTH),
            OutputTransport::Bluetooth
        );
        // Bluetooth-LE folds into the same UI category as classic Bluetooth.
        assert_eq!(
            OutputTransport::from_transport_code(TRANSPORT_BLUETOOTH_LE),
            OutputTransport::Bluetooth
        );
        assert_eq!(
            OutputTransport::from_transport_code(TRANSPORT_HDMI),
            OutputTransport::Hdmi
        );
        assert_eq!(
            OutputTransport::from_transport_code(TRANSPORT_DISPLAY_PORT),
            OutputTransport::DisplayPort
        );
        assert_eq!(
            OutputTransport::from_transport_code(TRANSPORT_AIRPLAY),
            OutputTransport::AirPlay
        );
        assert_eq!(
            OutputTransport::from_transport_code(TRANSPORT_AGGREGATE),
            OutputTransport::Aggregate
        );
        assert_eq!(
            OutputTransport::from_transport_code(TRANSPORT_AUTO_AGGREGATE),
            OutputTransport::Aggregate
        );
        assert_eq!(
            OutputTransport::from_transport_code(TRANSPORT_THUNDERBOLT),
            OutputTransport::Thunderbolt
        );
        // Unknown codes fall back to `Other`.
        assert_eq!(
            OutputTransport::from_transport_code(0xDEAD_BEEF),
            OutputTransport::Other
        );
    }

    #[test]
    fn transport_serializes_to_lowercase_wire_form() {
        // The frontend union depends on these exact tags.
        assert_eq!(
            serde_json::to_string(&OutputTransport::BuiltIn).unwrap(),
            "\"builtin\""
        );
        assert_eq!(
            serde_json::to_string(&OutputTransport::DisplayPort).unwrap(),
            "\"displayport\""
        );
        assert_eq!(
            serde_json::to_string(&OutputTransport::AirPlay).unwrap(),
            "\"airplay\""
        );
    }

    #[test]
    fn output_device_serializes_camel_case_keys() {
        let dev = OutputDevice {
            id: 42,
            uid: "AppleHDAEngineOutput".into(),
            name: "MacBook Pro Speakers".into(),
            transport: OutputTransport::BuiltIn,
            is_default: true,
            is_alive: true,
        };
        let json = serde_json::to_string(&dev).unwrap();
        assert!(json.contains("\"isDefault\":true"), "{json}");
        assert!(json.contains("\"isAlive\":true"), "{json}");
        assert!(json.contains("\"transport\":\"builtin\""), "{json}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn total_channels_sums_every_buffer() {
        assert_eq!(total_channels(&[]), 0);
        assert_eq!(total_channels(&[2]), 2);
        // Planar/non-interleaved layouts report one channel per buffer.
        assert_eq!(total_channels(&[1, 1]), 2);
        assert_eq!(total_channels(&[2, 2, 2]), 6);
    }

    // The locally-defined FourCC codes must stay in lockstep with Core Audio.
    #[cfg(target_os = "macos")]
    #[test]
    fn local_transport_fourccs_match_core_audio() {
        use objc2_core_audio::{
            kAudioDeviceTransportTypeAirPlay, kAudioDeviceTransportTypeAggregate,
            kAudioDeviceTransportTypeAutoAggregate, kAudioDeviceTransportTypeBluetooth,
            kAudioDeviceTransportTypeBluetoothLE, kAudioDeviceTransportTypeBuiltIn,
            kAudioDeviceTransportTypeDisplayPort, kAudioDeviceTransportTypeHDMI,
            kAudioDeviceTransportTypeThunderbolt, kAudioDeviceTransportTypeUSB,
            kAudioDeviceTransportTypeVirtual,
        };
        assert_eq!(TRANSPORT_BUILT_IN, kAudioDeviceTransportTypeBuiltIn);
        assert_eq!(TRANSPORT_USB, kAudioDeviceTransportTypeUSB);
        assert_eq!(TRANSPORT_BLUETOOTH, kAudioDeviceTransportTypeBluetooth);
        assert_eq!(TRANSPORT_BLUETOOTH_LE, kAudioDeviceTransportTypeBluetoothLE);
        assert_eq!(TRANSPORT_HDMI, kAudioDeviceTransportTypeHDMI);
        assert_eq!(TRANSPORT_DISPLAY_PORT, kAudioDeviceTransportTypeDisplayPort);
        assert_eq!(TRANSPORT_AIRPLAY, kAudioDeviceTransportTypeAirPlay);
        assert_eq!(TRANSPORT_AGGREGATE, kAudioDeviceTransportTypeAggregate);
        assert_eq!(TRANSPORT_AUTO_AGGREGATE, kAudioDeviceTransportTypeAutoAggregate);
        assert_eq!(TRANSPORT_VIRTUAL, kAudioDeviceTransportTypeVirtual);
        assert_eq!(TRANSPORT_THUNDERBOLT, kAudioDeviceTransportTypeThunderbolt);
    }

    /// Listing must never panic on the dev host (it returns a possibly-empty
    /// list on macOS, or the cpal fallback elsewhere).
    #[test]
    fn listing_does_not_panic() {
        let _ = list_output_devices();
    }
}
