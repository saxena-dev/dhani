//! REST facade for forever orders.

use crate::rest::DhanClient;

/// The ForeverOrders facade, borrowed from a [`DhanClient`].
pub struct ForeverOrders<'c> {
    #[allow(dead_code, reason = "used by the endpoint methods of this group")]
    client: &'c DhanClient,
}

impl<'c> ForeverOrders<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }
}
