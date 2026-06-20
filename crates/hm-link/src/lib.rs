//! Phone Link — stream the phone's music library through the desktop.
//!
//! The phone runs a small HTTP **media server** and advertises it over mDNS;
//! this crate is the desktop **client**:
//!
//! * [`LinkState::discover`] browses the LAN for phones advertising
//!   `_hypemuzik._tcp` with `role=source`.
//! * [`LinkState::pair`] performs the PIN handshake (`POST /pair`) and persists
//!   the returned long-lived token (so reconnects are silent).
//! * [`LinkState::library`] fetches the phone's track list (`GET /library`).
//! * [`LinkState::stream_target`] resolves a `(url, headers)` for one track that
//!   the audio engine can stream + decode through the DSP chain (`GET /stream`).
//!
//! It deliberately knows nothing about Tauri or the audio engine — the app
//! layer wires `stream_target` into `engine.play_stream`. See
//! `docs/superpowers/specs/2026-06-18-phone-link-streaming.md`.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::{Deserialize, Serialize};

/// The mDNS service type both apps advertise / browse.
pub const SERVICE_TYPE: &str = "_hypemuzik._tcp.local.";

// -------------------------------------------------------------------- types

/// A phone discovered on the LAN or already paired (mirrors the front-end type).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneDevice {
    /// Stable per-install id advertised by the phone (mDNS TXT `id`).
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
}

/// One track in the phone's library. This struct is the wire shape (the phone
/// emits camelCase JSON) **and** the front-end shape — it round-trips verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneTrack {
    /// On-device song id (the phone maps it back to a file path to stream).
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    /// File extension (e.g. `mp3`) — appended to the stream URL so the decoder
    /// gets a format hint.
    pub ext: String,
    /// The folder the file lives in on the phone (its immediate parent folder
    /// name), so the desktop can browse the phone's music by folder.
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub has_art: bool,
}

#[derive(Debug, Deserialize)]
struct LibraryResponse {
    #[serde(default)]
    tracks: Vec<PhoneTrack>,
}

#[derive(Debug, Deserialize)]
struct PairResponse {
    token: String,
}

/// The phone's `GET /ping` reply — its id + display name.
#[derive(Debug, Deserialize)]
struct PingResponse {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
}

/// A persisted pairing: enough to reach the phone and authenticate silently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Paired {
    id: String,
    name: String,
    host: String,
    port: u16,
    token: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Store {
    /// This desktop's stable identity, sent to the phone during pairing so it
    /// can show/recognise us. Generated on first run.
    #[serde(default)]
    self_id: String,
    #[serde(default)]
    devices: Vec<Paired>,
}

/// Managed state: the paired-device store and its on-disk path.
pub struct LinkState {
    inner: Mutex<Store>,
    path: PathBuf,
}

