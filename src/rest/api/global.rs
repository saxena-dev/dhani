//! REST facade for Global Stocks.

use crate::rest::DhanClient;

/// The GlobalStocks facade, borrowed from a [`DhanClient`].
pub struct GlobalStocks<'c> {
    #[allow(dead_code, reason = "used by the endpoint methods of this group")]
    client: &'c DhanClient,
}

impl<'c> GlobalStocks<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }
}
