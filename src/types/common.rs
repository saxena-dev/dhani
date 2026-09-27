//! Shared wire enums: `ExchangeSegment`, `ProductType`, `TransactionType`, `OrderType`,
//! `Validity`, `OrderStatus`, `AmoTime`, `LegName`, `PositionType`, `OptionType`,
//! `InstrumentKind`, `ExpiryCode` and `ExpiryFlag`.
//!
//! Requests use these enums directly; responses wrap them in [`Inbound`](crate::types::Inbound)
//! so that values missing from the documentation are preserved.

use std::fmt;

use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize, Serializer};

use crate::types::{Inbound, UnknownValue, WireEnum};

crate::types::wire_enum! {
    /// An exchange segment (DOC:4116-4125).
    ///
    /// The documentation's table lists six segments. `NSE_CURRENCY` and `BSE_CURRENCY` come from
    /// the Python SDK's feed codes (PY:src/dhanhq/marketfeed.py:23-30, 314-321), `NSE_COMM` from
    /// the order request fields (DOC:3731) and `INX_EQ` from the Global Stocks feed (DOC:1833; code 14 at DOC:1935).
    /// Binary feeds carry the numeric code: see [`feed_code`](ExchangeSegment::feed_code).
    pub enum ExchangeSegment {
        /// Index values (feed code 0).
        IdxI => "IDX_I",
        /// NSE equity cash (feed code 1).
        NseEq => "NSE_EQ",
        /// NSE futures and options (feed code 2).
        NseFno => "NSE_FNO",
        /// NSE currency (feed code 3).
        NseCurrency => "NSE_CURRENCY",
        /// BSE equity cash (feed code 4).
        BseEq => "BSE_EQ",
        /// MCX commodity (feed code 5).
        McxComm => "MCX_COMM",
        /// BSE currency (feed code 7).
        BseCurrency => "BSE_CURRENCY",
        /// BSE futures and options (feed code 8).
        BseFno => "BSE_FNO",
        /// NSE commodity; it has no documented feed code (OQ-16).
        NseComm => "NSE_COMM",
        /// Global Stocks equity (feed code 14, Global Stocks feed only).
        InxEq => "INX_EQ",
    }
}

/// The single mapping between segments and binary feed codes (Appendix A D17, S14).
const FEED_CODES: [(ExchangeSegment, u8); 9] = [
    (ExchangeSegment::IdxI, 0),
    (ExchangeSegment::NseEq, 1),
    (ExchangeSegment::NseFno, 2),
    (ExchangeSegment::NseCurrency, 3),
    (ExchangeSegment::BseEq, 4),
    (ExchangeSegment::McxComm, 5),
    (ExchangeSegment::BseCurrency, 7),
    (ExchangeSegment::BseFno, 8),
    (ExchangeSegment::InxEq, 14),
];

impl ExchangeSegment {
    /// The numeric code binary feeds use for this segment; `None` for `NseComm` (OQ-16).
    pub fn feed_code(self) -> Option<u8> {
        FEED_CODES.iter().find(|(s, _)| *s == self).map(|&(_, c)| c)
    }

    /// The segment for a binary feed code; `None` for an unknown code.
    pub fn from_feed_code(code: u8) -> Option<Self> {
        FEED_CODES.iter().find(|(_, c)| *c == code).map(|&(s, _)| s)
    }
}

crate::types::wire_enum! {
    /// A product type (DOC:4129-4136). `CO` and `BO` appear in responses only
    /// (PY:src/dhanhq/dhanhq.py:43-44); requests reject them.
    pub enum ProductType {
        /// Cash and carry, for equity delivery.
        Cnc => "CNC",
        /// Intraday.
        Intraday => "INTRADAY",
        /// Carry-forward in futures and options.
        Margin => "MARGIN",
        /// Margin trading facility.
        Mtf => "MTF",
        /// Cover order (responses only).
        Co => "CO",
        /// Bracket order (responses only).
        Bo => "BO",
    }
}

