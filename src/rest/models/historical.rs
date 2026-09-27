//! Request and response models for daily and intraday historical candles.
//!
//! Requests: [`DailyRequest`] (DOC:753-761) and [`IntradayRequest`] (DOC:993-1001). Response:
//! [`Candles`], the columnar arrays of DOC:766-774 and DOC:1006-1014
//! (`OAS:#/components/schemas/HistoricalResponse`).

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{ValidationError, ValidationReason};
use crate::types::serde_ext::{LenientInt, null_as_empty};
use crate::types::{ExchangeSegment, ExpiryCode, InstrumentKind, SecurityId, epoch_to_ist};

/// The segments the chart endpoints accept (DOC:756, DOC:996).
fn chartable(segment: ExchangeSegment) -> bool {
    matches!(
        segment,
        ExchangeSegment::NseEq
            | ExchangeSegment::NseFno
            | ExchangeSegment::BseEq
            | ExchangeSegment::BseFno
            | ExchangeSegment::McxComm
            | ExchangeSegment::IdxI
    )
}

/// The checks both chart requests share: the segment, the security ID and dates that format as
/// `YYYY-MM-DD`.
fn check_common(
    segment: ExchangeSegment,
    security_id: &SecurityId,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<(), ValidationError> {
    if !chartable(segment) {
        return Err(ValidationError::new(
            "exchange_segment",
            ValidationReason::UnknownEnumValue,
        ));
    }
    security_id.validate()?;
    for (field, date) in [("from_date", from), ("to_date", to)] {
        if !(1..=9999).contains(&date.year()) {
            return Err(ValidationError::new(field, ValidationReason::OutOfRange));
        }
    }
    Ok(())
}

/// Daily candles for one instrument (DOC:753-761).
///
/// `to_date` is not inclusive (DOC:761), so the range must hold at least one day. `oi` is sent
/// as a JSON boolean (Appendix A D6); `expiry_code` only when set (Appendix A D7).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyRequest {
    /// The instrument, sent as a string.
    pub security_id: SecurityId,
    /// The segment: `NseEq`, `NseFno`, `BseEq`, `BseFno`, `McxComm` or `IdxI`.
    pub exchange_segment: ExchangeSegment,
    /// The instrument kind.
    pub instrument: InstrumentKind,
    /// The expiry of a derivative.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiry_code: Option<ExpiryCode>,
    /// Whether to include open interest.
    pub oi: bool,
    /// The first day, `YYYY-MM-DD`.
    pub from_date: NaiveDate,
    /// The day after the last, `YYYY-MM-DD` (not inclusive).
    pub to_date: NaiveDate,
}

impl DailyRequest {
    /// A request without open interest or an expiry.
    pub fn new(
        exchange_segment: ExchangeSegment,
        security_id: SecurityId,
        instrument: InstrumentKind,
        from_date: NaiveDate,
        to_date: NaiveDate,
    ) -> Self {
        Self {
            security_id,
            exchange_segment,
            instrument,
            expiry_code: None,
            oi: false,
            from_date,
            to_date,
        }
    }

    /// Sets the derivative expiry.
    pub fn with_expiry_code(mut self, expiry_code: ExpiryCode) -> Self {
        self.expiry_code = Some(expiry_code);
        self
    }

    /// Includes open interest.
    pub fn with_oi(mut self, oi: bool) -> Self {
        self.oi = oi;
        self
    }

    /// Checks the segment, the security ID and that `from_date` is before `to_date`.
    pub fn validate(&self) -> Result<(), ValidationError> {
        check_common(
            self.exchange_segment,
            &self.security_id,
            self.from_date,
            self.to_date,
        )?;
        if self.from_date >= self.to_date {
            return Err(ValidationError::new(
                "from_date",
                ValidationReason::Inconsistent("must be before to_date (to_date is not inclusive)"),
            ));
        }
        Ok(())
    }
}

/// The candle interval of an intraday request, in minutes, sent as a JSON integer
/// (`OAS:#/components/schemas/IntradayHistoricalRequest`).
///
/// The values follow the OpenAPI spec, the guide (DOC:5411) and the Python SDK, which list 25;
/// the endpoint table's 30 (DOC:998) is not used (Appendix A D8).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntradayInterval {
    /// One minute.
    Min1,
    /// Five minutes.
    Min5,
    /// Fifteen minutes.
    Min15,
    /// Twenty-five minutes.
    Min25,
    /// Sixty minutes.
    Min60,
}

impl IntradayInterval {
    /// The interval in minutes, as sent.
    pub fn minutes(self) -> u8 {
        match self {
            Self::Min1 => 1,
            Self::Min5 => 5,
            Self::Min15 => 15,
            Self::Min25 => 25,
            Self::Min60 => 60,
        }
    }
}

impl Serialize for IntradayInterval {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u8(self.minutes())
    }
}

/// Intraday candles for one instrument (DOC:993-1001).
///
/// The interval is required and has no default (Appendix A D10). The endpoint table does not say
/// `to_date` is exclusive, so a single day (`from_date == to_date`) is allowed.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntradayRequest {
    /// The instrument, sent as a string.
    pub security_id: SecurityId,
    /// The segment: `NseEq`, `NseFno`, `BseEq`, `BseFno`, `McxComm` or `IdxI`.
    pub exchange_segment: ExchangeSegment,
    /// The instrument kind.
    pub instrument: InstrumentKind,
    /// The candle interval.
    pub interval: IntradayInterval,
    /// Whether to include open interest.
    pub oi: bool,
    /// The first day, `YYYY-MM-DD`.
    pub from_date: NaiveDate,
    /// The last day, `YYYY-MM-DD`.
    pub to_date: NaiveDate,
}

