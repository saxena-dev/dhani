//! Request and response models for the option chain and the expiry list.
//!
//! Requests: [`OptionChainRequest`] (DOC:1211-1215) and [`UnderlyingRef`] (DOC:868-871), with
//! PascalCase wire keys. Response: [`OptionChainData`], typed from the OpenAPI schemas
//! `OptionChainResponse` and `OptionData`.

use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::error::{ValidationError, ValidationReason};
use crate::types::ExchangeSegment;
use crate::types::serde_ext::{null_as_empty, null_values_default};

/// The underlying of an option chain: its security ID and segment (DOC:868-871).
///
/// The security ID is sent as a JSON integer (DOC:870, DOC:1213).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct UnderlyingRef {
    /// The underlying's security ID; wire `UnderlyingScrip`.
    #[serde(rename = "UnderlyingScrip")]
    pub scrip: u32,
    /// The underlying's segment; wire `UnderlyingSeg`.
    #[serde(rename = "UnderlyingSeg")]
    pub segment: ExchangeSegment,
}

impl UnderlyingRef {
    /// The underlying `scrip` in `segment`.
    pub fn new(scrip: u32, segment: ExchangeSegment) -> Self {
        Self { scrip, segment }
    }

    /// The security ID must be positive, and the segment is not Global Stocks (`InxEq`).
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.segment == ExchangeSegment::InxEq {
            return Err(ValidationError::new(
                "segment",
                ValidationReason::UnknownEnumValue,
            ));
        }
        if self.scrip == 0 {
            return Err(ValidationError::new("scrip", ValidationReason::NotPositive));
        }
        Ok(())
    }
}

/// The option chain of one underlying for one expiry (DOC:1211-1215).
///
/// Active expiries come from the expiry list. Chain calls with the same underlying, segment and
/// expiry are limited to one per three seconds (DOC:3236).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct OptionChainRequest {
    /// The underlying.
    #[serde(flatten)]
    pub underlying: UnderlyingRef,
    /// The expiry; wire `Expiry`, `YYYY-MM-DD`.
    #[serde(rename = "Expiry")]
    pub expiry: NaiveDate,
}

impl OptionChainRequest {
    /// The chain of `underlying` for `expiry`.
    pub fn new(underlying: UnderlyingRef, expiry: NaiveDate) -> Self {
        Self { underlying, expiry }
    }

    /// Checks the underlying and that the expiry formats as `YYYY-MM-DD`.
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.underlying.validate()?;
        if !(1..=9999).contains(&self.expiry.year()) {
            return Err(ValidationError::new("expiry", ValidationReason::OutOfRange));
        }
        Ok(())
    }
}

/// An option chain (`OptionChainResponse`): the underlying's last price and one row per strike.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OptionChainData {
    /// The underlying's last traded price.
    #[serde(default)]
    pub last_price: Option<f64>,
    /// Rows by strike, keyed by the strike exactly as sent (its format is not specified); a
    /// `null` row is empty.
    #[serde(default, deserialize_with = "null_values_default")]
    pub oc: BTreeMap<String, StrikeRow>,
}

impl OptionChainData {
    /// The rows with their strikes parsed as numbers, in ascending strike order; rows whose key
    /// is not a number are skipped.
    pub fn strikes(&self) -> Vec<(f64, &StrikeRow)> {
        let mut rows: Vec<(f64, &StrikeRow)> = self
            .oc
            .iter()
            .filter_map(|(key, row)| {
                key.trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|s| s.is_finite())
                    .map(|s| (s, row))
            })
            .collect();
        rows.sort_by(|a, b| a.0.total_cmp(&b.0));
        rows
    }
}

/// The call and put at one strike.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct StrikeRow {
    /// The call.
    #[serde(default)]
    pub ce: Option<OptionData>,
    /// The put.
    #[serde(default)]
    pub pe: Option<OptionData>,
}

/// One option in the chain (`OptionData`).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OptionData {
    /// The volume-weighted average price.
    #[serde(default)]
    pub average_price: Option<f64>,
    /// The option greeks.
    #[serde(default)]
    pub greeks: Option<Greeks>,
    /// The implied volatility.
    #[serde(default)]
    pub implied_volatility: Option<f64>,
    /// The last traded price.
    #[serde(default)]
    pub last_price: Option<f64>,
    /// The open interest.
    #[serde(default)]
    pub oi: Option<i64>,
    /// The option's security ID.
    #[serde(default)]
    pub security_id: Option<u32>,
    /// The best bid price.
    #[serde(default)]
    pub top_bid_price: Option<f64>,
    /// The best bid quantity.
    #[serde(default)]
    pub top_bid_quantity: Option<i64>,
    /// The best ask price.
    #[serde(default)]
    pub top_ask_price: Option<f64>,
    /// The best ask quantity.
    #[serde(default)]
    pub top_ask_quantity: Option<i64>,
    /// The day's volume.
    #[serde(default)]
    pub volume: Option<i64>,
}

/// Option greeks.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Greeks {
    /// Delta.
    #[serde(default)]
    pub delta: Option<f64>,
    /// Theta.
    #[serde(default)]
    pub theta: Option<f64>,
    /// Gamma.
    #[serde(default)]
    pub gamma: Option<f64>,
    /// Vega.
    #[serde(default)]
    pub vega: Option<f64>,
}

/// The option-chain body: `{"data": {..}}`.
#[derive(Debug, Deserialize)]
pub(crate) struct OptionChainEnvelope {
    pub(crate) data: OptionChainData,
}

/// The expiry-list body: `{"data": ["YYYY-MM-DD", ..]}` (DOC:878); other keys are ignored and
/// a value that is not a calendar date fails the decode.
#[derive(Debug, Deserialize)]
pub(crate) struct ExpiryList {
    #[serde(default, deserialize_with = "null_as_empty")]
    pub(crate) data: Vec<NaiveDate>,
}

#[cfg(test)]
#[path = "option_chain_tests.rs"]
mod tests;