impl LinkState {
    /// Load the store from `path` (or start empty), ensuring a stable `self_id`.
    pub fn load(path: PathBuf) -> Self {
        let store = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<Store>(&t).ok())
            .unwrap_or_default();
        let state = Self {
            inner: Mutex::new(store),
            path,
        };
        {
            let mut s = state.inner.lock().expect("link poisoned");
            if s.self_id.is_empty() {
                s.self_id = random_hex(16);
                state.save(&s);
            }
        }
        state
    }

    fn save(&self, store: &Store) {
        if let Ok(json) = serde_json::to_string_pretty(store) {
            let tmp = self.path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }
    }

    /// This desktop's stable id.
    pub fn self_id(&self) -> String {
        self.inner.lock().expect("link poisoned").self_id.clone()
    }

    /// The phones we've already paired with.
    pub fn paired(&self) -> Vec<PhoneDevice> {
        self.inner
            .lock()
            .expect("link poisoned")
            .devices
            .iter()
            .map(|d| PhoneDevice {
                id: d.id.clone(),
                name: d.name.clone(),
                host: d.host.clone(),
                port: d.port,
            })
            .collect()
    }

    /// Forget a pairing (revokes the token locally).
    pub fn unpair(&self, device_id: &str) {
        let mut s = self.inner.lock().expect("link poisoned");
        s.devices.retain(|d| d.id != device_id);
        self.save(&s);
    }

    fn remember(&self, dev: Paired) {
        let mut s = self.inner.lock().expect("link poisoned");
        s.devices.retain(|d| d.id != dev.id);
        s.devices.push(dev);
        self.save(&s);
    }

    fn find(&self, device_id: &str) -> Result<Paired, String> {
        self.inner
            .lock()
            .expect("link poisoned")
            .devices
            .iter()
            .find(|d| d.id == device_id)
            .cloned()
            .ok_or_else(|| "device not paired".to_string())
    }

    // ----- network operations -----

    /// Browse the LAN for ~`timeout` and return phones advertising `role=source`.
    pub fn discover(&self, timeout: Duration) -> Result<Vec<PhoneDevice>, String> {
        let daemon = ServiceDaemon::new().map_err(|e| e.to_string())?;
        let receiver = daemon.browse(SERVICE_TYPE).map_err(|e| e.to_string())?;
        let deadline = Instant::now() + timeout;
        let mut found: HashMap<String, PhoneDevice> = HashMap::new();

        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            match receiver.recv_timeout(remaining) {
                Ok(ServiceEvent::ServiceResolved(info)) => {
                    if let Some(dev) = parse_phone(&info) {
                        found.insert(dev.id.clone(), dev);
                    }
                }
                Ok(_) => {}
                Err(_) => break, // timed out
            }
        }
        let _ = daemon.shutdown();
        let mut devices: Vec<PhoneDevice> = found.into_values().collect();
        devices.sort_by_key(|d| d.name.to_lowercase());
        // Silent reconnect: a paired phone may have a new DHCP address.
        self.update_addresses(&devices);
        Ok(devices)
    }

    /// Update the stored host/port of any paired device that was just
    /// rediscovered at a new address (so a changed phone IP self-heals).
    pub fn update_addresses(&self, discovered: &[PhoneDevice]) {
        let mut s = self.inner.lock().expect("link poisoned");
        let mut changed = false;
        for dev in &mut s.devices {
            if let Some(found) = discovered.iter().find(|d| d.id == dev.id) {
                if dev.host != found.host || dev.port != found.port {
                    dev.host = found.host.clone();
                    dev.port = found.port;
                    changed = true;
                }
            }
        }
        if changed {
            self.save(&s);
        }
    }

    /// Run the PIN handshake against a phone and persist the returned token.
    pub fn pair(
        &self,
        host: &str,
        port: u16,
        name: &str,
        device_id: &str,
        pin: &str,
    ) -> Result<PhoneDevice, String> {
        let self_id = self.self_id();
        let body = serde_json::json!({
            "pin": pin,
            "deviceId": self_id,
            "deviceName": device_name(),
        });
        let url = format!("http://{host}:{port}/pair");
        let resp = http_client()?
            .post(&url)
            .json(&body)
            .send()
            .map_err(|e| format!("couldn't reach the phone: {e}"))?;
        if resp.status().as_u16() == 403 {
            return Err("incorrect or expired PIN".into());
        }
        if !resp.status().is_success() {
            return Err(format!("pairing failed ({})", resp.status()));
        }
        let parsed: PairResponse = resp.json().map_err(|e| e.to_string())?;
        let device = PhoneDevice {
            id: device_id.to_string(),
            name: name.to_string(),
            host: host.to_string(),
            port,
        };
        self.remember(Paired {
            id: device.id.clone(),
            name: device.name.clone(),
            host: device.host.clone(),
            port,
            token: parsed.token,
        });
        Ok(device)
    }

    /// Pair with a phone by its address (`host:port`) + PIN, without mDNS
    /// discovery — pings the phone for its id/name first, then runs the normal
    /// PIN handshake. Lets pairing work when discovery can't see the phone
    /// (e.g. multicast blocked) or across networks (e.g. over a VPN).
    pub fn pair_by_address(
        &self,
        host: &str,
        port: u16,
        pin: &str,
    ) -> Result<PhoneDevice, String> {
        let url = format!("http://{host}:{port}/ping");
        let resp = http_client()?
            .get(&url)
            .send()
            .map_err(|e| format!("couldn't reach the phone at {host}:{port}: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("the phone didn't respond ({})", resp.status()));
        }
        let ping: PingResponse = resp.json().map_err(|e| e.to_string())?;
        let id = if ping.id.is_empty() {
            format!("{host}:{port}")
        } else {
            ping.id
        };
        let name = if ping.name.is_empty() {
            "Phone".to_string()
        } else {
            ping.name
        };
        self.pair(host, port, &name, &id, pin)
    }

    /// Fetch a track's embedded artwork as a `data:` URI (so it can drop
    /// straight into an `<img src>`), or `None` if the track has no art or the
    /// phone is unreachable. Goes through the authenticated client, so no webview
    /// CSP changes are needed.
    pub fn artwork_data_uri(&self, device_id: &str, track_id: &str) -> Option<String> {
        let dev = self.find(device_id).ok()?;
        let url = format!("http://{}:{}/art/{}", dev.host, dev.port, track_id);
        let resp = http_client()
            .ok()?
            .get(&url)
            .bearer_auth(&dev.token)
            .send()
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("image/jpeg")
            .to_string();
        let bytes = resp.bytes().ok()?;
        if bytes.is_empty() {
            return None;
        }
        Some(format!(
            "data:{};base64,{}",
            content_type,
            base64::engine::general_purpose::STANDARD.encode(&bytes)
        ))
    }

    /// Fetch a track's lyrics from the phone — the `.lrc` file the user keeps
    /// next to the audio (or embedded lyrics), as raw LRC/plain text. `None`
    /// when the phone has none or is unreachable. Goes through the authenticated
    /// client, so no webview CSP changes are needed.
    pub fn lyrics(&self, device_id: &str, track_id: &str) -> Option<String> {
        let dev = self.find(device_id).ok()?;
        let url = format!("http://{}:{}/lyrics/{}", dev.host, dev.port, track_id);
        let resp = http_client()
            .ok()?
            .get(&url)
            .bearer_auth(&dev.token)
            .send()
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let text = resp.text().ok()?;
        (!text.trim().is_empty()).then_some(text)
    }

    /// Fetch the phone's track list.
    pub fn library(&self, device_id: &str) -> Result<Vec<PhoneTrack>, String> {
        let dev = self.find(device_id)?;
        let url = format!("http://{}:{}/library", dev.host, dev.port);
        let resp = http_client()?
            .get(&url)
            .bearer_auth(&dev.token)
            .send()
            .map_err(|e| format!("couldn't reach the phone: {e}"))?;
        if resp.status().as_u16() == 401 {
            return Err("pairing expired — pair with the phone again".into());
        }
        if !resp.status().is_success() {
            return Err(format!("couldn't load the library ({})", resp.status()));
        }
        let parsed: LibraryResponse = resp.json().map_err(|e| e.to_string())?;
        Ok(parsed.tracks)
    }

    /// Resolve a streamable `(url, headers)` for one track. The extension is
    /// appended so the decoder gets a format hint from the URL.
    pub fn stream_target(
        &self,
        device_id: &str,
        track_id: &str,
        ext: &str,
    ) -> Result<(String, Vec<(String, String)>), String> {
        let dev = self.find(device_id)?;
        Ok(stream_url(&dev, track_id, ext))
    }

    // ----- cast (push) support -----

    /// Whether `token` belongs to a phone we've paired with. Used to
    /// authenticate cast/transport requests arriving at the desktop's control
    /// server (the phone proves who it is by its bearer token).
    pub fn is_known_token(&self, token: &str) -> bool {
        if token.is_empty() {
            return false;
        }
        self.inner
            .lock()
            .expect("link poisoned")
            .devices
            .iter()
            .any(|d| d.token == token)
    }

    /// Resolve a stream `(url, headers)` for the phone whose token matches
    /// `token` — i.e. the phone that is casting to us. `None` if no paired
    /// phone has that token.
    pub fn stream_target_for_token(
        &self,
        token: &str,
        track_id: &str,
        ext: &str,
    ) -> Option<(String, Vec<(String, String)>)> {
        if token.is_empty() {
            return None;
        }
        let s = self.inner.lock().expect("link poisoned");
        let dev = s.devices.iter().find(|d| d.token == token)?;
        Some(stream_url(dev, track_id, ext))
    }
}

