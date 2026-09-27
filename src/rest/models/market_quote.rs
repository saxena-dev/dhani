//! Request and response models for market quote.
//!
//! Request: [`QuoteRequest`] (DOC:2878-2892). Responses: [`QuoteData`] over [`LtpQuote`],
//! [`OhlcQuote`] and [`FullQuote`], typed from the OpenAPI schemas `LTPResponse`,
//! `OHLCResponse` and `QuoteResponse` (OQ-6), with snake_case wire names.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::error::{ValidationError, ValidationReason};
use crate::types::serde_ext::null_as_empty;
use crate::types::{ExchangeSegment, SecurityId, WireEnum, WireTime};

/// The most instruments one quote request may carry (DOC:2890). Larger sets are refused, never
/// split.
pub(crate) const MAX_QUOTE_INSTRUMENTS: usize = 1000;

/// The instruments to quote, grouped by segment (DOC:2878-2892).
///
/// On the wire each segment maps to an array of numeric security IDs, for example
/// `{"NSE_EQ": [11536], "NSE_FNO": [49081, 49082]}`, so every ID must be digits without leading
/// zeros; Global Stocks tickers cannot be quoted here. Adding the same instrument twice keeps
/// one. A request holds 1..=1000 instruments in total and is never split into several
/// requests.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QuoteRequest {
    instruments: BTreeMap<ExchangeSegment, BTreeSet<SecurityId>>,
}

impl QuoteRequest {
    /// An empty request.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one instrument.
    pub fn add(&mut self, segment: ExchangeSegment, security_id: SecurityId) -> &mut Self {
        self.instruments
            .entry(segment)
            .or_default()
            .insert(security_id);
        self
    }

    /// The number of distinct instruments.
    pub fn len(&self) -> usize {
        self.instruments.values().map(BTreeSet::len).sum()
    }

    /// Whether no instrument has been added.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Checks the instrument count and that every security ID is numeric, as the wire needs.
    pub fn validate(&self) -> Result<(), ValidationError> {
        let len = self.len();
        if len == 0 {
            return Err(ValidationError::new("instruments", ValidationReason::Empty));
        }
        if len > MAX_QUOTE_INSTRUMENTS {
            return Err(ValidationError::new(
                "instruments",
                ValidationReason::TooMany {
                    max: MAX_QUOTE_INSTRUMENTS,
                },
            ));
        }
        for id in self.instruments.values().flatten() {
            id.validate()?;
            if !id.as_ref().bytes().all(|b| b.is_ascii_digit()) {
                return Err(ValidationError::new(
                    "security_id",
                    ValidationReason::InvalidCharacters,
                ));
            }
            if id.as_ref().len() > 1 && id.as_ref().starts_with('0') {
                // "011536" and "11536" are the same integer on the wire.
                return Err(ValidationError::new(
                    "security_id",
                    ValidationReason::Inconsistent("leading zeros are lost on the wire"),
                ));
            }
            if id.as_numeric().is_none() {
                return Err(ValidationError::new(
                    "security_id",
                    ValidationReason::OutOfRange,
                ));
            }
        }
        Ok(())
    }

    /// The request body; call after [`validate`](Self::validate), which guarantees every ID is
    /// numeric.
    pub(crate) fn to_body(&self) -> serde_json::Value {
        let object = self
            .instruments
            .iter()
            .filter(|(_, ids)| !ids.is_empty())
            .map(|(segment, ids)| {
                let ids: Vec<serde_json::Value> = ids
                    .iter()
                    .filter_map(SecurityId::as_numeric)
                    .map(serde_json::Value::from)
                    .collect();
                (segment.as_wire().to_owned(), serde_json::Value::Array(ids))
            })
            .collect();
        serde_json::Value::Object(object)
    }
}

