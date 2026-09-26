//! REST facade for trader's control (kill switch and P&L exit).

use crate::rest::DhanClient;

/// The TraderControl facade, borrowed from a [`DhanClient`].
pub struct TraderControl<'c> {
    #[allow(dead_code, reason = "used by the endpoint methods of this group")]
    client: &'c DhanClient,
}

impl<'c> TraderControl<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }
}
