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
use tauri::{AppHandle, State};

use super::install::resolve_install_id;

/// Management API base, overridable at runtime via `HM_MANAGEMENT_API`
/// (e.g. `http://localhost:4000/api` for local dev).
const DEFAULT_API: &str = "http://37.60.225.220:9400/api";

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    access: Option<String>,
    refresh: Option<String>,
    email: Option<String>,
    name: Option<String>,
    /// Whether `email` is a real, claimed address. `false` for the synthetic
    /// `anon+<installId>@anon.hypemuzik.app` the anonymous bootstrap mints —
    /// that's how the UI tells "frictionless free session" from "signed in".
    #[serde(default)]
    email_verified: bool,
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
    /// `false` for a frictionless anonymous session (synthetic email, nothing
    /// claimed yet) — the Settings view uses this to offer "attach email"
    /// instead of showing account details.
    pub email_verified: bool,
    /// The admin account: exempt from purchases, always full premium access.
    /// The UI hides all Supporter upsell for it. Sourced live from `GET /me`.
    pub is_admin: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserDto {
    email: String,
    name: Option<String>,
    #[serde(default)]
    email_verified: bool,
}

/// `GET /me/entitlements` response — the one thing the premium gate consumes.
#[derive(Deserialize)]
struct EntitlementsDto {
    premium: bool,
}

/// `GET /me` response — we only read the role, to tell the **admin** account
/// apart. The admin is exempt from purchases and always has full premium
/// access, so the UI hides every upsell for it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MeDto {
    #[serde(default)]
    role: Option<String>,
}

impl MeDto {
    fn is_admin(&self) -> bool {
        self.role.as_deref() == Some("ADMIN")
    }
}

/// `POST /support/checkout` response.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckoutDto {
    checkout_url: String,
}

/// `POST /support/restore` response.
#[derive(Deserialize)]
struct RestoreDto {
    restored: bool,
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
        s.email_verified = dto.user.email_verified;
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