/// A quote response: `{"status": .., "data": {"<SEGMENT>": {"<securityId>": T}}}`.
///
/// The OpenAPI schemas show one level under `data`, which cannot carry both the segment and the
/// ID; the published example nests two (Appendix A D59, OQ-31).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub struct QuoteData<T> {
    /// The response status, such as `success`.
    #[serde(default)]
    pub status: Option<String>,
    /// Quotes by segment wire name, then by security ID; `null` is empty.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub data: BTreeMap<String, BTreeMap<String, T>>,
}

impl<T> QuoteData<T> {
    /// The quote for one instrument, if present.
    pub fn get(&self, segment: ExchangeSegment, security_id: &SecurityId) -> Option<&T> {
        self.data.get(segment.as_wire())?.get(security_id.as_ref())
    }
}

/// A last-traded-price quote (`LTPResponse`).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LtpQuote {
    /// The last traded price.
    #[serde(default)]
    pub last_price: Option<f64>,
}

/// An open-high-low-close quote (`OHLCResponse`).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OhlcQuote {
    /// The last traded price.
    #[serde(default)]
    pub last_price: Option<f64>,
    /// The day's open, high, low and close.
    #[serde(default)]
    pub ohlc: Option<Ohlc>,
}

/// Open, high, low and close prices.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Ohlc {
    /// The open.
    #[serde(default)]
    pub open: Option<f64>,
    /// The high.
    #[serde(default)]
    pub high: Option<f64>,
    /// The low.
    #[serde(default)]
    pub low: Option<f64>,
    /// The close.
    #[serde(default)]
    pub close: Option<f64>,
}

/// A full quote with market depth (`QuoteResponse`).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FullQuote {
    /// The volume-weighted average price.
    #[serde(default)]
    pub average_price: Option<f64>,
    /// The total pending buy quantity.
    #[serde(default)]
    pub buy_quantity: Option<i64>,
    /// The total pending sell quantity.
    #[serde(default)]
    pub sell_quantity: Option<i64>,
    /// The last traded price.
    #[serde(default)]
    pub last_price: Option<f64>,
    /// The last traded quantity.
    #[serde(default)]
    pub last_quantity: Option<i64>,
    /// When the last trade happened.
    #[serde(default)]
    pub last_trade_time: Option<WireTime>,
    /// The lower circuit limit.
    #[serde(default)]
    pub lower_circuit_limit: Option<f64>,
    /// The upper circuit limit.
    #[serde(default)]
    pub upper_circuit_limit: Option<f64>,
    /// The change from the previous close.
    #[serde(default)]
    pub net_change: Option<f64>,
    /// The day's volume.
    #[serde(default)]
    pub volume: Option<i64>,
    /// The open interest.
    #[serde(default)]
    pub oi: Option<i64>,
    /// The day's highest open interest (in the published example, not the OpenAPI schema).
    #[serde(default)]
    pub oi_day_high: Option<i64>,
    /// The day's lowest open interest (in the published example, not the OpenAPI schema).
    #[serde(default)]
    pub oi_day_low: Option<i64>,
    /// The day's open, high, low and close (in the published example and DOC:1533, not the
    /// OpenAPI schema).
    #[serde(default)]
    pub ohlc: Option<Ohlc>,
    /// The market depth.
    #[serde(default)]
    pub depth: Option<QuoteDepth>,
}

/// Bid and ask levels of a full quote.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct QuoteDepth {
    /// Bid levels, best first; `null` is empty.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub buy: Vec<QuoteLevel>,
    /// Ask levels, best first; `null` is empty.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub sell: Vec<QuoteLevel>,
}

/// One depth level.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct QuoteLevel {
    /// The quantity at this price.
    #[serde(default)]
    pub quantity: Option<i64>,
    /// The price.
    #[serde(default)]
    pub price: Option<f64>,
    /// The number of orders at this price.
    #[serde(default)]
    pub orders: Option<i64>,
}

#[cfg(test)]
#[path = "market_quote_tests.rs"]
mod tests;
