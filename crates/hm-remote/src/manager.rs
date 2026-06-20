//! [`RemoteManager`] — the desktop's stateful owner of the iroh endpoint:
//! a stable identity, a persisted store of paired remote phones, the pairing
//! accept-loop, and one loopback proxy per connected phone.
//!
//! ## Pairing handshake (phone scans the desktop's QR)
//!
//! 1. Desktop [`RemoteManager::start_pairing`] opens a session and shows a QR
//!    encoding `hypemuzik://pair?ep=<desktopId>&pin=<6 digits>`.
//! 2. The phone scans it and **dials the desktop** on [`ALPN_PAIR`], sending
//!    `{ addr, name, pin, token }` — `addr` is the phone's own dialable iroh
//!    address, `token` is a media token it has authorised in its own shelf.
//! 3. The desktop verifies the pin, stores the peer, replies `{ accepted }`,
//!    then **dials the phone back** on [`ALPN_LINK`] and opens a loopback proxy,
//!    firing `on_paired` with `{ id, name, token, port }`.
//!
//! Thereafter the phone is just a paired device reachable at `127.0.0.1:<port>`,
//! so the existing `hm-link` HTTP calls work through the tunnel unchanged.

use anyhow::Result;
use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointAddr, EndpointId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{build_endpoint, endpoint_addr, open_proxy, RemoteProxy, ALPN_PAIR};

/// A paired remote phone, as persisted to disk (its loopback port is assigned
/// fresh each run, so it isn't stored).
#[derive(Clone, Serialize, Deserialize)]
struct RemotePeer {
    id: String,
    name: String,
    token: String,
}

/// Emitted to the app layer when a phone pairs or (re)connects — carries the
/// fresh loopback proxy port so the app can register it as a paired device.
#[derive(Clone, Debug, Serialize)]
pub struct PairedPhone {
    pub id: String,
    pub name: String,
    pub token: String,
    pub port: u16,
}

/// A remote phone's current status for the UI list.
#[derive(Clone, Debug, Serialize)]
pub struct RemotePhoneStatus {
    pub id: String,
    pub name: String,
    pub online: bool,
    pub port: Option<u16>,
}

/// What [`RemoteManager::start_pairing`] returns: show the QR + pin to the user.
#[derive(Clone, Debug, Serialize)]
pub struct PairingInfo {
    pub endpoint_id: String,
    pub pin: String,
    pub qr: String,
}

struct PairingSession {
    pin: String,
    expires: Instant,
}

/// The pairing request a phone sends over [`ALPN_PAIR`].
#[derive(Deserialize)]
struct PairRequest {
    addr: EndpointAddr,
    name: String,
    pin: String,
    token: String,
}

/// The desktop's reply.
#[derive(Serialize)]
struct PairReply {
    accepted: bool,
    name: String,
}

struct Inner {
    store_path: PathBuf,
    peers: Mutex<Vec<RemotePeer>>,
    proxies: Mutex<HashMap<String, RemoteProxy>>,
    pairing: Mutex<Option<PairingSession>>,
    on_paired: Box<dyn Fn(PairedPhone) + Send + Sync>,
    desktop_name: String,
}

/// Owns the iroh endpoint + its own tokio runtime; exposes a blocking API the
/// (synchronous) Tauri command layer can call.
pub struct RemoteManager {
    runtime: tokio::runtime::Runtime,
    endpoint: Endpoint,
    inner: Arc<Inner>,
}

