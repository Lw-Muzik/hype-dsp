//! System-wide EQ on Linux — backend dispatch.
//!
//! Linux ships two audio-server families, so a single mechanism can't serve
//! everyone. This module picks the right backend at runtime and presents the one
//! type (`LinuxSystemEq`) and `available()` facade the engine already calls
//! (`AudioEngine::start_system_eq` / `hm_audio::system_eq_available`):
//!
//! - **PipeWire** (the ~90–95 % case on 2026 desktops) → [`crate::system_eq_pipewire`],
//!   a native client that inserts a real graph node and moves app streams via
//!   `target.object` metadata. Transparent, zero-config, crash-safe, low-latency
//!   — the macOS-parity path. WirePlumber's routing policy makes the older
//!   `pactl` null-sink trick unreliable here, which is why this exists.
//! - **classic PulseAudio** (a shrinking minority) → [`crate::system_eq_pulse`],
//!   the repaired virtual-sink + `parec`/`pacat` approach, which *is* reliable on
//!   a real PulseAudio core.
//! - **neither** → honestly unavailable; the UI shows no toggle.
//!
//! Selection is deliberately PipeWire-first: on a PipeWire system `pactl` also
//! works (via `pipewire-pulse`), but the CLI path is the broken one there, so we
//! must never fall back to it while PipeWire is live.

#![cfg(target_os = "linux")]

use std::sync::Arc;

use arc_swap::ArcSwap;
use hm_core::EngineState;

use crate::error::AudioError;
use crate::system_eq_pipewire::PipewireSystemEq;
use crate::system_eq_pulse::PulseSystemEq;

/// Which system-EQ backend to drive for the current session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    /// A live PipeWire server → native client (preferred).
    Pipewire,
    /// A classic PulseAudio server → repaired virtual-sink CLI path.
    Pulse,
    /// No supported server reachable.
    Unavailable,
}

/// Choose a backend from what's reachable. PipeWire wins whenever it's running,
/// because its `pactl` compatibility layer makes the Pulse CLI *look* usable
/// while actually being the unreliable path. Pure so it is unit-tested on the
/// non-Linux dev host.
fn classify_stack(pipewire_running: bool, pulse_usable: bool) -> Backend {
    if pipewire_running {
        Backend::Pipewire
    } else if pulse_usable {
        Backend::Pulse
    } else {
        Backend::Unavailable
    }
}

fn detect_backend() -> Backend {
    classify_stack(
        crate::system_eq_pipewire::available(),
        crate::system_eq_pulse::available(),
    )
}

/// Whether *some* Linux backend can equalize system audio for this session.
pub fn available() -> bool {
    detect_backend() != Backend::Unavailable
}

/// A running Linux system-wide EQ session. Wraps whichever backend was selected;
/// dropping it runs that backend's teardown (routing restore, node cleanup).
pub enum LinuxSystemEq {
    Pipewire(PipewireSystemEq),
    Pulse(PulseSystemEq),
}

impl LinuxSystemEq {
    /// Start system-wide EQ using the best available backend. `state` is the
    /// engine's live parameter handle (EQ/effects/power/volume).
    pub fn start(state: Arc<ArcSwap<EngineState>>) -> Result<Self, AudioError> {
        match detect_backend() {
            Backend::Pipewire => Ok(Self::Pipewire(PipewireSystemEq::start(state)?)),
            Backend::Pulse => Ok(Self::Pulse(PulseSystemEq::start(state)?)),
            Backend::Unavailable => Err(AudioError::Unavailable(
                "no supported Linux audio server (PipeWire or PulseAudio) is available".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_pipewire_even_when_pulse_looks_usable() {
        // On PipeWire systems `pactl` works via pipewire-pulse, so both probes
        // can be true — we must still pick PipeWire, never the broken CLI path.
        assert_eq!(classify_stack(true, true), Backend::Pipewire);
        assert_eq!(classify_stack(true, false), Backend::Pipewire);
    }

    #[test]
    fn falls_back_to_pulse_only_without_pipewire() {
        assert_eq!(classify_stack(false, true), Backend::Pulse);
    }

    #[test]
    fn unavailable_when_neither_is_present() {
        assert_eq!(classify_stack(false, false), Backend::Unavailable);
    }
}
