//! REST facade for super orders.

use crate::rest::DhanClient;

/// The SuperOrders facade, borrowed from a [`DhanClient`].
pub struct SuperOrders<'c> {
    #[allow(dead_code, reason = "used by the endpoint methods of this group")]
    client: &'c DhanClient,
}

impl<'c> SuperOrders<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }
}
