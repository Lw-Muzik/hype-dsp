//! Auto-update: check on a cadence, download quietly, install at quit.
//!
//! # Why this lives in Rust rather than the webview
//!
//! The plugin exposes the same three steps to JavaScript, and driving them from
//! there is the more common shape. It is the wrong one here. A downloaded update
//! is held **in memory** — the plugin buffers the whole bundle with no resume —
//! so whoever owns the download owns tens of megabytes for as long as it is
//! staged. Tying that to the webview means the download dies with a reload, and
//! it means the quit path has to make an async round trip into JavaScript at the
//! exact moment the window is going away. Keeping it here makes quit a plain
//! function call against state we already hold.
//!
//! # When we install
//!
//! At quit, and only at quit. This app holds a Core Audio process tap and, on
//! Windows, a bundled audio driver. The updater's Windows path ends in
//! `std::process::exit(0)` — no unwinding, no `Drop`, no flush — so installing
//! mid-session would tear that down by simply vanishing. At quit the engine is
//! already being shut down and nothing is playing.
//!
//! [`stage_teardown`] is what makes that true rather than merely likely: it runs
//! before the installer, on every path that reaches it.
//!
//! # What is deliberately not here
//!
//! No retry loop. The download is unresumable and RAM-buffered, so a failed
//! attempt is retried on the *next cadence tick*, not immediately — retrying a
//! 45 MB download in a tight loop is worse than not updating.

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;

/// Diagnostics go to stderr — the updater runs unattended and its failures are
/// otherwise invisible.
fn log(msg: &str) {
    eprintln!("[updater] {msg}");
}

use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tauri_plugin_updater::{Update, UpdaterExt};

use hm_core::IpcError;

/// How long after launch the first check waits.
///
/// Long enough to stay out of the way of the library load, which is the slowest
/// and most visible thing the app does on startup. An update is never urgent
/// enough to compete with the user seeing their music.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(90);

/// Gap between checks thereafter. Six hours is often enough that a release is
/// picked up the same day, and rare enough to be invisible.
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// Event carrying every status change to the UI.
pub const EVENT: &str = "updater://status";

/// What the updater is doing, as the UI sees it.
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Status {
    /// Nothing known yet, or the last check found nothing.
    #[default]
    Idle,
    Checking,
    /// Downloading, with progress when the server sent a length. `total` is
    /// `None` for a chunked response — the bar has to degrade to indeterminate
    /// rather than pretend.
    Downloading { received: u64, total: Option<u64> },
    /// Downloaded, verified, and waiting for the app to quit.
    Ready { version: String, notes: Option<String> },
    /// The last attempt failed. Held for the UI; the next tick retries.
    Failed { message: String },
}

/// The staged update, and the bytes it will install.
///
/// Kept together because `Update::install` needs both, and separating them
/// would allow the pair to drift into "bytes for a version we no longer have".
struct Staged {
    update: Update,
    bytes: Vec<u8>,
}

#[derive(Default)]
pub struct UpdaterState {
    status: Mutex<Status>,
    staged: Mutex<Option<Staged>>,
}

impl UpdaterState {
    fn set_status<R: Runtime>(&self, app: &AppHandle<R>, next: Status) {
        if let Ok(mut guard) = self.status.lock() {
            if *guard == next {
                return; // don't spam the UI with identical frames
            }
            *guard = next.clone();
        }
        let _ = app.emit(EVENT, next);
    }

