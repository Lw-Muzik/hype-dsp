//! Device enumeration over `cpal`.
//!
//! This is the first real, working piece of the audio layer: it proves the
//! platform backend links and lets the UI list output/input devices. Selecting
//! and streaming to a device arrives with the engine in Phase 2.

use cpal::traits::HostTrait;
use serde::{Deserialize, Serialize};

use crate::error::AudioError;

/// A device as presented to the UI device picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    /// Backend-reported device name, used as its selection key.
    pub name: String,
    /// Whether this is the host's current default device.
    pub is_default: bool,
}

fn collect(
    devices: impl Iterator<Item = cpal::Device>,
    default_name: Option<&str>,
) -> Vec<DeviceInfo> {
    devices
        .map(|device| {
            // cpal 0.18 exposes the device name via `Display`.
            let name = device.to_string();
            let is_default = default_name == Some(name.as_str());
            DeviceInfo { name, is_default }
        })
        .collect()
}

/// List output (playback) devices on the default host.
pub fn list_output_devices() -> Result<Vec<DeviceInfo>, AudioError> {
    let host = cpal::default_host();
    let default_name = host.default_output_device().map(|d| d.to_string());
    let devices = host
        .output_devices()
        .map_err(|e| AudioError::Host(e.to_string()))?;
    Ok(collect(devices, default_name.as_deref()))
}

/// List input (capture) devices on the default host.
pub fn list_input_devices() -> Result<Vec<DeviceInfo>, AudioError> {
    let host = cpal::default_host();
    let default_name = host.default_input_device().map(|d| d.to_string());
    let devices = host
        .input_devices()
        .map_err(|e| AudioError::Host(e.to_string()))?;
    Ok(collect(devices, default_name.as_deref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Enumeration must succeed on the dev target (a list, possibly empty,
    /// never a panic). This is the Module 1 "report devices" acceptance.
    #[test]
    fn output_enumeration_does_not_error() {
        let result = list_output_devices();
        assert!(result.is_ok(), "output enumeration failed: {result:?}");
    }
}
