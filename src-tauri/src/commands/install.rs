//! Anonymous install identity + heartbeat — accountless user tracking.
//!
//! On first launch we mint a random UUID, persist it in **both** the app-data
//! dir (fast read) and the **OS keychain** (survives a reinstall, so a
//! supporter's status isn't lost), and register it with the Management API — so
//! first-time users are counted with **zero signup**. Then we heartbeat while
//! running. No account, no PII: just a random id + os/version/coarse country
//! (the server derives country from the request and discards the IP).

use std::path::PathBuf;
use std::time::Duration;

use tauri::{AppHandle, Manager};

/// Keychain entry — DELIBERATELY isolated from every other key in this app. The
/// YT cookie jar has a history of a test wiping real credentials; this id must
/// never be touched by tests, so it uses its own service name.
const KC_SERVICE: &str = "com.hypemuzik.desktop.install";
const KC_ACCOUNT: &str = "install-id";
const FILE_NAME: &str = "install-id";

/// Management API base (same env override as `account.rs`).
const DEFAULT_API: &str = "http://37.60.225.220:9400/api";
/// How often to heartbeat while running (the server rolls these up per-day).
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

fn api_base() -> String {
    std::env::var("HM_MANAGEMENT_API").unwrap_or_else(|_| DEFAULT_API.to_string())
}

fn is_uuid(s: &str) -> bool {
    uuid::Uuid::parse_str(s.trim()).is_ok()
}

fn id_file(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join(FILE_NAME))
}

fn read_file_id(app: &AppHandle) -> Option<String> {
    let p = id_file(app)?;
    let s = std::fs::read_to_string(&p).ok()?;
    let s = s.trim();
    is_uuid(s).then(|| s.to_string())
}

fn write_file_id(app: &AppHandle, id: &str) {
    if let Some(p) = id_file(app) {
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&p, id);
    }
}

fn keychain_id() -> Option<String> {
    let entry = keyring::Entry::new(KC_SERVICE, KC_ACCOUNT).ok()?;
    match entry.get_password() {
        Ok(v) if is_uuid(&v) => Some(v.trim().to_string()),
        _ => None,
    }
}

fn write_keychain_id(id: &str) {
    if let Ok(entry) = keyring::Entry::new(KC_SERVICE, KC_ACCOUNT) {
        let _ = entry.set_password(id);
    }
}

/// Resolve the persistent install id: app-data → keychain (reinstall) → mint.
/// Always writes the id back to both stores so it survives a reinstall.
fn resolve_install_id(app: &AppHandle) -> String {
    if let Some(id) = read_file_id(app) {
        // Keep the keychain copy in sync (belt-and-suspenders) without a read cost.
        return id;
    }
    if let Some(id) = keychain_id() {
        // Reinstall: the app-data file was cleared but the keychain kept the id.
        write_file_id(app, &id);
        return id;
    }
    let id = uuid::Uuid::new_v4().to_string();
    write_file_id(app, &id);
    write_keychain_id(&id);
    id
}

/// Register the install then heartbeat for the app's lifetime. Fire-and-forget:
/// tracking must never affect the app if the server is unreachable.
pub fn spawn_tracking(app: AppHandle) {
    std::thread::Builder::new()
        .name("hm-install-tracking".into())
        .spawn(move || {
            let id = resolve_install_id(&app);
            let base = api_base();
            let version = app.package_info().version.to_string();
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());

            // Register (upsert) — records os/arch/version, refreshes last-seen.
            let _ = client
                .post(format!("{base}/installs"))
                .json(&serde_json::json!({
                    "installId": id,
                    "os": std::env::consts::OS,
                    "arch": std::env::consts::ARCH,
                    "appVersion": version,
                }))
                .send();

            // Heartbeat on launch, then on a slow cadence.
            let ping_url = format!("{base}/installs/{id}/ping");
            loop {
                let _ = client
                    .post(&ping_url)
                    .json(&serde_json::json!({ "appVersion": version }))
                    .send();
                std::thread::sleep(HEARTBEAT_INTERVAL);
            }
        })
        .ok();
}
