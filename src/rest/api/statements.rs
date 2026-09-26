//! REST facade for statements (ledger and trade history).

use crate::rest::DhanClient;

/// The Statements facade, borrowed from a [`DhanClient`].
pub struct Statements<'c> {
    #[allow(dead_code, reason = "used by the endpoint methods of this group")]
    client: &'c DhanClient,
}

impl<'c> Statements<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }
}
