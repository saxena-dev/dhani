//! REST facade for market quote.

use crate::rest::DhanClient;

/// The MarketQuote facade, borrowed from a [`DhanClient`].
pub struct MarketQuote<'c> {
    #[allow(dead_code, reason = "used by the endpoint methods of this group")]
    client: &'c DhanClient,
}

impl<'c> MarketQuote<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }
}