/// Build the `(stream url, bearer headers)` for a paired device + track.
fn stream_url(dev: &Paired, track_id: &str, ext: &str) -> (String, Vec<(String, String)>) {
    let url = format!(
        "http://{}:{}/stream/{}{}",
        dev.host,
        dev.port,
        track_id,
        sanitize_ext(ext)
    );
    let headers = vec![("Authorization".to_string(), format!("Bearer {}", dev.token))];
    (url, headers)
}

/// Keeps an mDNS advertisement alive; withdraws it on drop.
pub struct Advertiser {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Drop for Advertiser {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

/// Advertise this desktop as a Phone-Link **player** so phones can discover it
/// and cast to its control server on `port`. Hold the returned [`Advertiser`]
/// for as long as the advertisement should be visible.
pub fn advertise_player(name: &str, id: &str, port: u16) -> Result<Advertiser, String> {
    let daemon = ServiceDaemon::new().map_err(|e| e.to_string())?;
    let mut props = HashMap::new();
    props.insert("role".to_string(), "player".to_string());
    props.insert("id".to_string(), id.to_string());
    props.insert("name".to_string(), name.to_string());
    props.insert("v".to_string(), "1".to_string());

    let host = format!("hypemuzik-{id}.local.");
    let service = ServiceInfo::new(SERVICE_TYPE, name, &host, "", port, props)
        .map_err(|e| e.to_string())?
        .enable_addr_auto();
    let fullname = service.get_fullname().to_string();
    daemon.register(service).map_err(|e| e.to_string())?;
    Ok(Advertiser { daemon, fullname })
}

// ------------------------------------------------------------------ helpers

/// Lowercased `.ext` suffix when `ext` is a known audio extension, else empty
/// (symphonia still probes by content). Guards against odd values in the URL.
fn sanitize_ext(ext: &str) -> String {
    let e = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    if matches!(e.as_str(), "mp3" | "aac" | "ogg" | "flac" | "m4a" | "wav") {
        format!(".{e}")
    } else {
        String::new()
    }
}

/// Bracket IPv6 literals so they're valid in a URL authority.
fn ip_host(addr: &IpAddr) -> String {
    match addr {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    }
}

/// Turn a resolved mDNS record into a phone (a music *source*), or `None` if it
/// isn't one or has no address.
fn parse_phone(info: &ServiceInfo) -> Option<PhoneDevice> {
    if info.get_property_val_str("role") != Some("source") {
        return None;
    }
    let addresses = info.get_addresses();
    let addr = addresses
        .iter()
        .find(|a| a.is_ipv4())
        .or_else(|| addresses.iter().next())?;
    let id = info
        .get_property_val_str("id")
        .unwrap_or_else(|| info.get_fullname())
        .to_string();
    let name = info
        .get_property_val_str("name")
        .unwrap_or("Phone")
        .to_string();
    Some(PhoneDevice {
        id,
        name,
        host: ip_host(addr),
        port: info.get_port(),
    })
}

/// Continuously browse the LAN for phones until `stop` is set, calling
/// `on_found` for each one as it's resolved (and re-resolved — addresses can
/// change). Unlike [`LinkState::discover`] this never closes the browse window,
/// so a phone is picked up the instant it appears — no polling or refresh.
///
/// On macOS this drives the system Bonjour daemon (`dns-sd`), because an app's
/// own raw multicast (what `mdns_sd` does) is blocked there; everywhere else it
/// uses the pure-Rust `mdns_sd` browser (no system mDNS dependency).
pub fn watch(stop: Arc<AtomicBool>, on_found: impl Fn(PhoneDevice)) {
    #[cfg(target_os = "macos")]
    watch_bonjour(stop, on_found);
    #[cfg(not(target_os = "macos"))]
    watch_mdns(stop, on_found);
}

#[cfg(not(target_os = "macos"))]
fn watch_mdns(stop: Arc<AtomicBool>, on_found: impl Fn(PhoneDevice)) {
    let Ok(daemon) = ServiceDaemon::new() else {
        return;
    };
    let Ok(receiver) = daemon.browse(SERVICE_TYPE) else {
        return;
    };
    while !stop.load(Ordering::Relaxed) {
        match receiver.recv_timeout(Duration::from_millis(400)) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                if let Some(dev) = parse_phone(&info) {
                    on_found(dev);
                }
            }
            // Other events, or a recv timeout: loop to re-check `stop`. The
            // daemon stays alive here, so the channel won't disconnect.
            _ => {}
        }
    }
    let _ = daemon.shutdown();
}

