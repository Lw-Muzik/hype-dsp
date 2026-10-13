//! Hype Connect — this desktop as a player a browser can drive.
//!
//! Rust owns the socket and the tokens; TypeScript owns the semantics. The
//! [`hm_connect`] client keeps one socket to the hub; this module feeds it the
//! account's tokens, forwards what it hears to the webview as events, and
//! exposes the REST side (linking, devices) as commands:
//!
//! * `connect:inbound` — every `welcome`/`devices`/`state`/`cmd`/`rpc`/`error`
//!   (and, v1.1, `viz_sub`/`pong`) frame, as protocol JSON
//!   (`src/features/connect/` executes them);
//! * `connect:status` — [`ConnectStatusDto`] whenever the connection changes;
//! * `connect_send(frame)` — the player's `ack` / `state` / `rpc_result`, and
//!   (v1.1) its `ping` / `viz`.
//!
//! The webview doesn't open the socket itself: a `tauri://` page opening
//! `ws://` is mixed content, and the tokens already live here.
//!
//! The client runs on its own small tokio runtime, built on first use behind
//! `catch_unwind` (a panicking command never answers, leaving the UI's
//! promise pending forever — see `link.rs`'s `catch_build`). Every REST call
//! goes through the account's blocking client, which has a 15 s timeout.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use hm_connect::protocol::{
    Cap, ConflictBody, DeviceHello, DeviceKind, DeviceLinkRequest, DevicesResponse, Frame,
    LinkClaim, LinkPreview, LinkTicket,
};
use hm_connect::{
    ClientConfig, ClientEvent, ConnectHandle, ConnectionStatus, Renewal, StopReason, TokenProvider,
};
use hm_core::IpcError;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use super::account::AccountState;
use super::install::resolve_install_id;

const STATUS_EVENT: &str = "connect:status";
const INBOUND_EVENT: &str = "connect:inbound";

/// Persisted in `connect.json`. On by default: being reachable from the web is
/// the point of the feature, and the socket is idle between commands.
#[derive(Serialize, Deserialize)]
struct Settings {
    enabled: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// What the Settings card shows (`connect_status`, `connect:status`).
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectStatusDto {
    pub enabled: bool,
    /// This desktop's Connect device id — its install id.
    pub device_id: String,
    /// `None` while no client runs (disabled, or not started yet).
    pub connection: Option<ConnectionStatus>,
}

/// A "Link with your phone" ticket for the UI. The claim secret stays here.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkTicketDto {
    pub link_id: String,
    pub code: String,
    pub qr: String,
    pub expires_at: String,
}

/// Where a pending phone link stands: `pending`, `approved`, `denied`,
/// `expired`. Tokens from an approval are adopted here, never returned.
#[derive(Serialize)]
pub struct LinkPollDto {
    pub status: &'static str,
}

struct PendingLink {
    link_id: String,
    secret: String,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Run `build`, turning a panic into an `Err` (see the module docs).
fn catch_panic<T>(build: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(build)) {
        Ok(result) => result,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&'static str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown panic");
            Err(format!("Hype Connect crashed while starting: {msg}"))
        }
    }
}

/// The account session as the client's token source.
struct AccountTokens {
    app: AppHandle,
}

impl TokenProvider for AccountTokens {
    fn current(&self) -> Option<String> {
        self.app.state::<AccountState>().access_token()
    }

    fn renew(&self, rejected: &str) -> Renewal {
        self.app.state::<AccountState>().renew_access(rejected)
    }
}

pub struct ConnectState {
    app: AppHandle,
    settings_path: PathBuf,
    enabled: Mutex<bool>,
    runtime: OnceLock<Result<tokio::runtime::Runtime, String>>,
    client: Mutex<Option<ConnectHandle>>,
    /// Bumped on every start/stop, so a replaced client's late events are
    /// never mistaken for the current one's.
    generation: AtomicU64,
    pending_link: Mutex<Option<PendingLink>>,
    install_id: OnceLock<String>,
    device_name: OnceLock<String>,
}

