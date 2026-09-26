//! REST facade for auth (consent and partner flows).

use crate::rest::DhanClient;

/// The Auth facade, borrowed from a [`DhanClient`].
pub struct Auth<'c> {
    #[allow(dead_code, reason = "used by the endpoint methods of this group")]
    client: &'c DhanClient,
}

impl<'c> Auth<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }
}
