//! Remote Phone Link over **iroh** (QUIC with NAT hole-punching + relay
//! fallback) — lets the desktop reach a phone's music library across *different*
//! networks, not just the same LAN.
//!
//! ## Transparent TCP-over-QUIC tunnel
//!
//! The phone already runs an HTTP **shelf** server (`/library`, `/stream` with
//! Range, `/art`, `/pair`, …). Rather than re-implement any of that over iroh,
//! we tunnel raw TCP bytes over an iroh bi-directional stream:
//!
//! ```text
//! reqwest (desktop) → 127.0.0.1:proxyPort → iroh bi-stream ══QUIC══►
//!                                          → 127.0.0.1:shelfPort (phone) → shelf
//! ```
//!
//! Because QUIC streams are reliable + ordered, HTTP/1.1 (Range, keep-alive)
//! rides through untouched. A remote phone therefore looks to the rest of the
//! app exactly like a LAN phone reachable at `127.0.0.1:<proxyPort>`, so every
//! existing `hm-link` call works unchanged.
//!
//! This crate is shared: the **desktop** links it directly; the **phone** links
//! it via `flutter_rust_bridge` (it runs [`serve_tunnel`] alongside its shelf).

use anyhow::Result;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr, SecretKey};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

pub mod manager;
pub mod secret;

pub use manager::{PairedPhone, PairingInfo, RemoteManager, RemotePhoneStatus};

/// ALPN for the media tunnel (desktop ⇄ phone shelf).
pub const ALPN_LINK: &[u8] = b"hypemuzik/link/0";
/// ALPN for the pairing control stream (added in M2).
pub const ALPN_PAIR: &[u8] = b"hypemuzik/pair/0";

/// Build + bind an iroh endpoint with a stable identity (`secret`) and the given
/// ALPNs. `relay = true` uses n0's default relays + discovery (so a peer can be
/// reached anywhere by id); `false` disables relays (direct addresses only —
/// used by the in-process tests).
pub async fn build_endpoint(
    secret: SecretKey,
    alpns: Vec<Vec<u8>>,
    relay: bool,
) -> Result<Endpoint> {
    let mut builder = Endpoint::builder(iroh::endpoint::presets::N0)
        .secret_key(secret)
        .alpns(alpns);
    if !relay {
        builder = builder.relay_mode(iroh::RelayMode::Disabled);
    }
    Ok(builder.bind().await?)
}

/// This endpoint's own dialable address (id + relay/direct addresses), e.g. to
/// hand to a peer during pairing. Polls briefly until at least one direct/relay
/// address is known so the returned address is actually dialable.
pub async fn endpoint_addr(ep: &Endpoint) -> Result<EndpointAddr> {
    for _ in 0..60 {
        let addr = ep.addr();
        if addr.ip_addrs().next().is_some() {
            return Ok(addr);
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Ok(ep.addr())
}

// ----------------------------------------------------------------- phone side

/// Phone side: accept incoming tunnel connections and pipe each bi-stream to the
/// local shelf server at `127.0.0.1:shelf_port`. Runs until the endpoint closes.
pub async fn serve_tunnel(ep: Endpoint, shelf_port: u16) -> Result<()> {
    while let Some(incoming) = ep.accept().await {
        tokio::spawn(async move {
            let Ok(conn) = incoming.await else { return };
            serve_connection(conn, shelf_port).await;
        });
    }
    Ok(())
}

/// Handle one peer connection: every bi-stream it opens becomes a proxied TCP
/// connection to the shelf.
async fn serve_connection(conn: Connection, shelf_port: u16) {
    // Each bi-stream the peer opens becomes a proxied TCP connection to the
    // shelf; the loop ends when the peer closes the connection (or it errors).
    while let Ok((send, recv)) = conn.accept_bi().await {
        tokio::spawn(stream_to_shelf(send, recv, shelf_port));
    }
}

/// Pipe one iroh bi-stream ⇄ a fresh TCP connection to the local shelf.
async fn stream_to_shelf(send: SendStream, recv: RecvStream, shelf_port: u16) {
    let Ok(tcp) = TcpStream::connect(("127.0.0.1", shelf_port)).await else {
        return;
    };
    pipe(tcp, send, recv).await;
}

// --------------------------------------------------------------- desktop side

/// A running loopback proxy for one remote phone. While alive, connecting to
/// `127.0.0.1:port` tunnels to the phone's shelf over iroh. Dropping it stops
/// the proxy (the iroh connection is kept by `_conn` until then).
pub struct RemoteProxy {
    pub port: u16,
    task: tokio::task::JoinHandle<()>,
    _conn: Arc<Connection>,
}

impl Drop for RemoteProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Desktop side: dial `target` (an [`EndpointAddr`] — from a stored id, or a full
/// address during pairing/tests) and start a loopback TCP listener whose every
/// connection is tunneled over a fresh bi-stream to the phone. Returns the bound
/// loopback port via [`RemoteProxy`].
pub async fn open_proxy(ep: &Endpoint, target: impl Into<EndpointAddr>) -> Result<RemoteProxy> {
    let conn = ep.connect(target, ALPN_LINK).await?;
    let conn = Arc::new(conn);
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();

    let conn_for_task = conn.clone();
    let task = tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                break;
            };
            let conn = conn_for_task.clone();
            tokio::spawn(async move {
                let Ok((send, recv)) = conn.open_bi().await else {
                    return;
                };
                pipe(tcp, send, recv).await;
            });
        }
    });

    Ok(RemoteProxy {
        port,
        task,
        _conn: conn,
    })
}

