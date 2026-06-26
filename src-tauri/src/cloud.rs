//! Cloud music: Google Drive + Dropbox, ported from the Hype mobile app.
//!
//! Desktop OAuth uses a **loopback redirect + PKCE** (no client secret needed
//! for Dropbox; Google "Desktop app" clients use an embedded, non-confidential
//! secret). We open the system browser to the consent page, catch the redirect
//! on a local one-shot HTTP listener, exchange the code for tokens, and persist
//! them. Listing + streaming hit the providers' REST APIs with the access token.
//!
//! Credentials are read from env vars (see `docs/cloud-setup.md`); the redirect
//! is a fixed `http://localhost:53682` so it can be registered in the consoles.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ------------------------------------------------------------------- config

const REDIRECT_URI: &str = "http://localhost:53682";
const LOOPBACK_ADDR: &str = "127.0.0.1:53682";
const DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive.readonly";

// Bundled OAuth defaults so the desktop connects with no env vars (env vars
// still override). The desktop uses a loopback + PKCE flow:
//   * Google: a dedicated **Desktop app** OAuth client (in Google Cloud project
//     618382337035). Desktop clients accept the loopback redirect automatically
//     and their "client secret" is non-confidential by Google's design, so it's
//     embedded here like the other identifiers. Access is gated by the consent
//     screen's test-user list until the app is verified.
//   * Dropbox: works with just the app key, but `http://localhost:53682` must be
//     added to this app's Redirect URIs in the Dropbox App Console (one-time).
// See docs/cloud-setup.md.
const DEFAULT_GOOGLE_CLIENT_ID: &str =
    "618382337035-kuak9rr26kk7r62g3ac5hte0eei1l36d.apps.googleusercontent.com";
const DEFAULT_GOOGLE_CLIENT_SECRET: &str = "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";
const DEFAULT_DROPBOX_APP_KEY: &str = "1d0mou7l0x19mas";

fn env_or(name: &str, default: &str) -> String {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => default.to_string(),
    }
}
fn google_client_id() -> String {
    env_or("HM_GDRIVE_CLIENT_ID", DEFAULT_GOOGLE_CLIENT_ID)
}
fn google_client_secret() -> String {
    env_or("HM_GDRIVE_CLIENT_SECRET", DEFAULT_GOOGLE_CLIENT_SECRET)
}
fn dropbox_app_key() -> String {
    env_or("HM_DROPBOX_APP_KEY", DEFAULT_DROPBOX_APP_KEY)
}

// -------------------------------------------------------------------- types

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CloudProvider {
    GoogleDrive,
    Dropbox,
}

/// An entry in a cloud folder — a subfolder or an audio file (mirrors the
/// front-end type). Browsed folder-by-folder, like a normal file browser.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudEntry {
    pub provider: CloudProvider,
    /// Which connected account this entry belongs to (its stable id). Stamped by
    /// the high-level `list`/`all_audio`; streaming/metadata use it to pick the
    /// right account's tokens (two Google accounts share `provider`).
    #[serde(default)]
    pub account_id: String,
    /// Folder/file handle to navigate or stream. Drive: object id (folders too).
    /// Dropbox: lowercased path.
    pub id: String,
    pub name: String,
    pub is_folder: bool,
    pub size: u64,
    /// Parent folder name, for the flat account-wide listing's grouping label
    /// (`None` for folder-by-folder browse entries).
    #[serde(default)]
    pub folder: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Tokens {
    access: String,
    refresh: Option<String>,
    /// Unix seconds at which `access` expires.
    expires_at: Option<u64>,
}