/// macOS: browse via the system Bonjour daemon by parsing `dns-sd -Z`'s
/// continuous SRV + TXT output (the same daemon `dns-sd -B` uses).
#[cfg(target_os = "macos")]
fn watch_bonjour(stop: Arc<AtomicBool>, on_found: impl Fn(PhoneDevice)) {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    #[derive(Default)]
    struct Partial {
        host: Option<String>,
        port: Option<u16>,
        role: Option<String>,
        id: Option<String>,
        name: Option<String>,
    }

    let child = Command::new("dns-sd")
        .args(["-Z", "_hypemuzik._tcp", "local."])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        return;
    };
    let child = Arc::new(Mutex::new(child));
    {
        // `dns-sd` is a long-running process that blocks the line reader below;
        // kill it when `stop` is set so the reader (and this thread) exits.
        let killer = child.clone();
        let stop = stop.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(250));
            }
            if let Ok(mut c) = killer.lock() {
                let _ = c.kill();
            }
        });
    }

    let mut map: HashMap<String, Partial> = HashMap::new();
    for line in BufReader::new(stdout).lines() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let Ok(line) = line else {
            break;
        };
        let trimmed = line.trim_start();
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() < 2 || !parts[0].contains("_hypemuzik._tcp") {
            continue;
        }
        let instance = parts[0].to_string();
        match parts[1] {
            "SRV" if parts.len() >= 6 => {
                if let Ok(port) = parts[4].parse::<u16>() {
                    let e = map.entry(instance.clone()).or_default();
                    e.host = Some(parts[5].trim_end_matches('.').to_string());
                    e.port = Some(port);
                }
            }
            "TXT" => {
                let e = map.entry(instance.clone()).or_default();
                for (k, v) in parse_txt_quoted(trimmed) {
                    match k.as_str() {
                        "role" => e.role = Some(v),
                        "id" => e.id = Some(v),
                        "name" => e.name = Some(v),
                        _ => {}
                    }
                }
            }
            _ => continue,
        }

        if let Some(p) = map.get(&instance) {
            if p.role.as_deref() == Some("source") {
                if let (Some(host), Some(port)) = (&p.host, p.port) {
                    on_found(PhoneDevice {
                        id: p.id.clone().unwrap_or_else(|| instance.clone()),
                        name: p.name.clone().unwrap_or_else(|| "Phone".to_string()),
                        host: resolve_host(host, port).unwrap_or_else(|| host.clone()),
                        port,
                    });
                }
            }
        }
    }
    let _ = child.lock().map(|mut c| {
        let _ = c.kill();
        let _ = c.wait();
    });
}

