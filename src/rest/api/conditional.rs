//! REST facade for conditional and multi orders.

use crate::rest::DhanClient;

/// Borrowed from a client with [`DhanClient::conditional`]. No calls yet: they arrive in a later
/// 0.x release.
pub struct ConditionalOrders<'c> {
    #[allow(dead_code, reason = "used by the endpoint methods of this group")]
    client: &'c DhanClient,
}

impl<'c> ConditionalOrders<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }
}
