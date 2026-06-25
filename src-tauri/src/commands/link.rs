//! Phone Link commands: discover phones on the LAN, pair via PIN, browse the
//! phone's library, and stream a track through the DSP chain.
//!
//! The heavy lifting (mDNS, pairing handshake, token store, URL resolution)
//! lives in the pure `hm-link` crate; these commands are the thin bridge that
//! also routes the resolved stream URL into the audio engine.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hm_audio::stream_queue::{StreamResolver, StreamTarget};
use hm_audio::AudioEngine;
use hm_core::IpcError;
use hm_link::{LinkState, PhoneDevice, PhoneTrack};
use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager, State};

/// Managed handle to the continuous-discovery thread (if running).
#[derive(Default)]
pub struct DiscoveryState {
    stop: Mutex<Option<Arc<AtomicBool>>>,
}

/// Start streaming discovered phones to the UI over `link:phone_found` events —
/// a continuous mDNS browse that surfaces a phone the instant it appears (no
/// polling / refresh). Idempotent; replaces any running watcher.
#[tauri::command]
pub fn link_discover_start(app: AppHandle, disc: State<'_, DiscoveryState>) {
    if let Some(prev) = disc.stop.lock().expect("discovery poisoned").take() {
        prev.store(true, Ordering::Relaxed);
    }
    let stop = Arc::new(AtomicBool::new(false));
    *disc.stop.lock().expect("discovery poisoned") = Some(stop.clone());
    let _ = std::thread::Builder::new()
        .name("hm-link-watch".into())
        .spawn(move || {
            hm_link::watch(stop, move |dev| {
                // Silently update a paired phone's stored address if its IP
                // changed, so streaming keeps working.
                app.state::<LinkState>()
                    .update_addresses(std::slice::from_ref(&dev));
                let _ = app.emit("link:phone_found", dev);
            });
        });
}

/// Stop the continuous discovery (e.g. when leaving the Phone screen).
#[tauri::command]
pub fn link_discover_stop(disc: State<'_, DiscoveryState>) {
    if let Some(stop) = disc.stop.lock().expect("discovery poisoned").take() {
        stop.store(true, Ordering::Relaxed);
    }
}

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
///
/// Routes through the same ping-then-pair path as the manual "connect by
/// address" form (which is proven to work): we ping the discovered address
/// first so an unreachable/wrong host fails fast with the IP in the message,
/// instead of hanging on the pairing POST. The discovered `name`/`device_id`
/// are superseded by what the phone reports on `/ping`.
#[tauri::command(async)]
pub fn link_pair(
    link: State<'_, LinkState>,
    host: String,
    port: u16,
    name: String,
    device_id: String,
    pin: String,
) -> Result<PhoneDevice, IpcError> {
    let _ = (name, device_id);
    link.pair_by_address(&host, port, &pin)
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

// ------------------------------------------------- remote (cross-network) link

/// Open a pairing session for a phone on a *different* network and return the
/// QR payload + 6-digit PIN to display. The phone scans the QR and connects
/// peer-to-peer over iroh (registered into [`LinkState`] by the `on_paired`
/// callback wired in `lib.rs`).
#[tauri::command]
pub fn link_remote_qr(remote: State<'_, hm_remote::RemoteManager>) -> hm_remote::PairingInfo {
    remote.start_pairing(Duration::from_secs(300))
}

/// Close any open remote pairing session.
#[tauri::command]
pub fn link_remote_cancel(remote: State<'_, hm_remote::RemoteManager>) {
    remote.cancel_pairing();
}

/// Current status of every paired remote phone (online = tunnel connected).
#[tauri::command]
pub fn link_remote_status(
    remote: State<'_, hm_remote::RemoteManager>,
) -> Vec<hm_remote::RemotePhoneStatus> {
    remote.remote_phones()
}

/// (Re)dial every known remote phone and register the connected ones into the
/// device store so their libraries load. Returns the refreshed status list.
#[tauri::command(async)]
pub fn link_remote_connect(
    remote: State<'_, hm_remote::RemoteManager>,
    link: State<'_, LinkState>,
    app: AppHandle,
) -> Vec<hm_remote::RemotePhoneStatus> {
    for phone in remote.connect_known() {
        link.register_remote(phone.id.clone(), phone.name, phone.port, phone.token);
        let _ = app.emit("link:remote_connected", &phone.id);
    }
    remote.remote_phones()
}

/// Forget a remote phone: drop its tunnel and remove it from both stores.
#[tauri::command]
pub fn link_remote_forget(
    remote: State<'_, hm_remote::RemoteManager>,
    link: State<'_, LinkState>,
    device_id: String,
) {
    remote.forget(&device_id);
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

/// One track in a phone crossfade/gapless queue.
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PhoneQueueItem {
    pub id: String,
    pub ext: String,
}

/// Play a queue of phone tracks gaplessly / crossfading. Each track's LAN stream
/// URL is resolved lazily via the paired device's `stream_target`; only the
/// current + next track are streamed/decoded (see `StreamQueueSource`).
#[tauri::command]
pub fn link_play_queue(
    app: AppHandle,
    engine: State<'_, AudioEngine>,
    device_id: String,
    items: Vec<PhoneQueueItem>,
    start: usize,
) -> Result<(), IpcError> {
    if items.is_empty() {
        return Err(IpcError::new("invalid", "empty phone queue"));
    }
    let count = items.len();
    let items = Arc::new(items);
    let device_id = Arc::new(device_id);
    let resolver: StreamResolver = Arc::new(move |i: usize| {
        let item = items.get(i).ok_or_else(|| "queue index out of range".to_string())?;
        let (url, headers) =
            app.state::<LinkState>()
                .stream_target(&device_id, &item.id, &item.ext)?;
        Ok(StreamTarget {
            url,
            headers,
            ext: Some(item.ext.clone()),
        })
    });
    engine
        .play_stream_queue(resolver, count, start)
        .map_err(Into::into)
}