// -------------------------------------------------------------------- copying

/// Bidirectionally copy between a TCP socket and an iroh bi-stream until both
/// directions hit EOF, signalling end-of-stream on the iroh send side so the
/// other end sees the request/response boundary.
async fn pipe(tcp: TcpStream, mut send: SendStream, mut recv: RecvStream) {
    let (mut tcp_r, mut tcp_w) = tcp.into_split();
    let up = async move {
        let _ = tokio::io::copy(&mut tcp_r, &mut send).await;
        let _ = send.finish();
    };
    let down = async move {
        let _ = tokio::io::copy(&mut recv, &mut tcp_w).await;
        let _ = tcp_w.shutdown().await;
    };
    let _ = tokio::join!(up, down);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    /// Full round-trip: a dummy shelf, a phone endpoint serving the tunnel, a
    /// desktop endpoint opening a loopback proxy, and an HTTP GET that must reach
    /// the shelf *through iroh* and come back. Relay disabled → pure localhost.
    #[tokio::test]
    async fn http_round_trips_over_iroh_tunnel() -> Result<()> {
        // 1. Dummy "shelf": replies 200 hello to any request.
        let shelf = TcpListener::bind(("127.0.0.1", 0)).await?;
        let shelf_port = shelf.local_addr()?.port();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = shelf.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let _ = sock.read(&mut buf).await;
                    let _ = sock
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
                        )
                        .await;
                    let _ = sock.shutdown().await;
                });
            }
        });

        // 2. Phone endpoint → serve the tunnel into the shelf.
        let phone = build_endpoint(SecretKey::from_bytes(&[7u8; 32]), vec![ALPN_LINK.to_vec()], false)
            .await?;
        let phone_addr = endpoint_addr(&phone).await?;
        let phone_serve = phone.clone();
        tokio::spawn(async move {
            let _ = serve_tunnel(phone_serve, shelf_port).await;
        });

        // 3. Desktop endpoint → dial the phone + loopback proxy.
        let desktop =
            build_endpoint(SecretKey::from_bytes(&[9u8; 32]), vec![ALPN_LINK.to_vec()], false)
                .await?;
        let proxy = open_proxy(&desktop, phone_addr).await?;

        // 4. HTTP GET the loopback proxy — must round-trip through iroh.
        let mut sock = TcpStream::connect(("127.0.0.1", proxy.port)).await?;
        sock.write_all(b"GET /ping HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .await?;
        let mut resp = Vec::new();
        sock.read_to_end(&mut resp).await?;
        let text = String::from_utf8_lossy(&resp);
        assert!(text.contains("200 OK"), "expected 200, got: {text}");
        assert!(text.contains("hello"), "expected body, got: {text}");
        Ok(())
    }
}
