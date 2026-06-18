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
use std::sync::Mutex;
use std::time::{Duration, Instant};

use mdns_sd::{ServiceDaemon, ServiceEvent};
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
                    if info.get_property_val_str("role") != Some("source") {
                        continue;
                    }
                    let addresses = info.get_addresses();
                    let addr = addresses
                        .iter()
                        .find(|a| a.is_ipv4())
                        .or_else(|| addresses.iter().next());
                    if let Some(addr) = addr {
                        let id = info
                            .get_property_val_str("id")
                            .unwrap_or_else(|| info.get_fullname())
                            .to_string();
                        let name = info
                            .get_property_val_str("name")
                            .unwrap_or("Phone")
                            .to_string();
                        found.insert(
                            id.clone(),
                            PhoneDevice {
                                id,
                                name,
                                host: ip_host(addr),
                                port: info.get_port(),
                            },
                        );
                    }
                }
                Ok(_) => {}
                Err(_) => break, // timed out
            }
        }
        let _ = daemon.shutdown();
        let mut devices: Vec<PhoneDevice> = found.into_values().collect();
        devices.sort_by_key(|d| d.name.to_lowercase());
        Ok(devices)
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
            "deviceName": self_device_name(),
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
        let suffix = sanitize_ext(ext);
        let url = format!(
            "http://{}:{}/stream/{}{}",
            dev.host, dev.port, track_id, suffix
        );
        let headers = vec![("Authorization".to_string(), format!("Bearer {}", dev.token))];
        Ok((url, headers))
    }
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

/// A friendly name for this desktop, shown on the phone during pairing.
fn self_device_name() -> String {
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
            has_art: true,
        };
        let json = serde_json::to_string(&track).unwrap();
        assert!(json.contains("\"durationMs\":1000"));
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
