//! REST facade for EDIS.

use crate::rest::DhanClient;

/// The Edis facade, borrowed from a [`DhanClient`].
pub struct Edis<'c> {
    #[allow(dead_code, reason = "used by the endpoint methods of this group")]
    client: &'c DhanClient,
}

impl<'c> Edis<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }
}