crate::types::wire_enum! {
    /// The side of a transaction (DOC:3730).
    pub enum TransactionType {
        /// Buy.
        Buy => "BUY",
        /// Sell.
        Sell => "SELL",
    }
}

crate::types::wire_enum! {
    /// An order type (DOC:3733).
    pub enum OrderType {
        /// Limit order.
        Limit => "LIMIT",
        /// Market order.
        Market => "MARKET",
        /// Stop-loss limit order.
        StopLoss => "STOP_LOSS",
        /// Stop-loss market order.
        StopLossMarket => "STOP_LOSS_MARKET",
    }
}

crate::types::wire_enum! {
    /// Order validity (DOC:3734).
    pub enum Validity {
        /// Valid for the trading day.
        Day => "DAY",
        /// Immediate or cancel.
        Ioc => "IOC",
    }
}

crate::types::wire_enum! {
    /// An order status: the union of the values listed on different pages (DOC:4140-4151,
    /// DOC:195, DOC:1587, DOC:2340; Appendix A D56). Always received as `Inbound<OrderStatus>`.
    pub enum OrderStatus {
        /// Did not reach the exchange.
        Transit => "TRANSIT",
        /// Awaiting execution.
        Pending => "PENDING",
        /// Super order with both entry and exit placed.
        Closed => "CLOSED",
        /// Super order whose target or stop-loss leg triggered.
        Triggered => "TRIGGERED",
        /// Rejected by the broker or exchange.
        Rejected => "REJECTED",
        /// Cancelled by the user.
        Cancelled => "CANCELLED",
        /// Partly traded.
        PartTraded => "PART_TRADED",
        /// Fully traded.
        Traded => "TRADED",
        /// Expired.
        Expired => "EXPIRED",
        /// Modified.
        Modified => "MODIFIED",
        /// Inactive.
        Inactive => "INACTIVE",
    }
}

crate::types::wire_enum! {
    /// When an after-market order is sent to the exchange (DOC:4155-4162). `PRE_OPEN` is
    /// allowed, following the documentation rather than the Python SDK (Appendix A D26).
    pub enum AmoTime {
        /// At the pre-market session.
        PreOpen => "PRE_OPEN",
        /// At market open.
        Open => "OPEN",
        /// 30 minutes after market open.
        Open30 => "OPEN_30",
        /// 60 minutes after market open.
        Open60 => "OPEN_60",
    }
}

crate::types::wire_enum! {
    /// A super-order leg (DOC:3139, DOC:3194).
    pub enum LegName {
        /// The entry leg (the whole super order while it is pending).
        EntryLeg => "ENTRY_LEG",
        /// The target leg.
        TargetLeg => "TARGET_LEG",
        /// The stop-loss leg.
        StopLossLeg => "STOP_LOSS_LEG",
        /// No leg.
        Na => "NA",
    }
}

crate::types::wire_enum! {
    /// A position's direction (DOC:1486, DOC:303).
    pub enum PositionType {
        /// Long.
        Long => "LONG",
        /// Short.
        Short => "SHORT",
        /// Closed.
        Closed => "CLOSED",
    }
}

crate::types::wire_enum! {
    /// An option type (DOC:1508).
    pub enum OptionType {
        /// Call.
        Call => "CALL",
        /// Put.
        Put => "PUT",
    }
}

crate::types::wire_enum! {
    /// An instrument kind (DOC:4176-4187).
    pub enum InstrumentKind {
        /// Index.
        Index => "INDEX",
        /// Index futures.
        Futidx => "FUTIDX",
        /// Index options.
        Optidx => "OPTIDX",
        /// Equity.
        Equity => "EQUITY",
        /// Stock futures.
        Futstk => "FUTSTK",
        /// Stock options.
        Optstk => "OPTSTK",
        /// Commodity futures.
        Futcom => "FUTCOM",
        /// Options on commodity futures.
        Optfut => "OPTFUT",
    }
}

