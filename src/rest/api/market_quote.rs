//! REST facade for market quotes (§9 rows Q1–Q3).
//!
//! Every call is a read-only query in the Quote rate class, with a `JsonWithClientId` body: the
//! client ID is injected next to the segment keys, as the Python SDK sends it (Appendix A D42,
//! D44). Each typed method has a `*_raw` twin returning the same response undecoded.

use serde::de::DeserializeOwned;

use crate::error::Result;
use crate::rest::endpoint::{self, Endpoint};
use crate::rest::transport::Call;
use crate::rest::{DhanClient, FullQuote, LtpQuote, OhlcQuote, QuoteData, QuoteRequest};
use crate::types::RawJson;

/// The MarketQuote facade, borrowed from a [`DhanClient`].
pub struct MarketQuote<'c> {
    client: &'c DhanClient,
}

impl<'c> MarketQuote<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }

    async fn post<T: DeserializeOwned>(
        &self,
        ep: &'static Endpoint,
        req: &QuoteRequest,
    ) -> Result<T> {
        self.client
            .execute(ep, || {
                req.validate()?;
                Ok(Call {
                    body: Some(req.to_body()),
                    ..Call::empty()
                })
            })
            .await
    }

    /// Last traded prices: `POST /marketfeed/ltp` (DOC:1117-1158).
    pub async fn ltp(&self, req: &QuoteRequest) -> Result<QuoteData<LtpQuote>> {
        self.post(&endpoint::MARKET_QUOTE_LTP, req).await
    }

    /// Open, high, low, close and last price: `POST /marketfeed/ohlc` (DOC:1159-1199).
    pub async fn ohlc(&self, req: &QuoteRequest) -> Result<QuoteData<OhlcQuote>> {
        self.post(&endpoint::MARKET_QUOTE_OHLC, req).await
    }

    /// Full quotes with market depth: `POST /marketfeed/quote` (DOC:1529-1569).
    pub async fn quote(&self, req: &QuoteRequest) -> Result<QuoteData<FullQuote>> {
        self.post(&endpoint::MARKET_QUOTE_QUOTE, req).await
    }

    /// [`ltp`](Self::ltp), undecoded.
    pub async fn ltp_raw(&self, req: &QuoteRequest) -> Result<RawJson> {
        self.post(&endpoint::MARKET_QUOTE_LTP, req).await
    }

    /// [`ohlc`](Self::ohlc), undecoded.
    pub async fn ohlc_raw(&self, req: &QuoteRequest) -> Result<RawJson> {
        self.post(&endpoint::MARKET_QUOTE_OHLC, req).await
    }

    /// [`quote`](Self::quote), undecoded.
    pub async fn quote_raw(&self, req: &QuoteRequest) -> Result<RawJson> {
        self.post(&endpoint::MARKET_QUOTE_QUOTE, req).await
    }
}