/// Resolve an mDNS hostname (`name.local`) to an IPv4 string via the OS resolver
/// (mDNSResponder), so the rest of the app streams to a plain address.
#[cfg(target_os = "macos")]
fn resolve_host(host: &str, port: u16) -> Option<String> {
    use std::net::ToSocketAddrs;
    let addrs: Vec<_> = format!("{host}:{port}").to_socket_addrs().ok()?.collect();
    let a = addrs.iter().find(|a| a.is_ipv4()).or_else(|| addrs.first())?;
    Some(ip_host(&a.ip()))
}

/// Extract `key=value` pairs from `dns-sd`'s quoted TXT output
/// (e.g. `"role=source" "name=My Phone"`).
#[cfg(target_os = "macos")]
fn parse_txt_quoted(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut buf = String::new();
        for c2 in chars.by_ref() {
            if c2 == '"' {
                break;
            }
            buf.push(c2);
        }
        if let Some(eq) = buf.find('=') {
            out.push((buf[..eq].to_string(), buf[eq + 1..].to_string()));
        }
    }
    out
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    let _ = getrandom::getrandom(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

/// A friendly name for this desktop, shown on the phone during pairing/cast.
pub fn device_name() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().trim_end_matches(".local").to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "HypeMuzik Desktop".to_string())
}