impl Tokens {
    fn near_expiry(&self) -> bool {
        match self.expires_at {
            Some(exp) => now_secs() + 60 >= exp,
            None => false,
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// One connected cloud account: a stable id, its provider, a human label (the
/// account's email / display name), and its OAuth tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredAccount {
    id: String,
    provider: CloudProvider,
    label: String,
    tokens: Tokens,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    accounts: Vec<StoredAccount>,
    // Legacy single-account fields (pre-multi-account builds). Read once for
    // migration into `accounts`, then dropped (never written again).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    google: Option<Tokens>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dropbox: Option<Tokens>,
}

impl Store {
    /// Fold any legacy single-account tokens into `accounts`. The migrated id is
    /// the provider's *old* cache key (e.g. `"GoogleDrive"`), so the existing
    /// on-disk listing + metadata caches stay valid; the label stays generic
    /// until the account is reconnected (which resolves the real email).
    fn migrate_legacy(&mut self) {
        let legacy = [
            (CloudProvider::GoogleDrive, self.google.take(), "Google Drive"),
            (CloudProvider::Dropbox, self.dropbox.take(), "Dropbox"),
        ];
        for (provider, tokens, label) in legacy {
            if let Some(tokens) = tokens {
                let id = format!("{provider:?}");
                if !self.accounts.iter().any(|a| a.id == id) {
                    self.accounts.push(StoredAccount {
                        id,
                        provider,
                        label: label.to_string(),
                        tokens,
                    });
                }
            }
        }
    }
}

/// One connected account as surfaced to the UI (no tokens).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudAccount {
    pub id: String,
    pub provider: CloudProvider,
    pub label: String,
}

/// Connection status surfaced to the UI: the connected accounts (any number per
/// provider) plus whether each provider has credentials configured.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudStatus {
    pub accounts: Vec<CloudAccount>,
    pub google_configured: bool,
    pub dropbox_configured: bool,
}

/// Managed Tauri state: connected cloud accounts + their on-disk path.
pub struct CloudState {
    inner: Mutex<Store>,
    path: PathBuf,
}