impl IntradayRequest {
    /// A request without open interest.
    pub fn new(
        exchange_segment: ExchangeSegment,
        security_id: SecurityId,
        instrument: InstrumentKind,
        interval: IntradayInterval,
        from_date: NaiveDate,
        to_date: NaiveDate,
    ) -> Self {
        Self {
            security_id,
            exchange_segment,
            instrument,
            interval,
            oi: false,
            from_date,
            to_date,
        }
    }

    /// Includes open interest.
    pub fn with_oi(mut self, oi: bool) -> Self {
        self.oi = oi;
        self
    }

    /// Checks the segment, the security ID and that `from_date` is not after `to_date`.
    pub fn validate(&self) -> Result<(), ValidationError> {
        check_common(
            self.exchange_segment,
            &self.security_id,
            self.from_date,
            self.to_date,
        )?;
        if self.from_date > self.to_date {
            return Err(ValidationError::new(
                "from_date",
                ValidationReason::Inconsistent("must not be after to_date"),
            ));
        }
        Ok(())
    }
}

/// Candles as the server sends them: one array per field, index-aligned (DOC:5383-5399, OQ-5).
///
/// Every array defaults to empty, and `null` is empty; a scalar where an array belongs is a
/// decode error. All non-empty arrays must have the same length, and when there are timestamps
/// the price and volume arrays must be present; otherwise decoding fails. `open_interest` is
/// empty unless it was requested. A body that wraps the columns in `data` is a decode error
/// rather than an empty result.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Candles {
    /// Open prices.
    pub open: Vec<f64>,
    /// High prices.
    pub high: Vec<f64>,
    /// Low prices.
    pub low: Vec<f64>,
    /// Close prices.
    pub close: Vec<f64>,
    /// Volumes.
    pub volume: Vec<i64>,
    /// Open interest, when requested.
    pub open_interest: Vec<i64>,
    /// Candle times, epoch seconds. Like the volume columns, integral floats and integer
    /// strings are accepted (a response may carry `1.7567e9`-style numbers).
    pub timestamp: Vec<i64>,
}

/// The wire shape of [`Candles`], before the length checks.
#[derive(Deserialize)]
struct WireCandles {
    /// Present only if the server wrapped the columns; refused rather than read as empty.
    #[serde(default)]
    data: Option<serde::de::IgnoredAny>,
    #[serde(default, deserialize_with = "null_as_empty")]
    open: Vec<f64>,
    #[serde(default, deserialize_with = "null_as_empty")]
    high: Vec<f64>,
    #[serde(default, deserialize_with = "null_as_empty")]
    low: Vec<f64>,
    #[serde(default, deserialize_with = "null_as_empty")]
    close: Vec<f64>,
    #[serde(default, deserialize_with = "null_as_empty")]
    volume: Vec<LenientInt>,
    #[serde(default, deserialize_with = "null_as_empty")]
    open_interest: Vec<LenientInt>,
    #[serde(default, deserialize_with = "null_as_empty")]
    timestamp: Vec<LenientInt>,
}

impl<'de> Deserialize<'de> for Candles {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = WireCandles::deserialize(d)?;
        if w.data.is_some() {
            return Err(serde::de::Error::custom(
                "candles are wrapped in a data object",
            ));
        }
        let ints = |v: Vec<LenientInt>| v.into_iter().map(|i| i.0).collect::<Vec<_>>();
        let candles = Candles {
            open: w.open,
            high: w.high,
            low: w.low,
            close: w.close,
            volume: ints(w.volume),
            open_interest: ints(w.open_interest),
            timestamp: ints(w.timestamp),
        };
        let lengths = [
            candles.open.len(),
            candles.high.len(),
            candles.low.len(),
            candles.close.len(),
            candles.volume.len(),
            candles.open_interest.len(),
            candles.timestamp.len(),
        ];
        let n = candles.timestamp.len();
        if lengths.iter().any(|&len| len != 0 && len != n) {
            return Err(serde::de::Error::custom(
                "candle arrays have different lengths",
            ));
        }
        // Every candle needs its prices and volume; only open interest is optional.
        if n != 0 && lengths[..5].contains(&0) {
            return Err(serde::de::Error::custom("a candle array is missing"));
        }
        Ok(candles)
    }
}

impl Candles {
    /// The number of candles.
    pub fn len(&self) -> usize {
        self.timestamp.len()
    }

    /// Whether there are no candles.
    pub fn is_empty(&self) -> bool {
        self.timestamp.is_empty()
    }

    /// The candles in order. A decoded value has aligned columns; if the public fields were
    /// changed so that they are not, iteration stops at the shortest required column.
    pub fn iter(&self) -> impl Iterator<Item = Candle> + '_ {
        (0..self.len()).map_while(|i| {
            Some(Candle {
                ts: self.timestamp[i],
                open: *self.open.get(i)?,
                high: *self.high.get(i)?,
                low: *self.low.get(i)?,
                close: *self.close.get(i)?,
                volume: *self.volume.get(i)?,
                open_interest: self.open_interest.get(i).copied(),
            })
        })
    }
}

/// One candle.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candle {
    /// The candle time, epoch seconds.
    pub ts: i64,
    /// The open price.
    pub open: f64,
    /// The high price.
    pub high: f64,
    /// The low price.
    pub low: f64,
    /// The close price.
    pub close: f64,
    /// The volume.
    pub volume: i64,
    /// The open interest, when requested.
    pub open_interest: Option<i64>,
}

impl Candle {
    /// The candle time in IST.
    pub fn time_ist(&self) -> Option<DateTime<FixedOffset>> {
        epoch_to_ist(self.ts)
    }
}

#[cfg(test)]
#[path = "historical_tests.rs"]
mod tests;
