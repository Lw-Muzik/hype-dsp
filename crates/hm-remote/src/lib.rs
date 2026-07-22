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

use anyhow::{anyhow, Result};
use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr, SecretKey};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

pub mod ffi;
pub mod manager;
pub mod phone;
pub mod secret;

pub use manager::{PairedPhone, PairingInfo, RemoteManager, RemotePhoneStatus};
pub use phone::PhoneNode;

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
    build_endpoint_with(secret, alpns, relay, None).await
}

/// Like [`build_endpoint`], but with an optional custom DNS resolver.
///
/// When `dns` is `None` iroh constructs its default resolver, which reads the
/// SYSTEM DNS configuration. On Android that read goes over JNI via
/// `ndk_context` — which is only initialized by ndk-glue/android-activity or an
/// explicit `install_android_jni_context` call. In a library loaded through
/// dart:ffi's `dlopen` (no `JNI_OnLoad`, no glue) that lookup PANICS in release
/// builds, so the endpoint can never bind. The phone side therefore passes an
/// explicit public-nameserver resolver (see `phone::phone_dns_resolver`).
/// How long [`build_endpoint_with`] waits for `bind()` before giving up.
///
/// Binding is normally near-instant (sockets + background tasks), but it runs
/// OS-specific setup that has stalled in the field — and it happens lazily
/// behind a UI click, where an unbounded await reads as the button spinning
/// forever. Generous enough for a slow first netcheck; finite so the UI always
/// gets an answer.
const BIND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

pub async fn build_endpoint_with(
    secret: SecretKey,
    alpns: Vec<Vec<u8>>,
    relay: bool,
    dns: Option<iroh::dns::DnsResolver>,
) -> Result<Endpoint> {
    let mut builder = Endpoint::builder(iroh::endpoint::presets::N0)
        .secret_key(secret)
        .alpns(alpns);
    if let Some(dns) = dns {
        builder = builder.dns_resolver(dns);
    }
    if !relay {
        builder = builder.relay_mode(iroh::RelayMode::Disabled);
    }
    match tokio::time::timeout(BIND_TIMEOUT, builder.bind()).await {
        Ok(bound) => Ok(bound?),
        Err(_) => Err(anyhow!(
            "network endpoint setup timed out after {}s — check firewall/VPN and try again",
            BIND_TIMEOUT.as_secs()
        )),
    }
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

/// Phone side of pairing: dial the desktop's [`ALPN_PAIR`] endpoint (`desktop`
/// is the id from its QR; discovery resolves it on any network) and send our
/// `{addr, name, pin, token}`. Returns the desktop's name on acceptance. The
/// `token` is a shelf bearer token the phone has already authorised for this
/// desktop, so its media requests through the tunnel pass `/` auth.
pub async fn dial_pair(
    endpoint: &Endpoint,
    desktop: impl Into<EndpointAddr>,
    name: &str,
    pin: &str,
    token: &str,
) -> Result<String> {
    let own = endpoint_addr(endpoint).await?;
    let conn = endpoint
        .connect(desktop, ALPN_PAIR)
        .await
        .map_err(|e| anyhow!("couldn't reach the desktop: {e}"))?;
    let (mut send, mut recv) = conn.open_bi().await?;
    let req = serde_json::json!({ "addr": own, "name": name, "pin": pin, "token": token });
    send.write_all(&serde_json::to_vec(&req)?).await?;
    let _ = send.finish();
    let reply = recv.read_to_end(64 * 1024).await?;
    let reply: serde_json::Value = serde_json::from_slice(&reply)?;
    if reply.get("accepted").and_then(|v| v.as_bool()) == Some(true) {
        Ok(reply
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Desktop")
            .to_string())
    } else {
        Err(anyhow!("the desktop rejected the pairing code"))
    }
}

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

    /// Regression for the Android QR-pairing failure: an endpoint built with an
    /// EXPLICIT custom DNS resolver (the phone path — avoids the JNI-backed
    /// system-config read that panics on Android) must bind and carry traffic
    /// exactly like the default one.
    #[tokio::test]
    async fn endpoint_with_custom_dns_resolver_binds_and_connects() -> Result<()> {
        use iroh::dns::{DnsProtocol, DnsResolver};
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};

        let resolver = DnsResolver::builder()
            .with_nameserver(
                SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 53),
                DnsProtocol::Udp,
            )
            .build();
        let a = build_endpoint_with(
            SecretKey::from_bytes(&[11u8; 32]),
            vec![ALPN_LINK.to_vec()],
            false,
            Some(resolver),
        )
        .await?;
        let a_addr = endpoint_addr(&a).await?;
        // Accept one connection on `a`.
        let accept = tokio::spawn(async move {
            let incoming = a.accept().await.expect("closed before accept");
            let conn = incoming.await.expect("handshake");
            let (mut send, mut recv) = conn.accept_bi().await.expect("bi");
            let mut buf = [0u8; 4];
            recv.read_exact(&mut buf).await.expect("read");
            send.write_all(&buf).await.expect("echo");
            let _ = send.finish();
            // Hold the connection open until the dialer has read the echo.
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        });

        // Dial from a second custom-resolver endpoint (direct addrs, no DNS).
        let b = build_endpoint_with(
            SecretKey::from_bytes(&[13u8; 32]),
            vec![ALPN_LINK.to_vec()],
            false,
            Some(
                DnsResolver::builder()
                    .with_nameserver(
                        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 53),
                        DnsProtocol::Udp,
                    )
                    .build(),
            ),
        )
        .await?;
        let conn = b.connect(a_addr, ALPN_LINK).await?;
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(b"ping").await?;
        let _ = send.finish();
        let mut echoed = [0u8; 4];
        recv.read_exact(&mut echoed).await?;
        assert_eq!(&echoed, b"ping");
        accept.await.expect("accept task");
        Ok(())
    }
}
