//! REST facade for EDIS.

use crate::rest::DhanClient;

/// Borrowed from a client with [`DhanClient::edis`]. No calls yet: they arrive in a later
/// 0.x release.
pub struct Edis<'c> {
    #[allow(dead_code, reason = "used by the endpoint methods of this group")]
    client: &'c DhanClient,
}

impl<'c> Edis<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }
}
