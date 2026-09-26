//! REST facade for funds and margin.

use crate::rest::DhanClient;

/// The Funds facade, borrowed from a [`DhanClient`].
pub struct Funds<'c> {
    #[allow(dead_code, reason = "used by the endpoint methods of this group")]
    client: &'c DhanClient,
}

impl<'c> Funds<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }
}
