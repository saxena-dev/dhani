//! REST facade for super orders.

use crate::rest::DhanClient;

/// Borrowed from a client with [`DhanClient::super_orders`]. No calls yet: they arrive in a later
/// 0.x release.
pub struct SuperOrders<'c> {
    #[allow(dead_code, reason = "used by the endpoint methods of this group")]
    client: &'c DhanClient,
}

impl<'c> SuperOrders<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }
}
