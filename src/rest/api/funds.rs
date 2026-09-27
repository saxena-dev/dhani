//! REST facade for fund limits and the margin calculators (§9 rows M1–M3).

use super::json_body;
use crate::error::Result;
use crate::rest::endpoint;
use crate::rest::transport::Call;
use crate::rest::{DhanClient, FundLimits, Margin, MarginRequest, MultiMargin, MultiMarginRequest};

/// The Funds facade, borrowed from a [`DhanClient`].
pub struct Funds<'c> {
    client: &'c DhanClient,
}

impl<'c> Funds<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }

    /// The account's balances and limits: `GET /fundlimit` (DOC:899-937).
    pub async fn limits(&self) -> Result<FundLimits> {
        self.client
            .execute(&endpoint::FUNDS_LIMITS, || Ok(Call::empty()))
            .await
    }

    /// The margin one order needs: `POST /margincalculator` (DOC:10-72). A read-only query,
    /// so it is retried like a read.
    pub async fn margin(&self, req: &MarginRequest) -> Result<Margin> {
        self.client
            .execute(&endpoint::FUNDS_MARGIN, || {
                req.validate()?;
                Ok(Call {
                    body: Some(json_body(req)?),
                    ..Call::empty()
                })
            })
            .await
    }

    /// The combined margin of several orders: `POST /margincalculator/multi` (DOC:73-130).
    /// A read-only query, so it is retried like a read.
    pub async fn margin_multi(&self, req: &MultiMarginRequest) -> Result<MultiMargin> {
        self.client
            .execute(&endpoint::FUNDS_MARGIN_MULTI, || {
                req.validate()?;
                Ok(Call {
                    body: Some(json_body(req)?),
                    ..Call::empty()
                })
            })
            .await
    }
}