impl CloudState {
    pub fn load(path: PathBuf) -> Self {
        let mut store = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<Store>(&t).ok())
            .unwrap_or_default();
        store.migrate_legacy();
        Self {
            inner: Mutex::new(store),
            path,
        }
    }

    fn save(&self, store: &Store) {
        if let Ok(json) = serde_json::to_string_pretty(store) {
            let tmp = self.path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }
    }

    /// Every connected account (no tokens) — for the UI account list.
    pub fn accounts(&self) -> Vec<CloudAccount> {
        let s = self.inner.lock().expect("cloud poisoned");
        s.accounts
            .iter()
            .map(|a| CloudAccount {
                id: a.id.clone(),
                provider: a.provider,
                label: a.label.clone(),
            })
            .collect()
    }

    pub fn status(&self) -> CloudStatus {
        CloudStatus {
            accounts: self.accounts(),
            // Google's desktop flow also needs a client secret (the mobile app
            // doesn't have one), so require it before offering "Connect".
            google_configured: !google_client_id().is_empty()
                && !google_client_secret().is_empty(),
            dropbox_configured: !dropbox_app_key().is_empty(),
        }
    }

    /// Forget one account's tokens (by id).
    pub fn disconnect(&self, account_id: &str) {
        let mut s = self.inner.lock().expect("cloud poisoned");
        s.accounts.retain(|a| a.id != account_id);
        self.save(&s);
    }

    fn provider_of(&self, account_id: &str) -> Option<CloudProvider> {
        let s = self.inner.lock().expect("cloud poisoned");
        s.accounts
            .iter()
            .find(|a| a.id == account_id)
            .map(|a| a.provider)
    }

    fn tokens(&self, account_id: &str) -> Option<(CloudProvider, Tokens)> {
        let s = self.inner.lock().expect("cloud poisoned");
        s.accounts
            .iter()
            .find(|a| a.id == account_id)
            .map(|a| (a.provider, a.tokens.clone()))
    }

    fn set_tokens(&self, account_id: &str, tokens: Tokens) {
        let mut s = self.inner.lock().expect("cloud poisoned");
        if let Some(a) = s.accounts.iter_mut().find(|a| a.id == account_id) {
            a.tokens = tokens;
        }
        self.save(&s);
    }

    /// Insert a new account, or replace an existing one's tokens + label
    /// (re-authing the same account keeps its id, so caches stay valid).
    fn upsert(&self, account: StoredAccount) {
        let mut s = self.inner.lock().expect("cloud poisoned");
        if let Some(a) = s.accounts.iter_mut().find(|a| a.id == account.id) {
            a.tokens = account.tokens;
            a.label = account.label;
        } else {
            s.accounts.push(account);
        }
        self.save(&s);
    }

    /// A valid access token for one account, refreshing first if near expiry.
    fn access_token(&self, account_id: &str) -> Result<String, String> {
        let (provider, mut tk) = self
            .tokens(account_id)
            .ok_or_else(|| "not connected".to_string())?;
        if tk.near_expiry() {
            if let Some(refresh) = tk.refresh.clone() {
                tk = refresh_tokens(provider, &refresh)?;
                self.set_tokens(account_id, tk.clone());
            }
        }
        Ok(tk.access)
    }

    // ----- the high-level operations used by the commands -----

    /// Run the interactive OAuth flow for `provider`, resolve the signed-in
    /// account's identity (so two accounts of the same provider stay distinct),
    /// and store it. Re-authing an already-connected account updates it in place.
    pub fn connect(&self, provider: CloudProvider) -> Result<CloudAccount, String> {
        let tokens = oauth_connect(provider)?;
        let (remote_id, label) = fetch_identity(provider, &tokens.access)?;
        let id = format!("{}:{}", provider_key(provider), remote_id);
        self.upsert(StoredAccount {
            id: id.clone(),
            provider,
            label: label.clone(),
            tokens,
        });
        Ok(CloudAccount {
            id,
            provider,
            label,
        })
    }

    /// List the contents of one folder (subfolders + audio files) of `account`.
    /// `folder` is the provider handle, or "" for the account root.
    pub fn list(&self, account_id: &str, folder: &str) -> Result<Vec<CloudEntry>, String> {
        let provider = self
            .provider_of(account_id)
            .ok_or_else(|| "not connected".to_string())?;
        let access = self.access_token(account_id)?;
        let mut entries = match provider {
            CloudProvider::GoogleDrive => drive_browse(&access, folder)?,
            CloudProvider::Dropbox => dropbox_browse(&access, folder)?,
        };
        for e in &mut entries {
            e.account_id = account_id.to_string();
        }
        // Folders first, then files; alphabetical within each.
        entries.sort_by(|a, b| {
            b.is_folder
                .cmp(&a.is_folder)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(entries)
    }

    /// Every audio file in `account`, flat — mirrors the mobile app, which lists
    /// all audio account-wide rather than folder-by-folder, so the Player sees
    /// songs nested in subfolders too. Each entry carries its parent folder's
    /// name as a grouping label, and the owning account's id.
    pub fn all_audio(&self, account_id: &str) -> Result<Vec<CloudEntry>, String> {
        let provider = self
            .provider_of(account_id)
            .ok_or_else(|| "not connected".to_string())?;
        let access = self.access_token(account_id)?;
        let mut entries = match provider {
            CloudProvider::GoogleDrive => drive_all_audio(&access)?,
            CloudProvider::Dropbox => dropbox_all_audio(&access)?,
        };
        for e in &mut entries {
            e.account_id = account_id.to_string();
        }
        entries.sort_by_key(|e| e.name.to_lowercase());
        Ok(entries)
    }

    /// Resolve a streamable `(url, headers)` for a file in `account`.
    pub fn stream_target(
        &self,
        account_id: &str,
        file_id: &str,
    ) -> Result<(String, Vec<(String, String)>), String> {
        let provider = self
            .provider_of(account_id)
            .ok_or_else(|| "not connected".to_string())?;
        let access = self.access_token(account_id)?;
        match provider {
            CloudProvider::GoogleDrive => Ok((
                format!("https://www.googleapis.com/drive/v3/files/{file_id}?alt=media"),
                vec![("Authorization".into(), format!("Bearer {access}"))],
            )),
            CloudProvider::Dropbox => {
                let link = dropbox_temporary_link(&access, file_id)?;
                Ok((link, Vec::new()))
            }
        }
    }
}

/// The provider's stable key for building account ids (matches the front-end
/// `CloudProvider` serialization).
fn provider_key(provider: CloudProvider) -> &'static str {
    match provider {
        CloudProvider::GoogleDrive => "googleDrive",
        CloudProvider::Dropbox => "dropbox",
    }
}

