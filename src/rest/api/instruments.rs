//! REST facade for the instrument master CSVs (§9 rows I1–I2).

use crate::error::Result;
use crate::rest::endpoint::{self, Endpoint};
use crate::rest::models::parse_scrip_master;
use crate::rest::transport::Call;
use crate::rest::{DhanClient, InstrumentRecord, ScripMasterKind};

/// The Instruments facade, borrowed from a [`DhanClient`].
pub struct Instruments<'c> {
    client: &'c DhanClient,
}

impl<'c> Instruments<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }

    /// Downloads and parses a scrip master CSV from its absolute URL in the client's
    /// [`Urls`](crate::config::Urls) (DOC:5729, DOC:5735).
    ///
    /// The download sends no credentials, is not rate limited and is bounded by the CSV body
    /// limit. Parsing runs on the calling task and blocks its executor thread for the whole
    /// parse (a fraction of a second in release builds for the detailed file); a caller on a
    /// busy runtime may prefer to run this inside `tokio::task::spawn_blocking` or its own task.
    /// A malformed row is a `Decode` error whose detail names only the row (`row N`, counting
    /// data rows from 1).
    pub async fn scrip_master(&self, kind: ScripMasterKind) -> Result<Vec<InstrumentRecord>> {
        let ep: &'static Endpoint = match kind {
            ScripMasterKind::Compact => &endpoint::INSTRUMENTS_SCRIP_MASTER_COMPACT,
            ScripMasterKind::Detailed => &endpoint::INSTRUMENTS_SCRIP_MASTER_DETAILED,
        };
        self.client
            .execute_csv(
                ep,
                || Ok(Call::empty()),
                |text| parse_scrip_master(text).map_err(|bad| format!("row {}", bad.row)),
            )
            .await
    }
}
