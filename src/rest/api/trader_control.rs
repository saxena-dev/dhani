//! REST facade for trader's control (kill switch and P&L exit).

use crate::rest::DhanClient;

/// Borrowed from a client with [`DhanClient::trader_control`]. No calls yet: they arrive in a
/// later 0.x release.
pub struct TraderControl<'c> {
    #[allow(dead_code, reason = "used by the endpoint methods of this group")]
    client: &'c DhanClient,
}

impl<'c> TraderControl<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }
}
