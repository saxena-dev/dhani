//! REST facade for daily and intraday historical candles.

use super::json_body;
use crate::error::Result;
use crate::rest::endpoint;
use crate::rest::transport::Call;
use crate::rest::{Candles, DailyRequest, DhanClient, IntradayRequest};

/// Daily and intraday candles. Borrowed from a client with [`DhanClient::historical`].
///
/// Candles arrive as one array per field, index-aligned, in [`Candles`](crate::rest::Candles).
/// Every call is read-only and follows the client's [retry rules](crate::rest#retries).
pub struct Historical<'c> {
    client: &'c DhanClient,
}

impl<'c> Historical<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }

    /// Daily candles: `POST /charts/historical` (DOC:742-793). A read-only query in the Data
    /// rate class.
    pub async fn daily(&self, req: &DailyRequest) -> Result<Candles> {
        self.client
            .execute(&endpoint::HISTORICAL_DAILY, || {
                req.validate()?;
                Ok(Call {
                    body: Some(json_body(req)?),
                    ..Call::empty()
                })
            })
            .await
    }

    /// Intraday candles: `POST /charts/intraday` (DOC:982-1033). A read-only query in the Data
    /// rate class.
    pub async fn intraday(&self, req: &IntradayRequest) -> Result<Candles> {
        self.client
            .execute(&endpoint::HISTORICAL_INTRADAY, || {
                req.validate()?;
                Ok(Call {
                    body: Some(json_body(req)?),
                    ..Call::empty()
                })
            })
            .await
    }
}
