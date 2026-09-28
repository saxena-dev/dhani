//! REST facade for the option chain and the expiry list.

use chrono::NaiveDate;

use super::json_body;
use crate::error::Result;
use crate::rest::endpoint;
use crate::rest::models::{ExpiryList, OptionChainEnvelope};
use crate::rest::transport::{Call, OptionChainKey};
use crate::rest::{DhanClient, OptionChainData, OptionChainRequest, UnderlyingRef};
use crate::types::RawJson;

/// The option chain of an underlying and its expiry list. Borrowed from a client with
/// [`DhanClient::option_chain`].
///
/// Dhan allows one chain call every three seconds for the same underlying and expiry; the
/// client's rate limiter waits for that window. Every call is read-only and follows the
/// client's [retry rules](crate::rest#retries).
pub struct OptionChain<'c> {
    client: &'c DhanClient,
}

impl<'c> OptionChain<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }

    async fn post_chain<T: serde::de::DeserializeOwned>(
        &self,
        req: &OptionChainRequest,
    ) -> Result<T> {
        let key = OptionChainKey {
            scrip: req.underlying.scrip,
            segment: req.underlying.segment,
            expiry: req.expiry,
        };
        self.client
            .execute(&endpoint::OPTION_CHAIN_CHAIN, || {
                req.validate()?;
                Ok(Call {
                    body: Some(json_body(req)?),
                    option_chain_key: Some(key),
                    ..Call::empty()
                })
            })
            .await
    }

    /// The option chain of one underlying and expiry: `POST /optionchain` (DOC:1200-1234).
    ///
    /// A Data-class query; calls with the same underlying, segment and expiry are also limited
    /// to one per three seconds (DOC:3236).
    pub async fn chain(&self, req: &OptionChainRequest) -> Result<OptionChainData> {
        let body: OptionChainEnvelope = self.post_chain(req).await?;
        Ok(body.data)
    }

    /// [`chain`](Self::chain), undecoded.
    pub async fn chain_raw(&self, req: &OptionChainRequest) -> Result<RawJson> {
        self.post_chain(req).await
    }

    /// The active expiries of an underlying: `POST /optionchain/expirylist` (DOC:857-897).
    ///
    /// A value that is not a calendar date is a decode error (chrono's parser also tolerates
    /// forms such as `2024-1-5`).
    pub async fn expiries(&self, underlying: &UnderlyingRef) -> Result<Vec<NaiveDate>> {
        let body: ExpiryList = self
            .client
            .execute(&endpoint::OPTION_CHAIN_EXPIRIES, || {
                underlying.validate()?;
                Ok(Call {
                    body: Some(json_body(underlying)?),
                    ..Call::empty()
                })
            })
            .await?;
        Ok(body.data)
    }
}
