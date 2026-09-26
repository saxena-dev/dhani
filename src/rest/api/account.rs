//! REST facade for account (token renewal, profile and static IP).

use crate::rest::DhanClient;

/// The Account facade, borrowed from a [`DhanClient`].
pub struct Account<'c> {
    #[allow(dead_code, reason = "used by the endpoint methods of this group")]
    client: &'c DhanClient,
}

impl<'c> Account<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }
}
