//! REST facade for portfolio (holdings and positions).

use crate::rest::DhanClient;

/// The Portfolio facade, borrowed from a [`DhanClient`].
pub struct Portfolio<'c> {
    #[allow(dead_code, reason = "used by the endpoint methods of this group")]
    client: &'c DhanClient,
}

impl<'c> Portfolio<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }
}