    fn snapshot(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Whether an update is downloaded and waiting.
    pub fn has_staged(&self) -> bool {
        self.staged.lock().map(|s| s.is_some()).unwrap_or(false)
    }
}

/// The current status, for the UI to render on mount (events cover the rest).
#[tauri::command]
pub fn updater_status(state: State<'_, UpdaterState>) -> Status {
    state.snapshot()
}

/// Check now, and download if something is found. Used by the Settings button;
/// the cadence loop calls the same code.
#[tauri::command]
pub async fn updater_check_now(app: AppHandle) -> Result<(), IpcError> {
    check_and_stage(&app).await;
    Ok(())
}

/// Install the staged update and restart, now, at the user's request.
///
/// The same teardown as the quit path runs first. On Windows `install` never
/// returns — the installer exits the process — so anything after it is
/// best-effort by definition.
#[tauri::command]
pub async fn updater_restart_now(app: AppHandle) -> Result<(), IpcError> {
    install_staged(&app).map_err(|e| IpcError::new("updater", e))?;
    // Only reached where `install` returns (macOS/Linux); on Windows the
    // installer has already exited the process. `restart` diverges.
    app.restart();
}

/// Start the background cadence. Returns immediately.
pub fn spawn_cadence(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK_DELAY).await;
        loop {
            // Once something is staged there is nothing left to look for — the
            // install happens at quit, and checking again would only discard a
            // download we already paid for.
            if !app.state::<UpdaterState>().has_staged() {
                check_and_stage(&app).await;
            }
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}

/// Check, and on a hit download and stage it.
async fn check_and_stage(app: &AppHandle) {
    let state = app.state::<UpdaterState>();
    state.set_status(app, Status::Checking);

    let updater = match app.updater() {
        Ok(u) => u,
        Err(e) => {
            state.set_status(app, Status::Failed { message: e.to_string() });
            return;
        }
    };

    let update = match updater.check().await {
        // No update. Back to idle — this is the overwhelmingly common outcome
        // and must never look like an error.
        Ok(None) => {
            state.set_status(app, Status::Idle);
            return;
        }
        Ok(Some(u)) => u,
        Err(e) => {
            // Offline, endpoint down, or an unparseable manifest. Logged, not
            // surfaced loudly: a failed check is not the user's problem.
            log(&format!("updater: check failed: {e}"));
            state.set_status(app, Status::Failed { message: e.to_string() });
            return;
        }
    };

    let version = update.version.clone();
    let notes = update.body.clone();
    log(&format!("updater: {version} available, downloading"));

    let mut received: u64 = 0;
    let downloaded = update
        .download(
            |chunk, total| {
                received += chunk as u64;
                app.state::<UpdaterState>()
                    .set_status(app, Status::Downloading { received, total });
            },
            || {},
        )
        .await;

    match downloaded {
        Ok(bytes) => {
            log(&format!("updater: {version} staged ({} bytes)", bytes.len()));
            if let Ok(mut slot) = state.staged.lock() {
                *slot = Some(Staged { update, bytes });
            }
            state.set_status(app, Status::Ready { version, notes });
        }
        Err(e) => {
            // Unresumable: the partial download is gone. The next tick starts over.
            log(&format!("updater: download failed: {e}"));
            state.set_status(app, Status::Failed { message: e.to_string() });
        }
    }
}

/// Bring the audio stack down before an installer replaces the binaries.
///
/// The reason this is not left to `Drop`: the Windows installer path ends in
/// `std::process::exit(0)`, which runs no destructors at all. On macOS a live
/// process tap and its private aggregate device would be left behind by a
/// vanishing process; on Windows the bundled driver's routing would.
///
/// Best-effort throughout. A teardown that fails must not block the update — a
/// stale tap is recoverable, a half-installed app is not.
fn stage_teardown<R: Runtime>(app: &AppHandle<R>) {
    if let Some(engine) = app.try_state::<hm_audio::AudioEngine>() {
        // Mirrors `stop_system_audio`: Linux/Windows own a self-contained EQ
        // session to drop, while macOS's routing lives in the playing source, so
        // stopping the engine is what releases the tap and its aggregate device.
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        engine.stop_system_eq();
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        engine.stop();
    }
    // Known gap, stated rather than hidden: the settings autosave runs on a 2s
    // debounce and there is no flush hook, so up to two seconds of very recent
    // changes are still only in memory here and are lost. Everything older is
    // already on disk. Not worth a plumbing change to close.
}

/// Install the staged update, if there is one. Returns `Ok(false)` when there
/// is nothing staged, so the quit path can carry on without special-casing.
fn install_staged<R: Runtime>(app: &AppHandle<R>) -> Result<bool, String> {
    let state = app.state::<UpdaterState>();
    let staged = match state.staged.lock() {
        Ok(mut slot) => slot.take(),
        Err(_) => None,
    };
    let Some(Staged { update, bytes }) = staged else {
        return Ok(false);
    };

    log(&format!("updater: installing {}", update.version));
    stage_teardown(app);
    update.install(bytes).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Install a staged update on the way out.
///
/// Called from the run loop's exit path — the ⌘Q / quit route, **not** window
/// close, which only hides this app. Failure is swallowed on purpose: refusing
/// to quit because an update would not install is a worse outcome than quitting
/// on the old version, which still works.
pub fn install_on_exit<R: Runtime>(app: &AppHandle<R>) {
    match install_staged(app) {
        Ok(true) => log("updater: installed on exit"),
        Ok(false) => {}
        Err(e) => log(&format!("updater: install on exit failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The UI renders straight off this, so the tag has to survive serde.
    #[test]
    fn status_serializes_with_a_discriminant_the_ui_can_switch_on() {
        let json = serde_json::to_string(&Status::Idle).unwrap();
        assert_eq!(json, r#"{"state":"idle"}"#);

        let json = serde_json::to_string(&Status::Downloading {
            received: 10,
            total: Some(100),
        })
        .unwrap();
        assert!(json.contains(r#""state":"downloading""#), "{json}");
        assert!(json.contains(r#""received":10"#), "{json}");
    }

    /// A chunked response has no length. The UI must be able to tell "no total"
    /// from "zero", or a progress bar sits at 0% for the whole download.
    #[test]
    fn an_unknown_total_is_null_rather_than_zero() {
        let json = serde_json::to_string(&Status::Downloading { received: 5, total: None }).unwrap();
        assert!(json.contains(r#""total":null"#), "{json}");
    }

    /// "No update available" is the common case and must not read as a failure.
    #[test]
    fn idle_and_failed_are_distinguishable() {
        assert_ne!(Status::Idle, Status::Failed { message: String::new() });
    }
}
