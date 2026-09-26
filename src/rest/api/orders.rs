//! REST facade for orders and trades.

use crate::rest::DhanClient;

/// The Orders facade, borrowed from a [`DhanClient`].
pub struct Orders<'c> {
    #[allow(dead_code, reason = "used by the endpoint methods of this group")]
    client: &'c DhanClient,
}

impl<'c> Orders<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }
}
