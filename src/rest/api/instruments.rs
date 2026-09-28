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
    /// limit. Parsing runs on Tokio's blocking pool, not on the calling task. A lot size, strike
    /// or tick size cell that is not a number reads as `None`, its text kept in `extra`.
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