crate::types::wire_enum! {
    /// Weekly or monthly expiry (DOC:813).
    pub enum ExpiryFlag {
        /// Weekly expiry.
        Week => "WEEK",
        /// Monthly expiry.
        Month => "MONTH",
    }
}

/// Which expiry of a contract series (DOC:4166-4172). Sent as a JSON integer.
///
/// **Unverified encoding (OQ-30).** The documentation's annexure and the rolling-options page
/// (DOC:814) use 1 = near, 2 = next, 3 = far, which is what this type sends. The OpenAPI spec
/// describes 0 = current, 1 = next, 2 = far (`OAS:#/components/schemas/ExpiredOptionsRequest`),
/// and the Python SDK accepts 0 to 3 for daily data (PY:src/dhanhq/_historical_data.py:57) and
/// documents 0 to 3 for rolling options (PY:src/dhanhq/_historical_data.py:86). If the OpenAPI
/// reading is the live behaviour, `Near` selects the next expiry. The encoding stays as
/// documented until it is checked against the live API.
///
/// Its wire text is the decimal code (`"1"`, `"2"`, `"3"`). When received as
/// `Inbound<ExpiryCode>`, a JSON string such as `"1"` is known, while a JSON number is kept as
/// `Unknown` holding its text, like every numeric enum value (see [`Inbound`]); response models
/// whose documented wire form is the integer read it with a crate-private helper that makes
/// `1`, `2` and `3` known.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ExpiryCode {
    /// The nearest expiry (1).
    Near,
    /// The next expiry (2).
    Next,
    /// The far expiry (3).
    Far,
}

impl ExpiryCode {
    fn code(self) -> u8 {
        match self {
            Self::Near => 1,
            Self::Next => 2,
            Self::Far => 3,
        }
    }

    fn from_code(code: u64) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|v| u64::from(v.code()) == code)
    }
}

impl WireEnum for ExpiryCode {
    const ALL: &'static [Self] = &[Self::Near, Self::Next, Self::Far];

    fn as_wire(self) -> &'static str {
        match self {
            Self::Near => "1",
            Self::Next => "2",
            Self::Far => "3",
        }
    }

    fn from_wire(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|v| v.as_wire() == s)
    }
}

impl fmt::Display for ExpiryCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_wire())
    }
}

impl std::str::FromStr for ExpiryCode {
    type Err = UnknownValue;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_wire(s).ok_or_else(|| UnknownValue::new(s))
    }
}

impl Serialize for ExpiryCode {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u8(self.code())
    }
}

impl<'de> Deserialize<'de> for ExpiryCode {
    /// Strict: the integer code, or its decimal text. The offending value is not echoed.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct CodeVisitor;

        impl Visitor<'_> for CodeVisitor {
            type Value = ExpiryCode;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an expiry code 1, 2 or 3")
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<ExpiryCode, E> {
                ExpiryCode::from_code(v).ok_or_else(|| E::custom("unknown ExpiryCode value"))
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<ExpiryCode, E> {
                u64::try_from(v)
                    .ok()
                    .and_then(ExpiryCode::from_code)
                    .ok_or_else(|| E::custom("unknown ExpiryCode value"))
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<ExpiryCode, E> {
                ExpiryCode::from_wire(v).ok_or_else(|| E::custom("unknown ExpiryCode value"))
            }

            // Other scalars get the same message, so their value is not echoed.
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<ExpiryCode, E> {
                Err(E::custom("unknown ExpiryCode value"))
            }

            fn visit_f64<E: de::Error>(self, _: f64) -> Result<ExpiryCode, E> {
                Err(E::custom("unknown ExpiryCode value"))
            }
        }

        d.deserialize_any(CodeVisitor)
    }
}

