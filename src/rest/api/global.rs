//! REST facade for Global Stocks.

use crate::rest::DhanClient;

/// Borrowed from a client with [`DhanClient::global`]. No calls yet: they arrive in a later
/// 0.x release.
pub struct GlobalStocks<'c> {
    #[allow(dead_code, reason = "used by the endpoint methods of this group")]
    client: &'c DhanClient,
}

impl<'c> GlobalStocks<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }
}
