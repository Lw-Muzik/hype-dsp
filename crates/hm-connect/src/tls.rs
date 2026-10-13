//! TLS for `wss://` hubs.
//!
//! The connector is built explicitly instead of letting tokio-tungstenite call
//! `rustls::ClientConfig::builder()`: in the desktop process rustls is
//! compiled with BOTH ring (via iroh) and aws-lc-rs (via reqwest), and the
//! implicit builder panics when it can't pick a process-wide provider. Naming
//! ring here sidesteps that, and the platform verifier checks the hub's
//! certificate exactly as the OS (and reqwest) would.

use std::sync::Arc;

use rustls_platform_verifier::BuilderVerifierExt;
use tokio_tungstenite::Connector;

pub fn connector() -> Result<Connector, rustls::Error> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_platform_verifier()?
        .with_no_client_auth();
    Ok(Connector::Rustls(Arc::new(config)))
}

#[cfg(test)]
mod tests {
    #[test]
    fn builds_without_a_process_default_provider() {
        // Must not panic even though no CryptoProvider was installed.
        assert!(super::connector().is_ok());
    }
}