impl<'de> Deserialize<'de> for Inbound<ExpiryCode> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        crate::types::deserialize_inbound(d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::DeserializeOwned;

    /// Every variant serialises to its wire string, parses back through serde, `FromStr` and
    /// `Inbound`, and an unknown string becomes `Inbound::Unknown`.
    fn round_trips<T>(expected: &[(T, &str)])
    where
        T: WireEnum + Serialize + DeserializeOwned + std::str::FromStr + fmt::Display,
        Inbound<T>: DeserializeOwned,
    {
        assert_eq!(T::ALL, expected.iter().map(|(v, _)| *v).collect::<Vec<_>>());
        for &(v, wire) in expected {
            let json = format!("\"{wire}\"");
            assert_eq!(serde_json::to_string(&v).unwrap(), json);
            assert_eq!(serde_json::from_str::<T>(&json).unwrap(), v);
            assert_eq!(v.to_string(), wire);
            assert!(wire.parse::<T>().is_ok_and(|p| p == v));
            assert_eq!(
                serde_json::from_str::<Inbound<T>>(&json).unwrap(),
                Inbound::Known(v)
            );
        }
        let unknown = serde_json::from_str::<Inbound<T>>(r#""NOT_A_VALUE""#).unwrap();
        assert_eq!(unknown.known(), None);
        assert_eq!(unknown.as_wire(), "NOT_A_VALUE");
        assert!(serde_json::from_str::<T>(r#""NOT_A_VALUE""#).is_err());
    }

    #[test]
    fn exchange_segment() {
        use ExchangeSegment::*;
        round_trips(&[
            (IdxI, "IDX_I"),
            (NseEq, "NSE_EQ"),
            (NseFno, "NSE_FNO"),
            (NseCurrency, "NSE_CURRENCY"),
            (BseEq, "BSE_EQ"),
            (McxComm, "MCX_COMM"),
            (BseCurrency, "BSE_CURRENCY"),
            (BseFno, "BSE_FNO"),
            (NseComm, "NSE_COMM"),
            (InxEq, "INX_EQ"),
        ]);
        assert_eq!(ExchangeSegment::ALL.len(), 10);
    }

    #[test]
    fn exchange_segment_feed_codes_map_both_ways() {
        use ExchangeSegment::*;
        let expected = [
            (IdxI, Some(0)),
            (NseEq, Some(1)),
            (NseFno, Some(2)),
            (NseCurrency, Some(3)),
            (BseEq, Some(4)),
            (McxComm, Some(5)),
            (BseCurrency, Some(7)),
            (BseFno, Some(8)),
            (NseComm, None),
            (InxEq, Some(14)),
        ];
        for (segment, code) in expected {
            assert_eq!(segment.feed_code(), code, "{segment:?}");
            if let Some(c) = code {
                assert_eq!(ExchangeSegment::from_feed_code(c), Some(segment));
            }
        }
        assert_eq!(ExchangeSegment::from_feed_code(14), Some(InxEq));
        assert_eq!(ExchangeSegment::from_feed_code(3), Some(NseCurrency));
        for unknown in [6, 9, 13, 15, 255] {
            assert_eq!(ExchangeSegment::from_feed_code(unknown), None, "{unknown}");
        }
    }

    #[test]
    fn product_transaction_order_validity() {
        use ProductType::*;
        round_trips(&[
            (Cnc, "CNC"),
            (Intraday, "INTRADAY"),
            (Margin, "MARGIN"),
            (Mtf, "MTF"),
            (Co, "CO"),
            (Bo, "BO"),
        ]);
        round_trips(&[
            (TransactionType::Buy, "BUY"),
            (TransactionType::Sell, "SELL"),
        ]);
        round_trips(&[
            (OrderType::Limit, "LIMIT"),
            (OrderType::Market, "MARKET"),
            (OrderType::StopLoss, "STOP_LOSS"),
            (OrderType::StopLossMarket, "STOP_LOSS_MARKET"),
        ]);
        round_trips(&[(Validity::Day, "DAY"), (Validity::Ioc, "IOC")]);
    }

    #[test]
    fn order_status_has_the_eleven_listed_values() {
        use OrderStatus::*;
        round_trips(&[
            (Transit, "TRANSIT"),
            (Pending, "PENDING"),
            (Closed, "CLOSED"),
            (Triggered, "TRIGGERED"),
            (Rejected, "REJECTED"),
            (Cancelled, "CANCELLED"),
            (PartTraded, "PART_TRADED"),
            (Traded, "TRADED"),
            (Expired, "EXPIRED"),
            (Modified, "MODIFIED"),
            (Inactive, "INACTIVE"),
        ]);
        assert_eq!(OrderStatus::ALL.len(), 11);
        // The upstream fixtures' Swagger placeholder decodes as an unknown status.
        let placeholder: Inbound<OrderStatus> = serde_json::from_str(r#""string""#).unwrap();
        assert_eq!(placeholder.as_wire(), "string");
    }

    #[test]
    fn amo_leg_position_option() {
        round_trips(&[
            (AmoTime::PreOpen, "PRE_OPEN"),
            (AmoTime::Open, "OPEN"),
            (AmoTime::Open30, "OPEN_30"),
            (AmoTime::Open60, "OPEN_60"),
        ]);
        round_trips(&[
            (LegName::EntryLeg, "ENTRY_LEG"),
            (LegName::TargetLeg, "TARGET_LEG"),
            (LegName::StopLossLeg, "STOP_LOSS_LEG"),
            (LegName::Na, "NA"),
        ]);
        round_trips(&[
            (PositionType::Long, "LONG"),
            (PositionType::Short, "SHORT"),
            (PositionType::Closed, "CLOSED"),
        ]);
        round_trips(&[(OptionType::Call, "CALL"), (OptionType::Put, "PUT")]);
    }

    #[test]
    fn instrument_kind_and_expiry_flag() {
        use InstrumentKind::*;
        round_trips(&[
            (Index, "INDEX"),
            (Futidx, "FUTIDX"),
            (Optidx, "OPTIDX"),
            (Equity, "EQUITY"),
            (Futstk, "FUTSTK"),
            (Optstk, "OPTSTK"),
            (Futcom, "FUTCOM"),
            (Optfut, "OPTFUT"),
        ]);
        round_trips(&[(ExpiryFlag::Week, "WEEK"), (ExpiryFlag::Month, "MONTH")]);
    }

    #[test]
    fn expiry_code_is_an_integer_on_the_wire() {
        assert_eq!(
            ExpiryCode::ALL,
            [ExpiryCode::Near, ExpiryCode::Next, ExpiryCode::Far]
        );
        assert_eq!(serde_json::to_string(&ExpiryCode::Near).unwrap(), "1");
        assert_eq!(serde_json::to_string(&ExpiryCode::Next).unwrap(), "2");
        assert_eq!(serde_json::to_string(&ExpiryCode::Far).unwrap(), "3");
        assert_eq!(
            serde_json::from_str::<ExpiryCode>("3").unwrap(),
            ExpiryCode::Far
        );
        assert_eq!(
            serde_json::from_str::<ExpiryCode>(r#""2""#).unwrap(),
            ExpiryCode::Next
        );
        for bad in [
            "0",
            "4",
            "-1",
            r#""Near""#,
            "true",
            "1.5",
            "18446744073709551616",
        ] {
            let err = serde_json::from_str::<ExpiryCode>(bad)
                .unwrap_err()
                .to_string();
            assert!(err.starts_with("unknown ExpiryCode value"), "{bad}: {err}");
        }
        assert_eq!(ExpiryCode::Near.to_string(), "1");
        assert_eq!("3".parse::<ExpiryCode>(), Ok(ExpiryCode::Far));
        // Inbound policy: a JSON number is preserved as Unknown text; the decimal string is known.
        let n: Inbound<ExpiryCode> = serde_json::from_str("1").unwrap();
        assert_eq!(n.known(), None);
        assert_eq!(n.as_wire(), "1");
        let s: Inbound<ExpiryCode> = serde_json::from_str(r#""1""#).unwrap();
        assert_eq!(s, Inbound::Known(ExpiryCode::Near));
    }
}
