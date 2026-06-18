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

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}
fn google_client_id() -> String {
    env_or("HM_GDRIVE_CLIENT_ID", "")
}
fn google_client_secret() -> String {
    env_or("HM_GDRIVE_CLIENT_SECRET", "")
}
fn dropbox_app_key() -> String {
    env_or("HM_DROPBOX_APP_KEY", "")
}

// -------------------------------------------------------------------- types

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CloudProvider {
    GoogleDrive,
    Dropbox,
}

/// An audio file discovered in a cloud account (mirrors the front-end type).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudFile {
    pub provider: CloudProvider,
    /// Drive: file id. Dropbox: lowercased path.
    pub id: String,
    pub name: String,
    pub folder: String,
    pub size: u64,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Store {
    google: Option<Tokens>,
    dropbox: Option<Tokens>,
}

/// Connection status surfaced to the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudStatus {
    pub google_connected: bool,
    pub dropbox_connected: bool,
    pub google_configured: bool,
    pub dropbox_configured: bool,
}

/// Managed Tauri state: cloud tokens + their on-disk path.
pub struct CloudState {
    inner: Mutex<Store>,
    path: PathBuf,
}

impl CloudState {
    pub fn load(path: PathBuf) -> Self {
        let store = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<Store>(&t).ok())
            .unwrap_or_default();
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

    pub fn status(&self) -> CloudStatus {
        let s = self.inner.lock().expect("cloud poisoned");
        CloudStatus {
            google_connected: s.google.is_some(),
            dropbox_connected: s.dropbox.is_some(),
            google_configured: !google_client_id().is_empty(),
            dropbox_configured: !dropbox_app_key().is_empty(),
        }
    }

    pub fn disconnect(&self, provider: CloudProvider) {
        let mut s = self.inner.lock().expect("cloud poisoned");
        match provider {
            CloudProvider::GoogleDrive => s.google = None,
            CloudProvider::Dropbox => s.dropbox = None,
        }
        self.save(&s);
    }

    fn set(&self, provider: CloudProvider, tokens: Tokens) {
        let mut s = self.inner.lock().expect("cloud poisoned");
        match provider {
            CloudProvider::GoogleDrive => s.google = Some(tokens),
            CloudProvider::Dropbox => s.dropbox = Some(tokens),
        }
        self.save(&s);
    }

    fn tokens(&self, provider: CloudProvider) -> Option<Tokens> {
        let s = self.inner.lock().expect("cloud poisoned");
        match provider {
            CloudProvider::GoogleDrive => s.google.clone(),
            CloudProvider::Dropbox => s.dropbox.clone(),
        }
    }

    /// A valid access token, refreshing first if it is near expiry.
    fn access_token(&self, provider: CloudProvider) -> Result<String, String> {
        let mut tk = self
            .tokens(provider)
            .ok_or_else(|| "not connected".to_string())?;
        if tk.near_expiry() {
            if let Some(refresh) = tk.refresh.clone() {
                tk = refresh_tokens(provider, &refresh)?;
                self.set(provider, tk.clone());
            }
        }
        Ok(tk.access)
    }

    // ----- the high-level operations used by the commands -----

    /// Run the interactive OAuth flow and store the resulting tokens.
    pub fn connect(&self, provider: CloudProvider) -> Result<(), String> {
        let tokens = oauth_connect(provider)?;
        self.set(provider, tokens);
        Ok(())
    }

    /// List audio files in the connected account.
    pub fn list(&self, provider: CloudProvider) -> Result<Vec<CloudFile>, String> {
        let access = self.access_token(provider)?;
        match provider {
            CloudProvider::GoogleDrive => drive_list(&access),
            CloudProvider::Dropbox => dropbox_list(&access),
        }
    }

    /// Resolve a streamable `(url, headers)` for a file.
    pub fn stream_target(
        &self,
        provider: CloudProvider,
        file_id: &str,
    ) -> Result<(String, Vec<(String, String)>), String> {
        let access = self.access_token(provider)?;
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

fn drive_list(access: &str) -> Result<Vec<CloudFile>, String> {
    const MIMES: [&str; 8] = [
        "audio/mpeg",
        "audio/mp4",
        "audio/x-m4a",
        "audio/flac",
        "audio/wav",
        "audio/ogg",
        "audio/aac",
        "audio/x-ms-wma",
    ];
    let client = http_client()?;
    let mime_query = MIMES
        .iter()
        .map(|m| format!("mimeType='{m}'"))
        .collect::<Vec<_>>()
        .join(" or ");
    let query = format!("({mime_query}) and trashed=false");

    let fields = "nextPageToken,files(id,name,size,parents)";
    let mut files = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut url = format!(
            "https://www.googleapis.com/drive/v3/files?q={}&fields={}&pageSize=200",
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
            files.push(CloudFile {
                provider: CloudProvider::GoogleDrive,
                id,
                name,
                folder: "Drive".into(),
                size,
            });
        }
        match data["nextPageToken"].as_str() {
            Some(pt) => page_token = Some(pt.to_string()),
            None => break,
        }
    }
    Ok(files)
}

// ------------------------------------------------------------------- Dropbox

fn dropbox_is_audio(name: &str) -> bool {
    matches!(
        name.rsplit('.').next().map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("mp3" | "m4a" | "flac" | "wav" | "ogg" | "aac" | "wma")
    )
}

fn dropbox_list(access: &str) -> Result<Vec<CloudFile>, String> {
    let client = http_client()?;
    let mut files = Vec::new();

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
            if !dropbox_is_audio(&name) {
                continue;
            }
            let path = entry["path_lower"].as_str().unwrap_or_default().to_string();
            let folder = path
                .rfind('/')
                .map(|i| &path[..i])
                .filter(|s| !s.is_empty())
                .unwrap_or("/")
                .to_string();
            files.push(CloudFile {
                provider: CloudProvider::Dropbox,
                id: path,
                name,
                folder,
                size: entry["size"].as_u64().unwrap_or(0),
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
    Ok(files)
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
