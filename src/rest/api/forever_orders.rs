//! REST facade for forever orders.

use crate::rest::DhanClient;

/// Borrowed from a client with [`DhanClient::forever_orders`]. No calls yet: they arrive in a
/// later 0.x release.
pub struct ForeverOrders<'c> {
    #[allow(dead_code, reason = "used by the endpoint methods of this group")]
    client: &'c DhanClient,
}

impl<'c> ForeverOrders<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }
}