    /// Authenticated POST (Bearer), retrying once after a token refresh on 401 —
    /// the `auth_get` pattern, for the claim endpoints. `None` means there's no
    /// session to authenticate with, or the request couldn't reach the server.
    fn auth_post(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Option<reqwest::blocking::Response> {
        let access = self.inner.lock().expect("account poisoned").access.clone()?;
        let resp = self
            .client
            .post(self.url(path))
            .bearer_auth(&access)
            .json(body)
            .send()
            .ok()?;
        if resp.status().as_u16() == 401 && self.refresh_after_401(&access) {
            let access = self.inner.lock().expect("account poisoned").access.clone()?;
            return self
                .client
                .post(self.url(path))
                .bearer_auth(&access)
                .json(body)
                .send()
                .ok();
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

    /// Silently provision a session for a brand-new user: no email, no
    /// password, keyed off the keychain-persisted install id. Idempotent —
    /// the server links the same `Install` to the same `User` every time, so
    /// calling this again (e.g. next launch, if the local session was lost)
    /// just re-issues tokens for the account already tied to this install.
    pub fn bootstrap_anonymous(
        &self,
        install_id: &str,
        os: &str,
        arch: &str,
        app_version: &str,
    ) -> Result<(), String> {
        let resp = self.post(
            "/auth/anonymous",
            serde_json::json!({
                "installId": install_id,
                "os": os,
                "arch": arch,
                "appVersion": app_version,
            }),
        )?;
        match resp.status().as_u16() {
            200 | 201 => {}
            code => return Err(format!("Couldn't start a session ({code})")),
        }
        let dto: AuthDto = resp.json().map_err(|e| e.to_string())?;
        self.store_session(dto);
        Ok(())
    }

    /// Request a code to attach a real email to the current (possibly
    /// anonymous) account. Does not change the account until `claim_verify`.
    pub fn claim_request(&self, email: &str) -> Result<(), String> {
        let resp = self
            .auth_post("/auth/claim/request", &serde_json::json!({ "email": email }))
            .ok_or_else(|| "couldn't reach the server".to_string())?;
        match resp.status().as_u16() {
            200 | 201 => Ok(()),
            409 => Err("That email already has an account — sign in instead".into()),
            401 => Err("Your session expired — restart the app and try again".into()),
            code => Err(format!("Couldn't send a code ({code})")),
        }
    }

    /// Verify the emailed code and attach the email to the current account in
    /// place — same user id, same history/entitlement, now with a real,
    /// verified email instead of the anonymous placeholder.
    pub fn claim_verify(&self, email: &str, code: &str, name: Option<&str>) -> Result<(), String> {
        let resp = self
            .auth_post(
                "/auth/claim/verify",
                &serde_json::json!({ "email": email, "code": code, "name": name }),
            )
            .ok_or_else(|| "couldn't reach the server".to_string())?;
        match resp.status().as_u16() {
            200 | 201 => {}
            401 => return Err("Invalid or expired code".into()),
            409 => return Err("That email already has an account — sign in instead".into()),
            code => return Err(format!("Verification failed ({code})")),
        }
        let dto: AuthDto = resp.json().map_err(|e| e.to_string())?;
        self.store_session(dto);
        Ok(())
    }

    /// Whether the signed-in account is entitled to Supporter/premium
    /// features. `Err` means the check couldn't be completed (offline,
    /// unauthenticated, or a server error) — callers should fall back to a
    /// cached last-known value rather than treat that as a hard "no", since a
    /// blip must never downgrade a supporter mid-session.
    pub fn entitlements(&self) -> Result<bool, String> {
        let resp = self
            .auth_get("/me/entitlements")
            .ok_or_else(|| "couldn't reach the server".to_string())?;
        if !resp.status().is_success() {
            return Err(format!("Entitlement check failed ({})", resp.status().as_u16()));
        }
        let dto: EntitlementsDto = resp.json().map_err(|e| e.to_string())?;
        Ok(dto.premium)
    }

    /// Start a Paddle "Supporter" checkout for this install. Public — no
    /// session required, so an anonymous free-tier donor never has to sign in
    /// first; the amount alone is what a supporter is paying for. `amount` is
    /// whole/fractional dollars (e.g. `10.0` for $10); `currency` is omitted
    /// from the request entirely when `None` rather than sent as JSON `null`,
    /// so the server's own default applies.
    pub fn support_checkout(
        &self,
        install_id: &str,
        amount: f64,
        currency: Option<&str>,
    ) -> Result<String, String> {
        let mut body = serde_json::json!({ "installId": install_id, "amount": amount });
        if let Some(currency) = currency {
            body["currency"] = serde_json::Value::String(currency.to_string());
        }
        let resp = self.post("/support/checkout", body)?;
        if !resp.status().is_success() {
            return Err(format!("Couldn't start checkout ({})", resp.status().as_u16()));
        }
        let dto: CheckoutDto = resp.json().map_err(|e| e.to_string())?;
        Ok(dto.checkout_url)
    }

    /// Restore a Supporter entitlement onto this install from a past
    /// checkout's receipt email (reinstall, or a second machine). Public —
    /// no session required, matching `support_checkout`.
    pub fn support_restore(&self, install_id: &str, email: &str) -> Result<bool, String> {
        let resp = self.post(
            "/support/restore",
            serde_json::json!({ "installId": install_id, "email": email }),
        )?;
        if !resp.status().is_success() {
            return Err(format!("Restore failed ({})", resp.status().as_u16()));
        }
        let dto: RestoreDto = resp.json().map_err(|e| e.to_string())?;
        Ok(dto.restored)
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
        let (email, name, has_access, email_verified) = {
            let s = self.inner.lock().expect("account poisoned");
            (s.email.clone(), s.name.clone(), s.access.is_some(), s.email_verified)
        };
        if !has_access {
            return AccountStatus {
                authenticated: false,
                email: None,
                name: None,
                license: None,
                email_verified: false,
                is_admin: false,
            };
        }
        let license = self
            .auth_get("/me/license")
            .filter(|r| r.status().is_success())
            .and_then(|r| r.json::<LicenseInfo>().ok());
        // Live role, so promoting an account to admin takes effect on the next
        // status refresh (no re-sign-in). A failed fetch just leaves it non-admin.
        let is_admin = self
            .auth_get("/me")
            .filter(|r| r.status().is_success())
            .and_then(|r| r.json::<MeDto>().ok())
            .is_some_and(|me| me.is_admin());
        // refresh() may have cleared a dead session while fetching.
        let authenticated = self.inner.lock().expect("account poisoned").access.is_some();
        AccountStatus {
            authenticated,
            email: if authenticated { email } else { None },
            name: if authenticated { name } else { None },
            license,
            email_verified: authenticated && email_verified,
            is_admin: authenticated && is_admin,
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

/// Silently provision the free-tier session on first launch (or whenever no
/// local session exists) — no email, no password. Uses the SAME
/// keychain-persisted install id as `install::spawn_tracking`, so the
/// anonymous account and the accountless install ping are always the same
/// identity; this must never mint a second id.
#[tauri::command(async)]
pub fn account_bootstrap_anonymous(
    app: AppHandle,
    account: State<'_, AccountState>,
) -> Result<AccountStatus, IpcError> {
    let install_id = resolve_install_id(&app);
    let version = app.package_info().version.to_string();
    account
        .bootstrap_anonymous(&install_id, std::env::consts::OS, std::env::consts::ARCH, &version)
        .map_err(|e| IpcError::new("account", e))?;
    Ok(account.status())
}

/// Start attaching a real email to the current account (Settings → "Sign in /
/// attach email"). Optional — never a gate.
#[tauri::command(async)]
pub fn account_claim_request(
    account: State<'_, AccountState>,
    email: String,
) -> Result<(), IpcError> {
    account
        .claim_request(&email)
        .map_err(|e| IpcError::new("account", e))
}

/// Verify the emailed code and attach it to the current account in place.
#[tauri::command(async)]
pub fn account_claim_verify(
    account: State<'_, AccountState>,
    email: String,
    code: String,
    name: Option<String>,
) -> Result<AccountStatus, IpcError> {
    account
        .claim_verify(&email, &code, name.as_deref())
        .map_err(|e| IpcError::new("account", e))?;
    Ok(account.status())
}

/// Whether the account is entitled to Supporter/premium features. Rejects
/// (rather than returning `false`) when the check couldn't be completed, so
/// the front-end's entitlement store can fall back to its cached last-known
/// value instead of wrongly downgrading a supporter who is offline.
#[tauri::command(async)]
pub fn account_entitlements(account: State<'_, AccountState>) -> Result<bool, IpcError> {
    account
        .entitlements()
        .map_err(|e| IpcError::new("account", e))
}

/// Start a Paddle "Supporter" checkout for this install and return the
/// checkout URL. Resolves the install id the same way
/// `account_bootstrap_anonymous` does — the checkout and the install/account
/// this app already tracks must always be the same identity. The front end
/// opens the returned URL itself (see `open_external_url`) once it has it,
/// rather than this command doing the fetch and the browser-open as one
/// blocking step.
#[tauri::command(async)]
pub fn account_support_checkout(
    app: AppHandle,
    account: State<'_, AccountState>,
    amount: f64,
    currency: Option<String>,
) -> Result<String, IpcError> {
    let install_id = resolve_install_id(&app);
    account
        .support_checkout(&install_id, amount, currency.as_deref())
        .map_err(|e| IpcError::new("account", e))
}

/// Restore a Supporter entitlement onto this install from a past checkout's
/// receipt email (e.g. after a reinstall, or supporting from a second
/// machine). The caller should re-fetch `account_entitlements` afterwards —
/// this only reports whether the server found and applied a match.
#[tauri::command(async)]
pub fn account_support_restore(
    app: AppHandle,
    account: State<'_, AccountState>,
    email: String,
) -> Result<bool, IpcError> {
    let install_id = resolve_install_id(&app);
    account
        .support_restore(&install_id, &email)
        .map_err(|e| IpcError::new("account", e))
}

/// Open a URL in the OS's default browser. Exposed so the front end can open
/// a URL it already holds (the Supporter checkout link) — reuses the exact
/// per-OS mechanism the cloud OAuth flow uses (`crate::cloud::open_url`)
/// rather than embedding a new way to leave the webview.
#[tauri::command(async)]
pub fn open_external_url(url: String) {
    crate::cloud::open_url(&url);
}

#[cfg(test)]
mod tests {
    use super::MeDto;

    /// The admin account (role `"ADMIN"`) is the only one flagged admin; every
    /// other role — and a `/me` payload that omits the field entirely — is not.
    #[test]
    fn me_dto_admin_derivation() {
        let admin: MeDto = serde_json::from_str(r#"{"role":"ADMIN"}"#).unwrap();
        assert!(admin.is_admin(), "ADMIN role must be admin");

        let user: MeDto = serde_json::from_str(r#"{"role":"USER"}"#).unwrap();
        assert!(!user.is_admin(), "USER role must not be admin");

        // `/me` may carry unrelated fields; only `role` matters, and a missing
        // role defaults to non-admin (safe: never grants exemption by accident).
        let missing: MeDto = serde_json::from_str(r#"{"email":"x@y.z"}"#).unwrap();
        assert!(!missing.is_admin(), "absent role must not be admin");
    }
}