impl RemoteManager {
    /// Build the endpoint (stable identity from `secret_path`), load the peer
    /// store, and start accepting pairings. `relay = true` uses n0's relays +
    /// discovery (reachable anywhere by id); `false` is direct-only (tests).
    pub fn new(
        secret_path: PathBuf,
        store_path: PathBuf,
        desktop_name: String,
        relay: bool,
        on_paired: impl Fn(PairedPhone) + Send + Sync + 'static,
    ) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        let secret = crate::secret::load_or_create_secret(&secret_path)?;
        let endpoint =
            runtime.block_on(build_endpoint(secret, vec![ALPN_PAIR.to_vec()], relay))?;
        let inner = Arc::new(Inner {
            peers: Mutex::new(load_peers(&store_path)),
            store_path,
            proxies: Mutex::new(HashMap::new()),
            pairing: Mutex::new(None),
            on_paired: Box::new(on_paired),
            desktop_name,
        });
        runtime.spawn(accept_pairings(endpoint.clone(), inner.clone()));
        Ok(Self {
            runtime,
            endpoint,
            inner,
        })
    }

    /// This desktop's stable iroh id (goes in the QR; discovery resolves it).
    pub fn endpoint_id(&self) -> String {
        self.endpoint.id().to_string()
    }

    /// This desktop's full dialable address (id + direct/relay addresses).
    pub fn dial_addr(&self) -> EndpointAddr {
        let ep = self.endpoint.clone();
        self.block(async move { endpoint_addr(&ep).await.unwrap_or_else(|_| ep.addr()) })
    }

    /// Open a pairing session and return the QR payload + pin to display.
    pub fn start_pairing(&self, ttl: Duration) -> PairingInfo {
        let pin = crate::secret::random_pin();
        let endpoint_id = self.endpoint_id();
        *self.inner.pairing.lock().expect("pairing lock") = Some(PairingSession {
            pin: pin.clone(),
            expires: Instant::now() + ttl,
        });
        PairingInfo {
            qr: format!("hypemuzik://pair?ep={endpoint_id}&pin={pin}"),
            endpoint_id,
            pin,
        }
    }

    /// Close any open pairing session.
    pub fn cancel_pairing(&self) {
        *self.inner.pairing.lock().expect("pairing lock") = None;
    }

    /// Dial every stored peer by id (discovery) that isn't already connected,
    /// returning all currently-connected phones with their loopback ports.
    pub fn connect_known(&self) -> Vec<PairedPhone> {
        let peers: Vec<RemotePeer> = self.inner.peers.lock().expect("peers lock").clone();
        let mut out = Vec::new();
        for p in peers {
            let existing = self
                .inner
                .proxies
                .lock()
                .expect("proxies lock")
                .get(&p.id)
                .map(|x| x.port);
            let port = match existing {
                Some(port) => Some(port),
                None => match EndpointId::from_str(&p.id) {
                    Ok(id) => {
                        let ep = self.endpoint.clone();
                        let inner = self.inner.clone();
                        self.block(async move {
                            connect_peer(&ep, &inner, EndpointAddr::new(id)).await.ok()
                        })
                    }
                    Err(_) => None,
                },
            };
            if let Some(port) = port {
                out.push(PairedPhone {
                    id: p.id,
                    name: p.name,
                    token: p.token,
                    port,
                });
            }
        }
        out
    }

    /// Current status of every paired phone (online = proxy connected).
    pub fn remote_phones(&self) -> Vec<RemotePhoneStatus> {
        let peers = self.inner.peers.lock().expect("peers lock");
        let proxies = self.inner.proxies.lock().expect("proxies lock");
        peers
            .iter()
            .map(|p| {
                let port = proxies.get(&p.id).map(|x| x.port);
                RemotePhoneStatus {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    online: port.is_some(),
                    port,
                }
            })
            .collect()
    }

    /// Forget a paired phone: drop its proxy + remove it from the store.
    pub fn forget(&self, id: &str) {
        self.inner.proxies.lock().expect("proxies lock").remove(id);
        let mut peers = self.inner.peers.lock().expect("peers lock");
        peers.retain(|p| p.id != id);
        save_peers(&self.inner.store_path, &peers);
    }

    /// Run `fut` on the manager's runtime and block until it finishes — safe to
    /// call from any thread (including another runtime's), unlike `block_on`.
    fn block<T, F>(&self, fut: F) -> T
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let (tx, rx) = std::sync::mpsc::channel();
        self.runtime.spawn(async move {
            let _ = tx.send(fut.await);
        });
        rx.recv().expect("remote runtime task dropped")
    }
}

/// Accept pairing connections until the endpoint closes.
async fn accept_pairings(ep: Endpoint, inner: Arc<Inner>) {
    while let Some(incoming) = ep.accept().await {
        let ep = ep.clone();
        let inner = inner.clone();
        tokio::spawn(async move {
            if let Ok(conn) = incoming.await {
                let _ = handle_pairing(ep, inner, conn).await;
            }
        });
    }
}

/// Handle one phone's pairing dial: verify the pin, store it, reply, then dial
/// back + proxy and fire `on_paired`.
async fn handle_pairing(ep: Endpoint, inner: Arc<Inner>, conn: Connection) -> Result<()> {
    let (mut send, mut recv) = conn.accept_bi().await?;
    let buf = recv.read_to_end(64 * 1024).await?;
    let req: PairRequest = serde_json::from_slice(&buf)?;

    let pin_ok = {
        let mut guard = inner.pairing.lock().expect("pairing lock");
        match guard.as_ref() {
            Some(s) if s.pin == req.pin && Instant::now() < s.expires => {
                *guard = None; // single-use
                true
            }
            _ => false,
        }
    };

    let reply = serde_json::to_vec(&PairReply {
        accepted: pin_ok,
        name: inner.desktop_name.clone(),
    })?;
    send.write_all(&reply).await?;
    let _ = send.finish();

    if pin_ok {
        let id = req.addr.id.to_string();
        upsert_peer(&inner, RemotePeer {
            id: id.clone(),
            name: req.name.clone(),
            token: req.token.clone(),
        });

        // Dial the phone back over the media ALPN using the address it gave us
        // (direct addrs enable hole-punching; the id alone would also work via
        // discovery). On success, hand the app the fresh loopback port.
        if let Ok(port) = connect_peer(&ep, &inner, req.addr).await {
            (inner.on_paired)(PairedPhone {
                id,
                name: req.name,
                token: req.token,
                port,
            });
        }
    }

    // Hold the pairing connection open until the phone has read the reply and
    // closed it (bounded), so the reply is reliably delivered before we drop it.
    let _ = tokio::time::timeout(Duration::from_secs(5), conn.closed()).await;
    Ok(())
}