// -------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("hm_link_test_{tag}_{}.json", random_hex(6)))
    }

    #[test]
    fn self_id_is_generated_and_persists_across_loads() {
        let path = temp_path("selfid");
        let first = LinkState::load(path.clone());
        let id = first.self_id();
        assert!(!id.is_empty(), "a self id should be generated");
        drop(first);

        let second = LinkState::load(path.clone());
        assert_eq!(second.self_id(), id, "self id must persist across loads");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn pairing_is_stored_streamable_and_revocable() {
        let path = temp_path("pair");
        let link = LinkState::load(path.clone());
        link.remember(Paired {
            id: "phone1".into(),
            name: "iPhone".into(),
            host: "192.168.1.5".into(),
            port: 8080,
            token: "tok123".into(),
        });

        let paired = link.paired();
        assert_eq!(paired.len(), 1);
        assert_eq!(paired[0].id, "phone1");
        assert_eq!(paired[0].host, "192.168.1.5");

        let (url, headers) = link.stream_target("phone1", "42", "mp3").unwrap();
        assert_eq!(url, "http://192.168.1.5:8080/stream/42.mp3");
        assert_eq!(
            headers,
            vec![("Authorization".to_string(), "Bearer tok123".to_string())]
        );

        assert!(
            link.stream_target("nope", "1", "mp3").is_err(),
            "unknown device must error"
        );

        link.unpair("phone1");
        assert!(link.paired().is_empty(), "unpair must remove the device");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn remember_replaces_an_existing_pairing() {
        let path = temp_path("replace");
        let link = LinkState::load(path.clone());
        for token in ["old", "new"] {
            link.remember(Paired {
                id: "p".into(),
                name: "Phone".into(),
                host: "10.0.0.2".into(),
                port: 9000,
                token: token.into(),
            });
        }
        assert_eq!(link.paired().len(), 1, "re-pairing must not duplicate");
        let (_, headers) = link.stream_target("p", "1", "flac").unwrap();
        assert_eq!(headers[0].1, "Bearer new", "token should be the latest");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn discovery_refreshes_a_paired_phones_changed_address() {
        let path = temp_path("reconnect");
        let link = LinkState::load(path.clone());
        link.remember(Paired {
            id: "p".into(),
            name: "Phone".into(),
            host: "192.168.1.5".into(),
            port: 8080,
            token: "t".into(),
        });

        // The phone reappears at a new DHCP address.
        link.update_addresses(&[PhoneDevice {
            id: "p".into(),
            name: "Phone".into(),
            host: "192.168.1.99".into(),
            port: 9000,
        }]);
        let (url, _) = link.stream_target("p", "1", "mp3").unwrap();
        assert_eq!(url, "http://192.168.1.99:9000/stream/1.mp3");

        // An unrelated discovered device must not create a pairing.
        link.update_addresses(&[PhoneDevice {
            id: "stranger".into(),
            name: "X".into(),
            host: "10.0.0.1".into(),
            port: 1,
        }]);
        assert_eq!(link.paired().len(), 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn token_resolution_authenticates_and_streams_casts() {
        let path = temp_path("token");
        let link = LinkState::load(path.clone());
        link.remember(Paired {
            id: "p".into(),
            name: "Phone".into(),
            host: "10.0.0.9".into(),
            port: 7000,
            token: "abc".into(),
        });

        assert!(link.is_known_token("abc"));
        assert!(!link.is_known_token("nope"));
        assert!(!link.is_known_token(""), "empty token is never known");

        let (url, headers) = link.stream_target_for_token("abc", "12", "mp3").unwrap();
        assert_eq!(url, "http://10.0.0.9:7000/stream/12.mp3");
        assert_eq!(headers[0].1, "Bearer abc");
        assert!(link.stream_target_for_token("nope", "12", "mp3").is_none());
        assert!(link.stream_target_for_token("", "12", "mp3").is_none());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn library_response_parses_phone_camelcase_payload() {
        let json = r#"{"tracks":[
            {"id":"7","title":"Song","artist":"A","album":null,
             "durationMs":215000,"ext":"mp3","hasArt":true},
            {"id":"8","title":"Two","artist":null,"album":"Disc",
             "durationMs":null,"ext":"flac","hasArt":false}
        ]}"#;
        let parsed: LibraryResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.tracks.len(), 2);
        let t = &parsed.tracks[0];
        assert_eq!(t.id, "7");
        assert_eq!(t.artist.as_deref(), Some("A"));
        assert_eq!(t.album, None);
        assert_eq!(t.duration_ms, Some(215_000));
        assert!(t.has_art);
        assert_eq!(parsed.tracks[1].album.as_deref(), Some("Disc"));
        assert!(!parsed.tracks[1].has_art);
    }

    #[test]
    fn phone_track_round_trips_to_camelcase_for_the_frontend() {
        let track = PhoneTrack {
            id: "9".into(),
            title: "T".into(),
            artist: Some("Artist".into()),
            album: None,
            duration_ms: Some(1000),
            ext: "m4a".into(),
            folder: Some("Rock".into()),
            has_art: true,
        };
        let json = serde_json::to_string(&track).unwrap();
        assert!(json.contains("\"durationMs\":1000"));
        assert!(json.contains("\"folder\":\"Rock\""));
        assert!(json.contains("\"hasArt\":true"));
    }

    #[test]
    fn sanitize_ext_only_allows_known_audio_extensions() {
        assert_eq!(sanitize_ext("mp3"), ".mp3");
        assert_eq!(sanitize_ext(".FLAC"), ".flac");
        assert_eq!(sanitize_ext("m4a"), ".m4a");
        assert_eq!(sanitize_ext("exe"), "");
        assert_eq!(sanitize_ext(""), "");
    }

    #[test]
    fn ipv6_hosts_are_bracketed() {
        let v4: IpAddr = "192.168.1.9".parse().unwrap();
        let v6: IpAddr = "fe80::1".parse().unwrap();
        assert_eq!(ip_host(&v4), "192.168.1.9");
        assert_eq!(ip_host(&v6), "[fe80::1]");
    }
}