impl ConnectState {
    pub fn new(app: AppHandle, settings_path: PathBuf) -> Self {
        let settings: Settings = std::fs::read_to_string(&settings_path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Self {
            app,
            settings_path,
            enabled: Mutex::new(settings.enabled),
            runtime: OnceLock::new(),
            client: Mutex::new(None),
            generation: AtomicU64::new(0),
            pending_link: Mutex::new(None),
            install_id: OnceLock::new(),
            device_name: OnceLock::new(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        *lock(&self.enabled)
    }

    fn install_id(&self) -> &str {
        self.install_id
            .get_or_init(|| resolve_install_id(&self.app))
    }

    fn device_name(&self) -> &str {
        // `device_name` shells out to `hostname`; once is enough.
        self.device_name.get_or_init(hm_link::device_name)
    }

    fn runtime(&self) -> Result<&tokio::runtime::Runtime, String> {
        self.runtime
            .get_or_init(|| {
                catch_panic(|| {
                    tokio::runtime::Builder::new_multi_thread()
                        .worker_threads(1)
                        .thread_name("hm-connect")
                        .enable_all()
                        .build()
                        .map_err(|e| format!("couldn't start Hype Connect: {e}"))
                })
                .inspect_err(|e| eprintln!("hype connect unavailable: {e}"))
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    pub fn status(&self) -> ConnectStatusDto {
        let connection = lock(&self.client).as_ref().map(ConnectHandle::status);
        self.status_with(connection)
    }

    fn status_with(&self, connection: Option<ConnectionStatus>) -> ConnectStatusDto {
        ConnectStatusDto {
            enabled: self.is_enabled(),
            device_id: self.install_id().to_string(),
            connection,
        }
    }

    fn emit_status(&self) {
        let _ = self.app.emit(STATUS_EVENT, self.status());
    }

    /// Start (or restart) the client. A running one is replaced.
    pub fn start(&self) -> Result<(), String> {
        catch_panic(|| self.start_inner())
    }

    fn start_inner(&self) -> Result<(), String> {
        let rt = self.runtime()?;
        let url = {
            let account = self.app.state::<AccountState>();
            hm_connect::rest::ws_url(account.api_base()).map_err(|e| e.to_string())?
        };
        let device = DeviceHello {
            id: self.install_id().to_string(),
            kind: DeviceKind::Desktop,
            name: self.device_name().to_string(),
            platform: std::env::consts::OS.to_string(),
            app_version: self.app.package_info().version.to_string(),
            caps: vec![Cap::Player, Cap::Yt, Cap::Local, Cap::Lyrics, Cap::Viz],
        };
        let tokens = Arc::new(AccountTokens {
            app: self.app.clone(),
        });
        let (handle, mut events) =
            hm_connect::spawn(ClientConfig::new(url, device), tokens, rt.handle());

        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        // Dropping the previous handle stops that client.
        drop(lock(&self.client).replace(handle));

        let app = self.app.clone();
        rt.spawn(async move {
            while let Some(event) = events.recv().await {
                let state = app.state::<ConnectState>();
                if state.generation.load(Ordering::SeqCst) != generation {
                    break; // replaced or stopped: its news is stale
                }
                match event {
                    ClientEvent::Frame(frame) => {
                        let _ = app.emit(INBOUND_EVENT, &*frame);
                    }
                    ClientEvent::Status(status) => {
                        if status
                            == (ConnectionStatus::Stopped {
                                reason: StopReason::Unlinked,
                            })
                        {
                            // 4003: this device was revoked. Its tokens are
                            // useless; the UI re-bootstraps an anonymous
                            // session (SessionGate) and the client stays off
                            // until the user reconnects.
                            app.state::<AccountState>().drop_session();
                        }
                        let _ = app.emit(STATUS_EVENT, state.status_with(Some(status)));
                    }
                }
            }
        });
        self.emit_status();
        Ok(())
    }

    /// Stop the client (if any) without changing the setting.
    fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(handle) = lock(&self.client).take() {
            handle.stop();
        }
        self.emit_status();
    }

    pub fn set_enabled(&self, on: bool) -> Result<(), String> {
        *lock(&self.enabled) = on;
        if let Some(dir) = self.settings_path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(&Settings { enabled: on }) {
            let _ = std::fs::write(&self.settings_path, json);
        }
        if on {
            self.start()
        } else {
            self.stop();
            Ok(())
        }
    }

    pub fn send(&self, frame: &Frame) -> Result<(), String> {
        let guard = lock(&self.client);
        let handle = guard.as_ref().ok_or("Hype Connect is off")?;
        handle.send(frame).map_err(|e| e.to_string())
    }
}

// ------------------------------------------------------------------ helpers

fn connect_err(message: impl Into<String>) -> IpcError {
    IpcError::new("connect", message)
}

/// Read a JSON body, or explain the status code.
fn json_body<T: serde::de::DeserializeOwned>(
    resp: reqwest::blocking::Response,
    what: &str,
) -> Result<T, IpcError> {
    let code = resp.status().as_u16();
    if !resp.status().is_success() {
        return Err(connect_err(format!("{what} failed ({code})")));
    }
    resp.json::<T>()
        .map_err(|e| connect_err(format!("{what}: unexpected reply ({e})")))
}

fn platform_name() -> String {
    std::env::consts::OS.to_string()
}

// ----------------------------------------------------------------- commands

#[tauri::command(async)]
pub fn connect_status(state: State<'_, ConnectState>) -> ConnectStatusDto {
    state.status()
}

/// Turn Hype Connect on/off (persisted). Turning it on also restarts a client
/// that stopped (after "unlinked" or "replaced").
#[tauri::command(async)]
pub fn connect_set_enabled(
    state: State<'_, ConnectState>,
    enabled: bool,
) -> Result<ConnectStatusDto, IpcError> {
    state.set_enabled(enabled).map_err(connect_err)?;
    Ok(state.status())
}

/// Send the player's `ack`, `state`, `rpc_result`, `ping` or `viz` to the hub. Sync on
/// purpose: commands run on the main thread in call order, so a state's
/// `seq` order is the order frames reach the socket.
#[tauri::command]
pub fn connect_send(state: State<'_, ConnectState>, frame: Frame) -> Result<(), IpcError> {
    state.send(&frame).map_err(connect_err)
}

/// "Link with your phone": ask the hub for a link ticket for this desktop.
/// The phone scans `qr` (or types `code`); poll with `connect_link_poll`.
#[tauri::command(async)]
pub fn connect_link_start(
    state: State<'_, ConnectState>,
    account: State<'_, AccountState>,
) -> Result<LinkTicketDto, IpcError> {
    let body = serde_json::to_value(DeviceLinkRequest {
        kind: DeviceKind::Desktop,
        name: state.device_name().to_string(),
        platform: Some(platform_name()),
        install_id: state.install_id().to_string(),
    })
    .map_err(|e| connect_err(e.to_string()))?;
    let resp = account
        .auth_send(
            reqwest::Method::POST,
            hm_connect::rest::LINK_DEVICE,
            Some(&body),
        )
        .map_err(connect_err)?;
    if resp.status().as_u16() == 403 {
        return Err(connect_err(
            "The server doesn't know this install yet — restart the app and try again",
        ));
    }
    let ticket: LinkTicket = json_body(resp, "Starting the link")?;
    *lock(&state.pending_link) = Some(PendingLink {
        link_id: ticket.link_id.clone(),
        secret: ticket.secret,
    });
    Ok(LinkTicketDto {
        link_id: ticket.link_id,
        code: ticket.code,
        qr: ticket.qr,
        expires_at: ticket.expires_at,
    })
}

/// Poll the pending phone link (the UI calls this every 2 s). On approval the
/// surviving account's tokens are adopted and the client reconnects under it.
#[tauri::command(async)]
pub fn connect_link_poll(
    state: State<'_, ConnectState>,
    account: State<'_, AccountState>,
) -> Result<LinkPollDto, IpcError> {
    let (link_id, secret) = match lock(&state.pending_link).as_ref() {
        Some(p) => (p.link_id.clone(), p.secret.clone()),
        None => return Ok(LinkPollDto { status: "expired" }),
    };
    let path = hm_connect::rest::link_claim(&link_id).map_err(|e| connect_err(e.to_string()))?;
    let resp = account
        .post(&path, serde_json::json!({ "secret": secret }))
        .map_err(connect_err)?;
    let claim: LinkClaim = json_body(resp, "Checking the link")?;
    let status = match claim {
        LinkClaim::Pending => return Ok(LinkPollDto { status: "pending" }),
        LinkClaim::Denied => "denied",
        LinkClaim::Expired => "expired",
        LinkClaim::Approved {
            access_token,
            refresh_token,
            ..
        } => {
            account.adopt_session(access_token, refresh_token);
            if state.is_enabled() {
                state.start().map_err(connect_err)?;
            }
            "approved"
        }
    };
    *lock(&state.pending_link) = None;
    Ok(LinkPollDto { status })
}

/// Abandon a pending phone link (the dialog was closed).
#[tauri::command]
pub fn connect_link_cancel(state: State<'_, ConnectState>) {
    *lock(&state.pending_link) = None;
}

/// "Link a browser": what is asking to link with this code?
#[tauri::command(async)]
pub fn connect_link_preview(
    account: State<'_, AccountState>,
    code: String,
) -> Result<LinkPreview, IpcError> {
    let path = hm_connect::rest::link_preview(&code)
        .map_err(|_| connect_err("Enter the 6-digit code shown in your browser"))?;
    let resp = account
        .auth_send(reqwest::Method::GET, &path, None)
        .map_err(connect_err)?;
    if resp.status().as_u16() == 404 {
        return Err(connect_err(
            "That code is wrong or has expired — get a new one",
        ));
    }
    json_body(resp, "Looking up the code")
}

/// Approve a pending link (this desktop's account is the one it joins).
#[tauri::command(async)]
pub fn connect_link_approve(
    account: State<'_, AccountState>,
    code: String,
) -> Result<(), IpcError> {
    let path = hm_connect::rest::link_approve(&code)
        .map_err(|_| connect_err("Enter the 6-digit code shown in your browser"))?;
    let resp = account
        .auth_send(reqwest::Method::POST, &path, None)
        .map_err(connect_err)?;
    match resp.status().as_u16() {
        200 | 201 => Ok(()),
        404 => Err(connect_err(
            "That code is wrong or has expired — get a new one",
        )),
        409 => {
            let code = resp
                .json::<ConflictBody>()
                .map(|b| b.code)
                .unwrap_or_default();
            Err(connect_err(if code == "BOTH_HAVE_ACCOUNTS" {
                "Both devices are signed in to different accounts — sign one out first"
            } else {
                "That link can't be approved"
            }))
        }
        code => Err(connect_err(format!("Approving failed ({code})"))),
    }
}

/// Refuse a pending link.
#[tauri::command(async)]
pub fn connect_link_deny(account: State<'_, AccountState>, code: String) -> Result<(), IpcError> {
    let path = hm_connect::rest::link_deny(&code)
        .map_err(|_| connect_err("Enter the 6-digit code shown in your browser"))?;
    let resp = account
        .auth_send(reqwest::Method::POST, &path, None)
        .map_err(connect_err)?;
    match resp.status().as_u16() {
        200 | 201 | 404 => Ok(()), // already gone is as good as denied
        code => Err(connect_err(format!("Denying failed ({code})"))),
    }
}

/// Every linked device of this account, online or not.
#[tauri::command(async)]
pub fn connect_devices(account: State<'_, AccountState>) -> Result<DevicesResponse, IpcError> {
    let resp = account
        .auth_send(reqwest::Method::GET, hm_connect::rest::DEVICES, None)
        .map_err(connect_err)?;
    json_body(resp, "Loading devices")
}

/// Unlink a device from this account.
#[tauri::command(async)]
pub fn connect_revoke(account: State<'_, AccountState>, id: String) -> Result<(), IpcError> {
    let path = hm_connect::rest::device(&id).map_err(|e| connect_err(e.to_string()))?;
    let resp = account
        .auth_send(reqwest::Method::DELETE, &path, None)
        .map_err(connect_err)?;
    match resp.status().as_u16() {
        200 | 201 | 204 | 404 => Ok(()),
        code => Err(connect_err(format!("Unlinking failed ({code})"))),
    }
}

#[cfg(test)]
mod tests {
    use super::{catch_panic, Settings};

    #[test]
    fn catch_panic_turns_a_panic_into_an_error() {
        let r: Result<(), String> = catch_panic(|| panic!("boom"));
        assert!(r.unwrap_err().contains("boom"));
        assert_eq!(catch_panic(|| Ok::<_, String>(7)), Ok(7));
    }

    #[test]
    fn connect_is_on_unless_turned_off() {
        assert!(Settings::default().enabled);
        let off: Settings = serde_json::from_str(r#"{"enabled":false}"#).unwrap();
        assert!(!off.enabled);
    }
}