/// Resolve a freshly-connected account's stable remote id + display label, so
/// multiple accounts of the same provider stay distinct. Uses the access we just
/// obtained — Drive's `about` exposes the signed-in user with only the
/// `drive.readonly` scope, so no extra OAuth scope is needed.
fn fetch_identity(provider: CloudProvider, access: &str) -> Result<(String, String), String> {
    let client = http_client()?;
    match provider {
        CloudProvider::GoogleDrive => {
            let resp = client
                .get("https://www.googleapis.com/drive/v3/about?fields=user")
                .bearer_auth(access)
                .send()
                .map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("account lookup failed ({})", resp.status()));
            }
            let data: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
            let email = data["user"]["emailAddress"].as_str().unwrap_or("");
            let name = data["user"]["displayName"].as_str().unwrap_or("");
            // The email is the stable, human-recognizable identity; fall back to
            // the display name only if Drive withholds the address.
            let remote = if !email.is_empty() { email } else { name };
            if remote.is_empty() {
                return Err("no account identity returned".into());
            }
            let label = if !email.is_empty() { email } else { name };
            Ok((remote.to_string(), label.to_string()))
        }
        CloudProvider::Dropbox => {
            // RPC endpoint with no args: send no body and no Content-Type.
            let resp = client
                .post("https://api.dropboxapi.com/2/users/get_current_account")
                .bearer_auth(access)
                .send()
                .map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("account lookup failed ({})", resp.status()));
            }
            let data: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
            let account_id = data["account_id"].as_str().unwrap_or("");
            if account_id.is_empty() {
                return Err("no account identity returned".into());
            }
            let email = data["email"].as_str().unwrap_or("");
            let name = data["name"]["display_name"].as_str().unwrap_or("");
            let label = if !email.is_empty() {
                email
            } else if !name.is_empty() {
                name
            } else {
                account_id
            };
            Ok((account_id.to_string(), label.to_string()))
        }
    }
}

// --------------------------------------------------------------- OAuth (PKCE)

fn random_b64url(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    let _ = getrandom::getrandom(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(target_os = "linux")]
    let _ = Command::new("xdg-open").arg(url).spawn();
}

