//! Phone Link commands: discover phones on the LAN, pair via PIN, browse the
//! phone's library, and stream a track through the DSP chain.
//!
//! The heavy lifting (mDNS, pairing handshake, token store, URL resolution)
//! lives in the pure `hm-link` crate; these commands are the thin bridge that
//! also routes the resolved stream URL into the audio engine.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use hm_audio::stream_queue::{StreamResolver, StreamTarget};
use hm_audio::AudioEngine;
use hm_core::IpcError;
use hm_link::{LinkState, PhoneDevice, PhoneTrack};
use serde::{Deserialize, Serialize};
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
                let link = app.state::<LinkState>();
                link.update_addresses(std::slice::from_ref(&dev));
                // A *paired* phone (re)appearing means it's reachable now — tell
                // the UI so it can auto-sync that phone's library without a
                // relaunch. Fires app-wide (discovery runs for the whole session),
                // so it works even when the Phone screen isn't open.
                if link.is_paired(&dev.id) {
                    let _ = app.emit("link:paired_online", dev.id.clone());
                }
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

/// Run a remote-link build step, converting a panic into an `Err`.
///
/// A panicking Tauri command never sends its response, so the invoking promise
/// stays pending forever — in the field that was the "Pair a phone" button
/// stuck on "Preparing…" with no error anywhere. iroh's endpoint init touches
/// OS-specific machinery (sockets, route monitoring, system DNS config) that
/// has panicked on platforms dev machines don't exercise; this turns that into
/// an error the card can actually show. `AssertUnwindSafe` is fine: on panic
/// the partially-built state is discarded, never observed.
fn catch_build<T>(build: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(build)) {
        Ok(result) => result,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&'static str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown panic");
            Err(format!("remote link initialization crashed: {msg}"))
        }
    }
}

/// Lazily-constructed [`hm_remote::RemoteManager`].
///
/// Building the manager spins up a dedicated multi-thread tokio runtime and an
/// iroh endpoint that connects to the n0 relays and stays connected for the
/// app's lifetime — far too heavy to pay unconditionally in `setup()` (it used
/// to block there, before first paint) for a feature many users never touch.
/// Instead:
/// * every remote command funnels through [`Self::manager`], which builds it
///   on first use;
/// * at startup, `lib.rs` spawns a background thread that builds it **only if
///   the persisted peers file is non-empty**, so previously-paired phones
///   silently reconnect exactly as before — without ever blocking setup.
pub struct RemoteState {
    app: AppHandle,
    secret_path: PathBuf,
    store_path: PathBuf,
    /// `Err` caches a failed construction for the session — matching the old
    /// behaviour, where a failure in `setup()` left remote link unavailable.
    cell: OnceLock<Result<hm_remote::RemoteManager, String>>,
}

impl RemoteState {
    pub fn new(app: AppHandle, secret_path: PathBuf, store_path: PathBuf) -> Self {
        Self {
            app,
            secret_path,
            store_path,
            cell: OnceLock::new(),
        }
    }

    /// Whether any remote phone is persisted on disk — answered from the peers
    /// file, so it never triggers a build.
    pub fn has_known_peers(&self) -> bool {
        std::fs::read(&self.store_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<serde_json::Value>>(&b).ok())
            .is_some_and(|peers| !peers.is_empty())
    }

    /// The already-built manager, if any (non-blocking) — for commands that are
    /// correct no-ops when nothing was ever built, like cancelling a pairing
    /// session that can't exist yet.
    fn built(&self) -> Option<&hm_remote::RemoteManager> {
        self.cell.get().and_then(|r| r.as_ref().ok())
    }

