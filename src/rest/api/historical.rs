//! REST facade for historical data.

use crate::rest::DhanClient;

/// The Historical facade, borrowed from a [`DhanClient`].
pub struct Historical<'c> {
    #[allow(dead_code, reason = "used by the endpoint methods of this group")]
    client: &'c DhanClient,
}

impl<'c> Historical<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }
}
