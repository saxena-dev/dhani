//! Construction of the rustls `ClientConfig` used by the feeds.
//!
//! The configuration is built with an explicit aws-lc-rs provider and the webpki root store; the
//! process-wide default provider is never installed or relied on.

use std::sync::Arc;

use rustls::ClientConfig;
use tokio_tungstenite::Connector;

use super::FeedSpawnError;

/// The TLS client configuration for `wss://` feeds.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "used when a feed connects over wss://")
)]
pub(crate) fn client_config() -> Result<Arc<ClientConfig>, FeedSpawnError> {
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| FeedSpawnError::Tls(Box::new(e)))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// The tokio-tungstenite connector carrying [`client_config`].
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "used when a feed connects over wss://")
)]
pub(crate) fn connector() -> Result<Connector, FeedSpawnError> {
    Ok(Connector::Rustls(client_config()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rustls_connector_is_built_without_a_global_provider() {
        let config = client_config().unwrap();
        assert!(config.alpn_protocols.is_empty());
        assert!(matches!(connector().unwrap(), Connector::Rustls(_)));
    }
}