/// Dial `addr` over [`ALPN_LINK`], open a loopback proxy, and remember it keyed
/// by the peer id. Returns the loopback port.
async fn connect_peer(ep: &Endpoint, inner: &Arc<Inner>, addr: EndpointAddr) -> Result<u16> {
    let id = addr.id.to_string();
    let proxy = open_proxy(ep, addr).await?;
    let port = proxy.port;
    inner.proxies.lock().expect("proxies lock").insert(id, proxy);
    Ok(port)
}

fn upsert_peer(inner: &Arc<Inner>, peer: RemotePeer) {
    let mut peers = inner.peers.lock().expect("peers lock");
    peers.retain(|p| p.id != peer.id);
    peers.push(peer);
    save_peers(&inner.store_path, &peers);
}

fn load_peers(path: &Path) -> Vec<RemotePeer> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_peers(path: &Path, peers: &[RemotePeer]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_vec_pretty(peers) {
        let _ = std::fs::write(path, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dial_pair, serve_tunnel, ALPN_LINK};
    use anyhow::anyhow;
    use iroh::SecretKey;
    use std::sync::mpsc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    /// End-to-end: a mock phone pairs with the manager (PIN over iroh), the
    /// manager dials it back + proxies, then an HTTP GET through the loopback
    /// port round-trips to the phone's shelf. Relay disabled → pure localhost.
    #[test]
    fn pairing_then_media_round_trips() -> Result<()> {
        let tag = crate::secret::random_pin();
        let secret_path = std::env::temp_dir().join(format!("hm_remote_sk_{tag}"));
        let store_path = std::env::temp_dir().join(format!("hm_remote_store_{tag}.json"));

        let (tx, rx) = mpsc::channel::<PairedPhone>();
        let manager = RemoteManager::new(
            secret_path.clone(),
            store_path.clone(),
            "Test Desktop".into(),
            false,
            move |p| {
                let _ = tx.send(p);
            },
        )?;
        let desktop_addr = manager.dial_addr();
        let info = manager.start_pairing(Duration::from_secs(30));

        // Mock phone on its own runtime; keep it alive for the whole test so the
        // dial-back + media tunnel keep working.
        let phone_rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        phone_rt.block_on(async {
            let shelf = TcpListener::bind(("127.0.0.1", 0)).await?;
            let shelf_port = shelf.local_addr()?.port();
            tokio::spawn(async move {
                while let Ok((mut s, _)) = shelf.accept().await {
                    tokio::spawn(async move {
                        let mut b = [0u8; 1024];
                        let _ = s.read(&mut b).await;
                        let _ = s
                            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello")
                            .await;
                        let _ = s.shutdown().await;
                    });
                }
            });

            let phone = build_endpoint(
                SecretKey::from_bytes(&[3u8; 32]),
                vec![ALPN_LINK.to_vec()],
                false,
            )
            .await?;
            let phone_serve = phone.clone();
            tokio::spawn(async move {
                let _ = serve_tunnel(phone_serve, shelf_port).await;
            });

            // Phone dials the desktop's pairing endpoint with the QR's pin,
            // exercising the same `dial_pair` the phone FFI uses.
            let desktop_name =
                dial_pair(&phone, desktop_addr, "My Phone", &info.pin, "tok-xyz").await?;
            assert_eq!(desktop_name, "Test Desktop", "pairing rejected");
            anyhow::Ok(())
        })?;

        // Desktop should have paired + dialed back, surfacing a loopback port.
        let paired = rx
            .recv_timeout(Duration::from_secs(20))
            .map_err(|e| anyhow!("no pairing callback: {e}"))?;
        assert_eq!(paired.name, "My Phone");
        assert_eq!(paired.token, "tok-xyz");

        // HTTP GET through the loopback proxy must reach the shelf via iroh.
        let media_rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        media_rt.block_on(async {
            let mut sock = TcpStream::connect(("127.0.0.1", paired.port)).await?;
            sock.write_all(b"GET /library HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
                .await?;
            let mut resp = Vec::new();
            sock.read_to_end(&mut resp).await?;
            let text = String::from_utf8_lossy(&resp);
            assert!(
                text.contains("200 OK") && text.contains("hello"),
                "bad response through tunnel: {text}"
            );
            anyhow::Ok(())
        })?;

        assert_eq!(manager.remote_phones().len(), 1, "peer not persisted");
        let _ = std::fs::remove_file(secret_path);
        let _ = std::fs::remove_file(store_path);
        Ok(())
    }
}