fn percent_decode(s: &str) -> String {
    let bytes = s.replace('+', " ");
    let bytes = bytes.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encode a string for use in a URL query / form body (RFC 3986
/// unreserved set kept; everything else `%XX`). Used instead of reqwest's
/// `.form()`/`.query()` helpers, which aren't compiled in our minimal build.
fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn form_encode(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", pct_encode(k), pct_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Wait (up to a deadline) for the OAuth redirect on the loopback listener and
/// return the `code`, verifying the CSRF `state`.
fn wait_for_code(listener: &TcpListener, expected_state: &str) -> Result<String, String> {
    listener
        .set_nonblocking(true)
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        if Instant::now() > deadline {
            return Err("authorization timed out".into());
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let line = req.lines().next().unwrap_or("");
                let path = line.split_whitespace().nth(1).unwrap_or("");
                let query = path.split('?').nth(1).unwrap_or("");

                let (mut code, mut state, mut err) = (None, None, None);
                for pair in query.split('&') {
                    let mut kv = pair.splitn(2, '=');
                    let k = kv.next().unwrap_or("");
                    let v = kv.next().unwrap_or("");
                    match k {
                        "code" => code = Some(percent_decode(v)),
                        "state" => state = Some(percent_decode(v)),
                        "error" => err = Some(percent_decode(v)),
                        _ => {}
                    }
                }

                let ok = err.is_none() && code.is_some();
                let body = if ok {
                    "<!doctype html><meta charset=utf-8><body style=\"font-family:-apple-system,\
                     sans-serif;background:#0a0a0c;color:#eceef2;display:grid;place-items:center;\
                     height:100vh;margin:0\"><div><h2>HypeMuzik connected ✓</h2><p>You can close \
                     this tab and return to the app.</p></div>"
                } else {
                    "<!doctype html><meta charset=utf-8><body style=\"font-family:sans-serif;\
                     background:#0a0a0c;color:#eceef2\"><h2>Sign-in failed</h2>"
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: \
                     {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());

                if let Some(e) = err {
                    return Err(format!("authorization denied: {e}"));
                }
                if state.as_deref() != Some(expected_state) {
                    return Err("state mismatch (possible CSRF)".into());
                }
                return code.ok_or_else(|| "no authorization code returned".into());
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(120));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn oauth_connect(provider: CloudProvider) -> Result<Tokens, String> {
    let verifier = random_b64url(64);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_b64url(16);

    let listener = TcpListener::bind(LOOPBACK_ADDR)
        .map_err(|e| format!("could not open loopback port 53682: {e}"))?;

    let auth_url = match provider {
        CloudProvider::GoogleDrive => {
            let cid = google_client_id();
            if cid.is_empty() {
                return Err("Google Drive isn't configured (set HM_GDRIVE_CLIENT_ID — see \
                            docs/cloud-setup.md)"
                    .into());
            }
            format!(
                "https://accounts.google.com/o/oauth2/v2/auth?client_id={cid}\
                 &redirect_uri={REDIRECT_URI}&response_type=code&scope={DRIVE_SCOPE}\
                 &access_type=offline&prompt=consent&code_challenge={challenge}\
                 &code_challenge_method=S256&state={state}"
            )
        }
        CloudProvider::Dropbox => {
            let key = dropbox_app_key();
            if key.is_empty() {
                return Err("Dropbox isn't configured (set HM_DROPBOX_APP_KEY — see \
                            docs/cloud-setup.md)"
                    .into());
            }
            format!(
                "https://www.dropbox.com/oauth2/authorize?client_id={key}\
                 &redirect_uri={REDIRECT_URI}&response_type=code&token_access_type=offline\
                 &code_challenge={challenge}&code_challenge_method=S256&state={state}"
            )
        }
    };

    open_url(&auth_url);
    let code = wait_for_code(&listener, &state)?;
    exchange_code(provider, &code, &verifier)
}

fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

fn token_from_response(r: TokenResponse, prev_refresh: Option<String>) -> Tokens {
    Tokens {
        access: r.access_token,
        refresh: r.refresh_token.or(prev_refresh),
        expires_at: r.expires_in.map(|s| now_secs() + s),
    }
}

fn exchange_code(provider: CloudProvider, code: &str, verifier: &str) -> Result<Tokens, String> {
    let client = http_client()?;
    let body = match provider {
        CloudProvider::GoogleDrive => form_encode(&[
            ("code", code),
            ("client_id", &google_client_id()),
            ("client_secret", &google_client_secret()),
            ("code_verifier", verifier),
            ("grant_type", "authorization_code"),
            ("redirect_uri", REDIRECT_URI),
        ]),
        CloudProvider::Dropbox => form_encode(&[
            ("code", code),
            ("client_id", &dropbox_app_key()),
            ("code_verifier", verifier),
            ("grant_type", "authorization_code"),
            ("redirect_uri", REDIRECT_URI),
        ]),
    };
    let endpoint = match provider {
        CloudProvider::GoogleDrive => "https://oauth2.googleapis.com/token",
        CloudProvider::Dropbox => "https://api.dropboxapi.com/oauth2/token",
    };
    let resp = client
        .post(endpoint)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("token exchange failed ({})", resp.status()));
    }
    let parsed: TokenResponse = resp.json().map_err(|e| e.to_string())?;
    Ok(token_from_response(parsed, None))
}

fn refresh_tokens(provider: CloudProvider, refresh: &str) -> Result<Tokens, String> {
    let client = http_client()?;
    let body = match provider {
        CloudProvider::GoogleDrive => form_encode(&[
            ("refresh_token", refresh),
            ("client_id", &google_client_id()),
            ("client_secret", &google_client_secret()),
            ("grant_type", "refresh_token"),
        ]),
        CloudProvider::Dropbox => form_encode(&[
            ("refresh_token", refresh),
            ("client_id", &dropbox_app_key()),
            ("grant_type", "refresh_token"),
        ]),
    };
    let endpoint = match provider {
        CloudProvider::GoogleDrive => "https://oauth2.googleapis.com/token",
        CloudProvider::Dropbox => "https://api.dropboxapi.com/oauth2/token",
    };
    let resp = client
        .post(endpoint)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("token refresh failed ({})", resp.status()));
    }
    let parsed: TokenResponse = resp.json().map_err(|e| e.to_string())?;
    Ok(token_from_response(parsed, Some(refresh.to_string())))
}

// --------------------------------------------------------------- Google Drive

const DRIVE_FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const DRIVE_AUDIO_MIMES: [&str; 8] = [
    "audio/mpeg",
    "audio/mp4",
    "audio/x-m4a",
    "audio/flac",
    "audio/wav",
    "audio/ogg",
    "audio/aac",
    "audio/x-ms-wma",
];

/// List one Drive folder: its subfolders + audio files (non-recursive).
fn drive_browse(access: &str, folder: &str) -> Result<Vec<CloudEntry>, String> {
    let parent = if folder.is_empty() { "root" } else { folder };
    let client = http_client()?;
    let kinds = std::iter::once(format!("mimeType='{DRIVE_FOLDER_MIME}'"))
        .chain(DRIVE_AUDIO_MIMES.iter().map(|m| format!("mimeType='{m}'")))
        .collect::<Vec<_>>()
        .join(" or ");
    let query = format!("'{parent}' in parents and trashed=false and ({kinds})");
    let fields = "nextPageToken,files(id,name,mimeType,size)";

    let mut entries = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut url = format!(
            "https://www.googleapis.com/drive/v3/files?q={}&fields={}&pageSize=200&orderBy=folder,name",
            pct_encode(&query),
            pct_encode(fields),
        );
        if let Some(pt) = &page_token {
            url.push_str(&format!("&pageToken={}", pct_encode(pt)));
        }
        let resp = client
            .get(&url)
            .bearer_auth(access)
            .send()
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("Drive list failed ({})", resp.status()));
        }
        let data: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
        for f in data["files"].as_array().unwrap_or(&Vec::new()) {
            let id = f["id"].as_str().unwrap_or_default().to_string();
            let name = f["name"].as_str().unwrap_or_default().to_string();
            if id.is_empty() || name.is_empty() {
                continue;
            }
            let is_folder = f["mimeType"].as_str() == Some(DRIVE_FOLDER_MIME);
            let size = f["size"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0);
            entries.push(CloudEntry {
                provider: CloudProvider::GoogleDrive,
                account_id: String::new(),
                id,
                name,
                is_folder,
                size,
                folder: None,
            });
        }
        match data["nextPageToken"].as_str() {
            Some(pt) => page_token = Some(pt.to_string()),
            None => break,
        }
    }
    Ok(entries)
}

