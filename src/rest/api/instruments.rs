//! REST facade for the instrument master (CSV).

use crate::rest::DhanClient;

/// The Instruments facade, borrowed from a [`DhanClient`].
pub struct Instruments<'c> {
    #[allow(dead_code, reason = "used by the endpoint methods of this group")]
    client: &'c DhanClient,
}

impl<'c> Instruments<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }
}
