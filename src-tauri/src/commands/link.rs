//! Phone Link commands: discover phones on the LAN, pair via PIN, browse the
//! phone's library, and stream a track through the DSP chain.
//!
//! The heavy lifting (mDNS, pairing handshake, token store, URL resolution)
//! lives in the pure `hm-link` crate; these commands are the thin bridge that
//! also routes the resolved stream URL into the audio engine.

use std::time::Duration;

use hm_audio::AudioEngine;
use hm_core::IpcError;
use hm_link::{LinkState, PhoneDevice, PhoneTrack};
use tauri::State;

/// Browse the LAN (~2.5 s) for phones sharing their library.
#[tauri::command(async)]
pub fn link_discover(link: State<'_, LinkState>) -> Result<Vec<PhoneDevice>, IpcError> {
    link.discover(Duration::from_millis(2500))
        .map_err(|e| IpcError::new("link", e))
}

/// Phones we've already paired with (silent reconnect — no PIN needed).
#[tauri::command]
pub fn link_paired(link: State<'_, LinkState>) -> Vec<PhoneDevice> {
    link.paired()
}

/// Pair with a phone using the 6-digit PIN it's showing.
#[tauri::command(async)]
pub fn link_pair(
    link: State<'_, LinkState>,
    host: String,
    port: u16,
    name: String,
    device_id: String,
    pin: String,
) -> Result<PhoneDevice, IpcError> {
    link.pair(&host, port, &name, &device_id, &pin)
        .map_err(|e| IpcError::new("link", e))
}

/// Pair with a phone by typing its address (`host:port`) + PIN — no mDNS
/// discovery needed (works when discovery can't see the phone, or across
/// networks over a VPN).
#[tauri::command(async)]
pub fn link_pair_address(
    link: State<'_, LinkState>,
    host: String,
    port: u16,
    pin: String,
) -> Result<PhoneDevice, IpcError> {
    link.pair_by_address(&host, port, &pin)
        .map_err(|e| IpcError::new("link", e))
}

/// Forget a paired phone.
#[tauri::command]
pub fn link_unpair(link: State<'_, LinkState>, device_id: String) {
    link.unpair(&device_id);
}

/// Fetch a paired phone's track list.
#[tauri::command(async)]
pub fn link_library(
    link: State<'_, LinkState>,
    device_id: String,
) -> Result<Vec<PhoneTrack>, IpcError> {
    link.library(&device_id).map_err(|e| IpcError::new("link", e))
}

/// Fetch a track's embedded artwork as a `data:` URI, or `None` if it has none.
/// Never errors — a missing cover just falls back to the gradient placeholder.
#[tauri::command(async)]
pub fn link_artwork(
    link: State<'_, LinkState>,
    device_id: String,
    track_id: String,
) -> Option<String> {
    link.artwork_data_uri(&device_id, &track_id)
}

/// Fetch a phone track's lyrics (a `.lrc` sidecar the user downloaded next to
/// the music, or embedded lyrics), as raw LRC/plain text. `None` when none.
/// Never errors — missing lyrics just fall back to the online sources.
#[tauri::command(async)]
pub fn link_lyrics(
    link: State<'_, LinkState>,
    device_id: String,
    track_id: String,
) -> Option<String> {
    link.lyrics(&device_id, &track_id)
}

/// Stream one track from the phone through the enhancement chain.
/// `duration_secs`, when the phone already knows the track length, makes the
/// stream seekable and shows its duration immediately.
#[tauri::command(async)]
pub fn link_play(
    link: State<'_, LinkState>,
    engine: State<'_, AudioEngine>,
    device_id: String,
    track_id: String,
    ext: String,
    duration_secs: Option<f64>,
) -> Result<(), IpcError> {
    let (url, headers) = link
        .stream_target(&device_id, &track_id, &ext)
        .map_err(|e| IpcError::new("link", e))?;
    engine
        .play_stream(url, headers, duration_secs)
        .map_err(Into::into)
}