/// Cap on distinct parent-folder name lookups for the flat listing, so a huge
/// account can't fan out into thousands of metadata calls. Songs always appear;
/// only some folder *labels* are omitted past this many distinct folders.
const DRIVE_FOLDER_NAME_CAP: usize = 300;

/// List EVERY audio file in the Drive account (flat, all folders) in one
/// paginated query, then resolve each file's parent folder name for grouping.
/// Mirrors the mobile app's `GoogleDriveService.listAudioFiles`.
fn drive_all_audio(access: &str) -> Result<Vec<CloudEntry>, String> {
    let client = http_client()?;
    let kinds = DRIVE_AUDIO_MIMES
        .iter()
        .map(|m| format!("mimeType='{m}'"))
        .collect::<Vec<_>>()
        .join(" or ");
    // No parent filter → every audio file the user can access, across all folders.
    let query = format!("({kinds}) and trashed=false");
    let fields = "nextPageToken,files(id,name,size,parents)";

    // (id, name, size, parent_id) — folder names are resolved in a second pass.
    let mut raw: Vec<(String, String, u64, Option<String>)> = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut url = format!(
            "https://www.googleapis.com/drive/v3/files?q={}&fields={}&pageSize=1000&orderBy=name",
            pct_encode(&query),
            pct_encode(fields),
        );
        if let Some(pt) = &page_token {
            url.push_str(&format!("&pageToken={}", pct_encode(pt)));
        }
        let resp = client
            .get(&url)
            .bearer_auth(access)
            .send()
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("Drive list failed ({})", resp.status()));
        }
        let data: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
        for f in data["files"].as_array().unwrap_or(&Vec::new()) {
            let id = f["id"].as_str().unwrap_or_default().to_string();
            let name = f["name"].as_str().unwrap_or_default().to_string();
            if id.is_empty() || name.is_empty() {
                continue;
            }
            let size = f["size"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0);
            let parent = f["parents"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            raw.push((id, name, size, parent));
        }
        match data["nextPageToken"].as_str() {
            Some(pt) => page_token = Some(pt.to_string()),
            None => break,
        }
    }

    // Resolve distinct parent folder names (bounded), so songs group by folder.
    let mut names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (_, _, _, parent) in &raw {
        if let Some(pid) = parent {
            if names.contains_key(pid) || names.len() >= DRIVE_FOLDER_NAME_CAP {
                continue;
            }
            if let Some(n) = drive_folder_name(&client, access, pid) {
                names.insert(pid.clone(), n);
            }
        }
    }

    Ok(raw
        .into_iter()
        .map(|(id, name, size, parent)| {
            let folder = parent.and_then(|p| names.get(&p).cloned());
            CloudEntry {
                provider: CloudProvider::GoogleDrive,
                account_id: String::new(),
                id,
                name,
                is_folder: false,
                size,
                folder,
            }
        })
        .collect())
}

