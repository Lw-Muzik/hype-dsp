//! Account + licensing against the HypeMuzik **Management API**. This replaces
//! the local license mock for gating: the user signs in, and feature access is
//! decided by the trial / license the server reports (`GET /me/license`).
//!
//! Tokens persist in `account.json` in the app data dir. Network calls use
//! `reqwest::blocking` on the async command pool (the established pattern here)
//! and transparently refresh the access token once on a 401.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use hm_core::IpcError;
use serde::{Deserialize, Serialize};
use tauri::State;

/// Management API base, overridable at runtime via `HM_MANAGEMENT_API`
/// (e.g. `http://localhost:4000/api` for local dev).
const DEFAULT_API: &str = "http://37.60.225.220:9400/api";

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    access: Option<String>,
    refresh: Option<String>,
    email: Option<String>,
    name: Option<String>,
}

/// The server's license verdict — `allowed` is the only thing the UI gates on.
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LicenseInfo {
    pub state: String,
    pub allowed: bool,
    pub days_left: u32,
    pub trial_ends_at: Option<String>,
    pub licensed_until: Option<String>,
}

/// What the front-end auth gate consumes.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStatus {
    pub authenticated: bool,
    pub email: Option<String>,
    pub name: Option<String>,
    pub license: Option<LicenseInfo>,
}

#[derive(Deserialize)]
struct UserDto {
    email: String,
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthDto {
    user: UserDto,
    access_token: String,
    refresh_token: String,
}

pub struct AccountState {
    inner: Mutex<Stored>,
    /// Serializes token refreshes. Refresh tokens are **single-use** (the server
    /// rotates + revokes the old one), so two concurrent 401s must not both spend
    /// the same token — the loser would present a revoked token, get a 401, and
    /// wrongly clear the session. Held across the refresh round-trip; `inner` is
    /// only locked briefly inside, so there's no lock-ordering hazard.
    refresh_lock: Mutex<()>,
    path: PathBuf,
    api: String,
    client: reqwest::blocking::Client,
}

impl AccountState {
    pub fn open(path: PathBuf) -> Self {
        let stored = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let api = std::env::var("HM_MANAGEMENT_API").unwrap_or_else(|_| DEFAULT_API.to_string());
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        Self {
            inner: Mutex::new(stored),
            refresh_lock: Mutex::new(()),
            path,
            api,
            client,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.api, path)
    }

    fn save(&self, s: &Stored) {
        if let Some(p) = self.path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        if let Ok(json) = serde_json::to_string_pretty(s) {
            let _ = std::fs::write(&self.path, json);
        }
    }

    fn store_session(&self, dto: AuthDto) {
        let mut s = self.inner.lock().expect("account poisoned");
        s.access = Some(dto.access_token);
        s.refresh = Some(dto.refresh_token);
        s.email = Some(dto.user.email);
        s.name = dto.user.name;
        self.save(&s);
    }

    fn clear(&self) {
        let mut s = self.inner.lock().expect("account poisoned");
        *s = Stored::default();
        self.save(&s);
    }

    fn post(&self, path: &str, body: serde_json::Value) -> Result<reqwest::blocking::Response, String> {
        self.client
            .post(self.url(path))
            .json(&body)
            .send()
            .map_err(|e| format!("couldn't reach the server: {e}"))
    }

    /// Refresh the access token after a request failed with 401 on
    /// `stale_access`. **Single-flight**: only one refresh runs at a time, and a
    /// caller that finds the token already rotated (by whoever held the lock
    /// first) skips its own refresh and uses the new token. This is what stops
    /// two periodic calls (the 5-min status check and the 60-s heartbeat) from
    /// both spending the single-use refresh token when the access token expires
    /// and revoking each other's session.
    ///
    /// The session is cleared **only** on a definitive rejection (the refresh
    /// token itself is invalid/expired → 401/403). A transient failure (server
    /// 5xx, rate-limit, offline) leaves the session intact so a later call can
    /// retry — a blip must never sign the user out.
    fn refresh_after_401(&self, stale_access: &str) -> bool {
        let _flight = self.refresh_lock.lock().unwrap_or_else(|e| e.into_inner());
        // Someone may have refreshed while we waited for the lock: if the access
        // token is no longer the stale one we 401'd on, it's already good.
        {
            let s = self.inner.lock().expect("account poisoned");
            match s.access.as_deref() {
                Some(cur) if cur != stale_access => return true, // already refreshed
                None => return false,                            // session cleared
                _ => {}
            }
        }
        let refresh = self.inner.lock().expect("account poisoned").refresh.clone();
        let Some(refresh) = refresh else {
            return false;
        };
        let resp = self
            .client
            .post(self.url("/auth/refresh"))
            .json(&serde_json::json!({ "refreshToken": refresh }))
            .send();
        match resp {
            Ok(r) if r.status().is_success() => match r.json::<AuthDto>() {
                Ok(dto) => {
                    self.store_session(dto);
                    true
                }
                // Parsed body was bad — treat as transient, keep the session.
                Err(_) => false,
            },
            // The refresh token itself was rejected → the session is genuinely
            // dead (expired, revoked, or signed out elsewhere). Only now clear.
            Ok(r) if matches!(r.status().as_u16(), 401 | 403) => {
                self.clear();
                false
            }
            // Any other response (5xx, 429, …) or a transport error is transient:
            // keep the session and let a later call retry instead of signing out.
            Ok(_) | Err(_) => false,
        }
    }