    /// Build-on-first-use funnel. The first caller pays the endpoint
    /// construction (and others block on it), so call this from `(async)`
    /// commands or background threads only. The `on_paired` callback registers
    /// the phone into [`LinkState`] as a loopback proxy and notifies the UI —
    /// the same wiring `setup()` used to do.
    pub fn manager(&self) -> Result<&hm_remote::RemoteManager, String> {
        self.cell
            .get_or_init(|| {
                let pair_handle = self.app.clone();
                catch_build(|| {
                    hm_remote::RemoteManager::new(
                        self.secret_path.clone(),
                        self.store_path.clone(),
                        hm_link::device_name(),
                        true,
                        move |phone| {
                            pair_handle.state::<LinkState>().register_remote(
                                phone.id.clone(),
                                phone.name.clone(),
                                phone.port,
                                phone.token.clone(),
                            );
                            let _ = pair_handle.emit("link:remote_connected", &phone.id);
                        },
                    )
                    .map_err(|e| e.to_string())
                })
                .inspect_err(|e| eprintln!("remote phone link unavailable: {e}"))
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

/// Open a pairing session for a phone on a *different* network and return the
/// QR payload + 6-digit PIN to display. The phone scans the QR and connects
/// peer-to-peer over iroh (registered into [`LinkState`] by the `on_paired`
/// callback wired in [`RemoteState::manager`]).
// `(async)`: the first call builds the iroh endpoint (network setup, ~seconds
// worst case) — never on the Tauri main thread.
#[tauri::command(async)]
pub fn link_remote_qr(
    remote: State<'_, RemoteState>,
) -> Result<hm_remote::PairingInfo, IpcError> {
    Ok(remote
        .manager()
        .map_err(|e| IpcError::new("remote", e))?
        .start_pairing(Duration::from_secs(300)))
}

/// Close any open remote pairing session.
#[tauri::command]
pub fn link_remote_cancel(remote: State<'_, RemoteState>) {
    // A session can only exist once the manager was built (by `link_remote_qr`),
    // so never build the endpoint just to cancel nothing.
    if let Some(manager) = remote.built() {
        manager.cancel_pairing();
    }
}

/// Current status of every paired remote phone (online = tunnel connected).
// `(async)`: may wait on (or trigger) the endpoint build when peers exist.
#[tauri::command(async)]
pub fn link_remote_status(remote: State<'_, RemoteState>) -> Vec<hm_remote::RemotePhoneStatus> {
    // Nothing built and nothing ever paired → the answer is an empty list;
    // don't build a relay-connected endpoint just to say so.
    if remote.built().is_none() && !remote.has_known_peers() {
        return Vec::new();
    }
    remote
        .manager()
        .map(|m| m.remote_phones())
        .unwrap_or_default()
}

/// (Re)dial every known remote phone and register the connected ones into the
/// device store so their libraries load. Returns the refreshed status list.
#[tauri::command(async)]
pub fn link_remote_connect(
    remote: State<'_, RemoteState>,
    link: State<'_, LinkState>,
    app: AppHandle,
) -> Vec<hm_remote::RemotePhoneStatus> {
    // No peers on disk and nothing built → there is nothing to dial; keep the
    // endpoint unbuilt (same empty result either way).
    if remote.built().is_none() && !remote.has_known_peers() {
        return Vec::new();
    }
    let Ok(remote) = remote.manager() else {
        return Vec::new();
    };
    for phone in remote.connect_known() {
        link.register_remote(phone.id.clone(), phone.name, phone.port, phone.token);
        let _ = app.emit("link:remote_connected", &phone.id);
    }
    remote.remote_phones()
}

/// Forget a remote phone: drop its tunnel and remove it from both stores.
// `(async)`: may wait on the endpoint build kicked off by the startup
// reconnect thread (a forgettable peer implies a non-empty peers file).
#[tauri::command(async)]
pub fn link_remote_forget(
    remote: State<'_, RemoteState>,
    link: State<'_, LinkState>,
    device_id: String,
) {
    // Forgetting must reach the manager's persisted peer store. If any peer
    // exists the startup thread already built (or is building) the manager, so
    // this doesn't construct a fresh endpoint in practice.
    if remote.built().is_some() || remote.has_known_peers() {
        if let Ok(manager) = remote.manager() {
            manager.forget(&device_id);
        }
    }
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
    // `fresh` is nothing to honour here: the url is derived per call from the
    // paired device's address and token, never held.
    let resolver: StreamResolver = Arc::new(move |i: usize, _fresh: bool| {
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

/// Progress of a file being sent to a phone, emitted on `link:upload`.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadProgress {
    /// The source path — identifies which transfer this is, since several can
    /// run at once.
    pub path: String,
    pub sent: u64,
    pub total: u64,
    pub done: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Send a local audio file to a paired phone, into its music library.
///
/// This is the only *write* toward the phone — everything else here pulls from
/// it. It fills a real gap: there was previously no way to put a track the user
/// already owns onto their phone. It also needs no transport work of its own,
/// because `hm-remote` tunnels bytes both ways over a desktop-dialled QUIC
/// connection, so this reaches a phone on cellular exactly as it does one on the
/// LAN.
// `(async)`: streams a whole file over the network — seconds to minutes.
#[tauri::command(async)]
pub fn link_upload(
    app: AppHandle,
    link: State<'_, LinkState>,
    device_id: String,
    path: String,
) -> Result<(), IpcError> {
    let source = PathBuf::from(&path);
    let progress_app = app.clone();
    let progress_path = path.clone();
    let result = link.upload(&device_id, &source, move |sent, total| {
        let _ = progress_app.emit(
            "link:upload",
            UploadProgress {
                path: progress_path.clone(),
                sent,
                total,
                done: false,
                error: None,
            },
        );
    });

    match result {
        Ok(_) => {
            let _ = app.emit(
                "link:upload",
                UploadProgress {
                    path,
                    sent: 0,
                    total: 0,
                    done: true,
                    error: None,
                },
            );
            Ok(())
        }
        Err(e) => {
            let _ = app.emit(
                "link:upload",
                UploadProgress {
                    path,
                    sent: 0,
                    total: 0,
                    done: true,
                    error: Some(e.clone()),
                },
            );
            Err(IpcError::new("link", e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `catch_build` exists because a panicking iroh endpoint init left the
    // "Pair a phone" button on "Preparing…" forever in the field: Tauri never
    // answers an invoke whose handler panicked, so the UI promise never
    // settles. Converting the panic to an `Err` makes the card show a real,
    // reportable error instead.

    #[test]
    fn catch_build_passes_ok_through() {
        assert_eq!(catch_build(|| Ok::<_, String>(7)), Ok(7));
    }

    #[test]
    fn catch_build_passes_err_through() {
        assert_eq!(
            catch_build(|| Err::<(), _>("plain failure".to_string())),
            Err("plain failure".to_string())
        );
    }

    #[test]
    fn catch_build_converts_a_str_panic_into_err() {
        let err = catch_build(|| -> Result<(), String> { panic!("iroh exploded") })
            .expect_err("panic must become Err");
        assert!(err.contains("iroh exploded"), "got: {err}");
    }

    #[test]
    fn catch_build_converts_a_string_panic_into_err() {
        let boom = String::from("formatted failure 42");
        let err = catch_build(move || -> Result<(), String> {
            panic!("{}", boom)
        })
        .expect_err("panic must become Err");
        assert!(err.contains("formatted failure 42"), "got: {err}");
    }
}