/// Fetch a Drive folder's display name by id (`None` on any error).
fn drive_folder_name(client: &reqwest::blocking::Client, access: &str, id: &str) -> Option<String> {
    let url = format!("https://www.googleapis.com/drive/v3/files/{id}?fields=name");
    let resp = client.get(&url).bearer_auth(access).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let data: serde_json::Value = resp.json().ok()?;
    data["name"].as_str().map(|s| s.to_string())
}

// ------------------------------------------------------------------- Dropbox

fn dropbox_is_audio(name: &str) -> bool {
    matches!(
        name.rsplit('.').next().map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("mp3" | "m4a" | "flac" | "wav" | "ogg" | "aac" | "wma")
    )
}

/// List one Dropbox folder: its subfolders + audio files (non-recursive).
/// `folder` is the path ("" = root).
fn dropbox_browse(access: &str, folder: &str) -> Result<Vec<CloudEntry>, String> {
    let client = http_client()?;
    let mut entries = Vec::new();

    let mut resp = client
        .post("https://api.dropboxapi.com/2/files/list_folder")
        .bearer_auth(access)
        .json(&serde_json::json!({ "path": folder, "recursive": false, "limit": 2000 }))
        .send()
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("Dropbox list failed ({})", resp.status()));
    }

    loop {
        let data: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
        for entry in data["entries"].as_array().unwrap_or(&Vec::new()) {
            let tag = entry[".tag"].as_str().unwrap_or("");
            let name = entry["name"].as_str().unwrap_or_default().to_string();
            let path = entry["path_lower"].as_str().unwrap_or_default().to_string();
            if path.is_empty() || name.is_empty() {
                continue;
            }
            match tag {
                "folder" => entries.push(CloudEntry {
                    provider: CloudProvider::Dropbox,
                    account_id: String::new(),
                    id: path,
                    name,
                    is_folder: true,
                    size: 0,
                    folder: None,
                }),
                "file" if dropbox_is_audio(&name) => entries.push(CloudEntry {
                    provider: CloudProvider::Dropbox,
                    account_id: String::new(),
                    id: path,
                    name,
                    is_folder: false,
                    size: entry["size"].as_u64().unwrap_or(0),
                    folder: None,
                }),
                _ => {}
            }
        }
        if data["has_more"].as_bool() != Some(true) {
            break;
        }
        let cursor = match data["cursor"].as_str() {
            Some(c) => c.to_string(),
            None => break,
        };
        resp = client
            .post("https://api.dropboxapi.com/2/files/list_folder/continue")
            .bearer_auth(access)
            .json(&serde_json::json!({ "cursor": cursor }))
            .send()
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            break;
        }
    }
    Ok(entries)
}