    /// Authenticated GET, retrying once after a token refresh on 401.
    fn auth_get(&self, path: &str) -> Option<reqwest::blocking::Response> {
        let access = self.inner.lock().expect("account poisoned").access.clone()?;
        let resp = self.client.get(self.url(path)).bearer_auth(&access).send().ok()?;
        if resp.status().as_u16() == 401 && self.refresh_after_401(&access) {
            let access = self.inner.lock().expect("account poisoned").access.clone()?;
            return self.client.get(self.url(path)).bearer_auth(&access).send().ok();
        }
        Some(resp)
    }

    /// Create a passwordless account (with details) — the server emails a code.
    pub fn signup(&self, email: &str, name: Option<&str>) -> Result<(), String> {
        let resp = self.post("/auth/signup", serde_json::json!({ "email": email, "name": name }))?;
        match resp.status().as_u16() {
            200 | 201 => Ok(()),
            409 => Err("That email is already registered — sign in instead".into()),
            code => Err(format!("Sign up failed ({code})")),
        }
    }

    /// Request a sign-in code for an existing account.
    pub fn request_otp(&self, email: &str) -> Result<(), String> {
        let resp = self.post("/auth/request-otp", serde_json::json!({ "email": email }))?;
        match resp.status().as_u16() {
            200 | 201 => Ok(()),
            404 => Err("No account for that email — sign up first".into()),
            code => Err(format!("Couldn't send a code ({code})")),
        }
    }

    /// Verify an emailed code and start a session.
    pub fn verify(&self, email: &str, code: &str) -> Result<(), String> {
        let resp = self.post(
            "/auth/verify-otp",
            serde_json::json!({ "email": email, "code": code }),
        )?;
        match resp.status().as_u16() {
            200 | 201 => {}
            401 => return Err("Invalid or expired code".into()),
            code => return Err(format!("Verification failed ({code})")),
        }
        let dto: AuthDto = resp.json().map_err(|e| e.to_string())?;
        self.store_session(dto);
        Ok(())
    }

    pub fn logout(&self) {
        let refresh = self.inner.lock().expect("account poisoned").refresh.clone();
        if let Some(refresh) = refresh {
            let _ = self
                .client
                .post(self.url("/auth/logout"))
                .json(&serde_json::json!({ "refreshToken": refresh }))
                .send();
        }
        self.clear();
    }

    pub fn status(&self) -> AccountStatus {
        let (email, name, has_access) = {
            let s = self.inner.lock().expect("account poisoned");
            (s.email.clone(), s.name.clone(), s.access.is_some())
        };
        if !has_access {
            return AccountStatus {
                authenticated: false,
                email: None,
                name: None,
                license: None,
            };
        }
        let license = self
            .auth_get("/me/license")
            .filter(|r| r.status().is_success())
            .and_then(|r| r.json::<LicenseInfo>().ok());
        // refresh() may have cleared a dead session while fetching.
        let authenticated = self.inner.lock().expect("account poisoned").access.is_some();
        AccountStatus {
            authenticated,
            email: if authenticated { email } else { None },
            name: if authenticated { name } else { None },
            license,
        }
    }

    pub fn heartbeat(&self, platform: &str, app_version: &str) {
        let body = serde_json::json!({ "platform": platform, "appVersion": app_version });
        let post = |access: &str| {
            self.client
                .post(self.url("/usage/heartbeat"))
                .bearer_auth(access)
                .json(&body)
                .send()
        };
        let Some(access) = self.inner.lock().expect("account poisoned").access.clone() else {
            return;
        };
        let resp = post(&access);
        // Refresh + retry once if the access token had expired (single-flight, so
        // it won't race the status check into revoking the session).
        if matches!(&resp, Ok(r) if r.status().as_u16() == 401) && self.refresh_after_401(&access) {
            if let Some(access) = self.inner.lock().expect("account poisoned").access.clone() {
                let _ = post(&access);
            }
        }
    }
}

// ----------------------------------------------------------------- commands

#[tauri::command(async)]
pub fn account_status(account: State<'_, AccountState>) -> AccountStatus {
    account.status()
}

#[tauri::command(async)]
pub fn account_signup(
    account: State<'_, AccountState>,
    email: String,
    name: Option<String>,
) -> Result<(), IpcError> {
    account
        .signup(&email, name.as_deref())
        .map_err(|e| IpcError::new("account", e))
}

#[tauri::command(async)]
pub fn account_request_otp(
    account: State<'_, AccountState>,
    email: String,
) -> Result<(), IpcError> {
    account
        .request_otp(&email)
        .map_err(|e| IpcError::new("account", e))
}

#[tauri::command(async)]
pub fn account_verify(
    account: State<'_, AccountState>,
    email: String,
    code: String,
) -> Result<AccountStatus, IpcError> {
    account
        .verify(&email, &code)
        .map_err(|e| IpcError::new("account", e))?;
    Ok(account.status())
}

#[tauri::command(async)]
pub fn account_logout(account: State<'_, AccountState>) {
    account.logout();
}

#[tauri::command(async)]
pub fn account_heartbeat(
    account: State<'_, AccountState>,
    platform: String,
    app_version: String,
) {
    account.heartbeat(&platform, &app_version);
}
