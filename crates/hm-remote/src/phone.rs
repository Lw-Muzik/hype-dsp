//! [`PhoneNode`] — the **phone** side of the remote link, used through the FFI
//! by the Flutter app. It runs an iroh endpoint that:
//!
//! * **serves the media tunnel** ([`serve_tunnel`]) — desktops dial in and the
//!   node pipes their requests to the phone's local HTTP shelf, so the existing
//!   `/library`, `/stream`, … all work across networks; and
//! * **dials the desktop to pair** ([`dial_pair`]) when the user scans the
//!   desktop's QR.
//!
//! It owns its own tokio runtime and exposes a blocking API (the FFI is
//! synchronous); keep it alive for as long as sharing is enabled.

use anyhow::{anyhow, Result};
use iroh::{Endpoint, EndpointAddr, EndpointId};
use std::path::PathBuf;
use std::str::FromStr;

use crate::{build_endpoint_with, dial_pair, secret, serve_tunnel, ALPN_LINK};

/// DNS resolver for the phone endpoint.
///
/// **Android:** iroh's default resolver reads the system DNS config over JNI
/// via `ndk_context`, which is only initialized by ndk-glue/android-activity —
/// neither exists here: this library is loaded by dart:ffi's `dlopen`, which
/// never fires `JNI_OnLoad`. That JNI lookup **panics in release builds**
/// (see `iroh-dns/src/android.rs`); the panic was caught at our FFI boundary
/// and the endpoint never bound — which made QR pairing fail on every scan.
/// Explicit public resolvers (Cloudflare + Google, v4 + v6, plain UDP 53)
/// avoid JNI entirely; the phone only needs them to resolve n0's discovery
/// and relay hostnames.
///
/// **Other platforms** (iOS/macOS/tests): the system default works — keep it.
pub(crate) fn phone_dns_resolver() -> Option<iroh::dns::DnsResolver> {
    #[cfg(target_os = "android")]
    {
        use iroh::dns::{DnsProtocol, DnsResolver};
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
        const NAMESERVERS: [IpAddr; 4] = [
            IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), // Cloudflare
            IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), // Google
            IpAddr::V6(Ipv6Addr::new(0x2606, 0x4700, 0x4700, 0, 0, 0, 0, 0x1111)),
            IpAddr::V6(Ipv6Addr::new(0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888)),
        ];
        let mut builder = DnsResolver::builder();
        for ip in NAMESERVERS {
            builder = builder.with_nameserver(SocketAddr::new(ip, 53), DnsProtocol::Udp);
        }
        Some(builder.build())
    }
    #[cfg(not(target_os = "android"))]
    None
}

/// A running phone node: iroh endpoint + its runtime + the tunnel server task.
pub struct PhoneNode {
    runtime: tokio::runtime::Runtime,
    endpoint: Endpoint,
}

impl PhoneNode {
    /// Bind the endpoint with a stable identity from `secret_path` and start
    /// serving the media tunnel into the local shelf at `127.0.0.1:shelf_port`.
    /// Uses n0's relays + discovery so the phone is reachable by id anywhere.
    pub fn start(secret_path: PathBuf, shelf_port: u16) -> Result<Self> {
        // Install a process-level rustls CryptoProvider (ring) before any TLS
        // config is built. On Android, iroh/reqwest otherwise panic with
        // "Could not automatically determine the process-level CryptoProvider".
        // Idempotent: returns Err if one is already installed, which we ignore.
        let _ = rustls::crypto::ring::default_provider().install_default();

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        let secret = secret::load_or_create_secret(&secret_path)?;
        let endpoint = runtime.block_on(build_endpoint_with(
            secret,
            vec![ALPN_LINK.to_vec()],
            true,
            phone_dns_resolver(),
        ))?;
        crate::ffi::diag(&format!(
            "hm-remote: phone endpoint bound, id={}",
            endpoint.id()
        ));
        runtime.spawn(serve_tunnel(endpoint.clone(), shelf_port));
        Ok(Self { runtime, endpoint })
    }

    /// This phone's stable iroh id.
    pub fn endpoint_id(&self) -> String {
        self.endpoint.id().to_string()
    }

    /// Pair with the desktop scanned from its QR (`desktop_ep` = its endpoint id,
    /// `pin` = the QR's PIN). `token` is a shelf bearer token the caller has
    /// authorised for this desktop. Returns the desktop's name on success.
    /// Blocking — call off the UI isolate.
    pub fn pair(&self, desktop_ep: &str, pin: &str, name: &str, token: &str) -> Result<String> {
        let id = EndpointId::from_str(desktop_ep).map_err(|e| anyhow!("bad desktop id: {e}"))?;
        let ep = self.endpoint.clone();
        let (name, pin, token) = (name.to_string(), pin.to_string(), token.to_string());
        let (tx, rx) = std::sync::mpsc::channel();
        self.runtime.spawn(async move {
            let _ = tx.send(dial_pair(&ep, EndpointAddr::new(id), &name, &pin, &token).await);
        });
        rx.recv().map_err(|_| anyhow!("runtime dropped"))?
    }
}