/// List EVERY audio file in the Dropbox account (recursive from the root) in one
/// cursor-paged sweep, mirroring the mobile app. Each file carries its parent
/// folder name as a grouping label.
fn dropbox_all_audio(access: &str) -> Result<Vec<CloudEntry>, String> {
    let client = http_client()?;
    let mut entries = Vec::new();

    let mut resp = client
        .post("https://api.dropboxapi.com/2/files/list_folder")
        .bearer_auth(access)
        .json(&serde_json::json!({ "path": "", "recursive": true, "limit": 2000 }))
        .send()
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("Dropbox list failed ({})", resp.status()));
    }

    loop {
        let data: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
        for entry in data["entries"].as_array().unwrap_or(&Vec::new()) {
            if entry[".tag"].as_str() != Some("file") {
                continue;
            }
            let name = entry["name"].as_str().unwrap_or_default().to_string();
            let path = entry["path_lower"].as_str().unwrap_or_default().to_string();
            if path.is_empty() || name.is_empty() || !dropbox_is_audio(&name) {
                continue;
            }
            let folder = dropbox_parent_folder(entry["path_display"].as_str().unwrap_or(&path));
            entries.push(CloudEntry {
                provider: CloudProvider::Dropbox,
                account_id: String::new(),
                id: path,
                name,
                is_folder: false,
                size: entry["size"].as_u64().unwrap_or(0),
                folder,
            });
        }
        if data["has_more"].as_bool() != Some(true) {
            break;
        }
        let cursor = match data["cursor"].as_str() {
            Some(c) => c.to_string(),
            None => break,
        };
        resp = client
            .post("https://api.dropboxapi.com/2/files/list_folder/continue")
            .bearer_auth(access)
            .json(&serde_json::json!({ "cursor": cursor }))
            .send()
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            break;
        }
    }
    Ok(entries)
}

/// Parent folder name of a Dropbox path ("/Music/Album/Song.mp3" → "Album");
/// `None` at the account root.
fn dropbox_parent_folder(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    (parts.len() >= 2).then(|| parts[parts.len() - 2].to_string())
}

fn dropbox_temporary_link(access: &str, path: &str) -> Result<String, String> {
    let client = http_client()?;
    let resp = client
        .post("https://api.dropboxapi.com/2/files/get_temporary_link")
        .bearer_auth(access)
        .json(&serde_json::json!({ "path": path }))
        .send()
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("Dropbox link failed ({})", resp.status()));
    }
    let data: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
    data["link"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "no temporary link returned".into())
}

#[cfg(test)]
mod tests {
    use super::dropbox_parent_folder;

    #[test]
    fn parent_folder_of_nested_path() {
        assert_eq!(
            dropbox_parent_folder("/Music/Album/Song.mp3").as_deref(),
            Some("Album")
        );
    }

    #[test]
    fn no_parent_folder_at_root() {
        assert_eq!(dropbox_parent_folder("/Song.mp3"), None);
        assert_eq!(dropbox_parent_folder(""), None);
    }
}
