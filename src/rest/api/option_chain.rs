//! REST facade for option chain and expiry list.

use crate::rest::DhanClient;

/// The OptionChain facade, borrowed from a [`DhanClient`].
pub struct OptionChain<'c> {
    #[allow(dead_code, reason = "used by the endpoint methods of this group")]
    client: &'c DhanClient,
}

impl<'c> OptionChain<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }
}
