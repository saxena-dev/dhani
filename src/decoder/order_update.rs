//! JSON order-update message parser.
//!
//! The Live Order Update feed sends JSON text frames (DOC:6063-6247). An `order_alert` message
//! carries an [`OrderUpdate`]; any other message type is kept as raw JSON. Every field of an
//! order update is optional (the documentation marks all of them "Required: No",
//! DOC:6198-6247), and numbers may arrive as strings or strings as numbers.
//!
//! Messages are parsed through `serde_json::Value` first: the documentation's own sample repeats
//! the `Remarks` key (DOC:6146, DOC:6166), which a derived struct would reject; through `Value`,
//! the last occurrence wins.

use serde::{Deserialize, Deserializer};

use super::error::{DecodeError, DecodeErrorKind};
use crate::credentials::ClientId;
use crate::types::serde_ext::{inbound_ci, int_or_string, num_or_string};
use crate::types::{Inbound, OrderStatus, ProductType, RawJson, WireTime};

/// The message type of an order update (DOC:6119-6169).
const ORDER_ALERT: &str = "order_alert";

/// One order-update message as sent: its type and, for order alerts, the order.
#[non_exhaustive]
#[derive(Clone, Debug, Deserialize)]
pub struct OrderUpdateMessage {
    /// The message type (wire `Type`), e.g. `order_alert`.
    #[serde(rename = "Type", default)]
    pub kind: Option<String>,
    /// The order (wire `Data`).
    #[serde(rename = "Data", default)]
    pub data: Option<OrderUpdate>,
}

/// An order's state as reported by the order-update feed (DOC:6198-6247). Every field is
/// optional.
#[non_exhaustive]
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct OrderUpdate {
    /// Exchange, e.g. `NSE`.
    #[serde(default)]
    pub exchange: Option<String>,
    /// Segment code, e.g. `E`.
    #[serde(default)]
    pub segment: Option<String>,
    /// Order source, e.g. `P` for API orders.
    #[serde(default)]
    pub source: Option<String>,
    /// Security ID.
    #[serde(default)]
    pub security_id: Option<String>,
    /// The account's client ID (redacted in `Debug`).
    #[serde(default)]
    pub client_id: Option<ClientId>,
    /// Exchange order number.
    #[serde(default)]
    pub exch_order_no: Option<String>,
    /// Dhan order number.
    #[serde(default)]
    pub order_no: Option<String>,
    /// Product code: `C`, `I`, `M`, `F`, `V` or `B`; see [`OrderUpdate::product`].
    #[serde(default)]
    pub product: Option<String>,
    /// Transaction type: `B` or `S`.
    #[serde(default)]
    pub txn_type: Option<String>,
    /// Order type: `LMT`, `MKT`, `SL` or `SLM`.
    #[serde(default)]
    pub order_type: Option<String>,
    /// Validity: `DAY` or `IOC`.
    #[serde(default)]
    pub validity: Option<String>,
    /// Disclosed quantity.
    #[serde(default, deserialize_with = "int_or_string")]
    pub disc_quantity: Option<i64>,
    /// Disclosed quantity pending.
    #[serde(default, deserialize_with = "int_or_string")]
    pub disc_qty_rem: Option<i64>,
    /// Quantity pending execution.
    #[serde(default, deserialize_with = "int_or_string")]
    pub remaining_quantity: Option<i64>,
    /// Total order quantity.
    #[serde(default, deserialize_with = "int_or_string")]
    pub quantity: Option<i64>,
    /// Quantity executed.
    #[serde(default, deserialize_with = "int_or_string")]
    pub traded_qty: Option<i64>,
    /// Order price.
    #[serde(default, deserialize_with = "num_or_string")]
    pub price: Option<f64>,
    /// Trigger price.
    #[serde(default, deserialize_with = "num_or_string")]
    pub trigger_price: Option<f64>,
    /// Price of the last execution.
    #[serde(default, deserialize_with = "num_or_string")]
    pub traded_price: Option<f64>,
    /// Average execution price.
    #[serde(default, deserialize_with = "num_or_string")]
    pub avg_traded_price: Option<f64>,
    /// Entry-leg order number of a bracket or cover order (documented as a number).
    #[serde(default, deserialize_with = "text_or_number")]
    pub algo_ord_no: Option<String>,
    /// After-market order flag: `1` or `0` (accepted as a number too).
    #[serde(default, deserialize_with = "text_or_number")]
    pub off_mkt_flag: Option<String>,
    /// When Dhan received the order.
    #[serde(default)]
    pub order_date_time: Option<WireTime>,
    /// When the order reached the exchange.
    #[serde(default)]
    pub exch_order_time: Option<WireTime>,
    /// Last update time.
    #[serde(default)]
    pub last_updated_time: Option<WireTime>,
    /// Remarks, e.g. `Super Order`.
    #[serde(default)]
    pub remarks: Option<String>,
    /// Market type: `NL`, `AU`, `A1` or `A2`.
    #[serde(default)]
    pub mkt_type: Option<String>,
    /// Rejection reason or status text.
    #[serde(default)]
    pub reason_description: Option<String>,
    /// Leg number: 1 entry, 2 stop loss, 3 target.
    #[serde(default, deserialize_with = "int_or_string")]
    pub leg_no: Option<i64>,
    /// Instrument type.
    #[serde(default)]
    pub instrument: Option<String>,
    /// Trading symbol.
    #[serde(default)]
    pub symbol: Option<String>,
    /// Product name, e.g. `CNC`.
    #[serde(default)]
    pub product_name: Option<String>,
    /// Order status, matched case-insensitively (the sample sends `Cancelled`, DOC:6153).
    #[serde(default, deserialize_with = "inbound_ci")]
    pub status: Option<Inbound<OrderStatus>>,
    /// Lot size.
    #[serde(default, deserialize_with = "int_or_string")]
    pub lot_size: Option<i64>,
    /// Strike price of an option.
    #[serde(default, deserialize_with = "num_or_string")]
    pub strike_price: Option<f64>,
    /// Contract expiry.
    #[serde(default)]
    pub expiry_date: Option<WireTime>,
    /// Option type: `CE`, `PE` or `XX`.
    #[serde(default)]
    pub opt_type: Option<String>,
    /// Display name.
    #[serde(default)]
    pub display_name: Option<String>,
    /// ISIN.
    #[serde(default)]
    pub isin: Option<String>,
    /// Exchange series.
    #[serde(default)]
    pub series: Option<String>,
    /// Good-till-days date.
    #[serde(default)]
    pub good_till_days_date: Option<WireTime>,
    /// Last traded price when the update was sent.
    #[serde(default, deserialize_with = "num_or_string")]
    pub ref_ltp: Option<f64>,
    /// Tick size.
    #[serde(default, deserialize_with = "num_or_string")]
    pub tick_size: Option<f64>,
    /// Exchange ID for special order types.
    #[serde(default)]
    pub algo_id: Option<String>,
    /// Contract multiplier.
    #[serde(default, deserialize_with = "int_or_string")]
    pub multiplier: Option<i64>,
    /// Caller-supplied correlation ID.
    #[serde(default)]
    pub correlation_id: Option<String>,
}

impl OrderUpdate {
    /// The product code as a [`ProductType`] (`C` CNC, `I` intraday, `M` margin, `F` MTF, `V`
    /// cover order, `B` bracket order; DOC:6198-6247); `None` if absent or unrecognised.
    pub fn product(&self) -> Option<ProductType> {
        match self.product.as_deref()? {
            "C" => Some(ProductType::Cnc),
            "I" => Some(ProductType::Intraday),
            "M" => Some(ProductType::Margin),
            "F" => Some(ProductType::Mtf),
            "V" => Some(ProductType::Co),
            "B" => Some(ProductType::Bo),
            _ => None,
        }
    }
}

/// A string field that may also arrive as a number.
fn text_or_number<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    use serde::de::Error;
    match serde_json::Value::deserialize(d)? {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(s) => Ok(Some(s)),
        // A whole number sent as a float (the field table types AlgoOrdNo as float) keeps no
        // fractional part.
        serde_json::Value::Number(n) => Ok(Some(match (n.as_f64(), n.is_f64()) {
            (Some(f), true) if f.fract() == 0.0 && f.abs() < 9.0e15 => format!("{f:.0}"),
            _ => n.to_string(),
        })),
        _ => Err(D::Error::custom("expected a string or a number")),
    }
}

/// One parsed order-update feed message.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum OrderUpdateEvent {
    /// An `order_alert` message.
    Order(Box<OrderUpdate>),
    /// Any other message, kept whole.
    Other {
        /// The message type, if present.
        kind: Option<String>,
        /// The whole message.
        raw: RawJson,
    },
}

fn json_error() -> DecodeError {
    DecodeError {
        kind: DecodeErrorKind::Json,
        offset: 0,
        packet_code: None,
    }
}

/// Parses one order-update text frame.
pub fn parse_order_update(bytes: &[u8]) -> Result<OrderUpdateEvent, DecodeError> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| json_error())?;
    let kind = value
        .get("Type")
        .and_then(|t| t.as_str())
        .map(str::to_owned);
    if kind.as_deref() != Some(ORDER_ALERT) {
        return Ok(OrderUpdateEvent::Other {
            kind,
            raw: RawJson(value),
        });
    }
    let message: OrderUpdateMessage = serde_json::from_value(value).map_err(|_| json_error())?;
    Ok(OrderUpdateEvent::Order(Box::new(
        message.data.ok_or_else(json_error)?,
    )))
}

/// Parses one order-update frame of either kind: the feed sends only text, so a binary frame is
/// [`DecodeErrorKind::UnexpectedBinary`].
pub fn parse_order_update_frame(
    binary: bool,
    bytes: &[u8],
) -> Result<OrderUpdateEvent, DecodeError> {
    if binary {
        return Err(DecodeError {
            kind: DecodeErrorKind::UnexpectedBinary,
            offset: 0,
            packet_code: None,
        });
    }
    parse_order_update(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(json: &str) -> OrderUpdate {
        match parse_order_update(json.as_bytes()).unwrap() {
            OrderUpdateEvent::Order(o) => *o,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn numbers_and_strings_are_both_accepted() {
        let o = order(
            r#"{"Type":"order_alert","Data":{"Quantity":"25","Price":"101.5","AlgoOrdNo":1124091136547,"Status":"TRADED","Product":"I"}}"#,
        );
        assert_eq!(
            (o.quantity, o.price, o.algo_ord_no.as_deref()),
            (Some(25), Some(101.5), Some("1124091136547"))
        );
        assert_eq!(o.status, Some(Inbound::Known(OrderStatus::Traded)));
        assert_eq!(o.product(), Some(ProductType::Intraday));
    }

    #[test]
    fn float_and_numeric_text_fields_are_normalised() {
        let o =
            order(r#"{"Type":"order_alert","Data":{"AlgoOrdNo":1124091136547.0,"OffMktFlag":1}}"#);
        assert_eq!(
            (o.algo_ord_no.as_deref(), o.off_mkt_flag.as_deref()),
            (Some("1124091136547"), Some("1"))
        );
        let o = order(r#"{"Type":"order_alert","Data":{"AlgoOrdNo":12.5}}"#);
        assert_eq!(o.algo_ord_no.as_deref(), Some("12.5"));
    }

    #[test]
    fn every_field_is_optional_and_unknown_fields_are_ignored() {
        let o = order(r#"{"Type":"order_alert","Data":{"NewField":1}}"#);
        assert_eq!((o.product(), o.exchange, o.status), (None, None, None));
    }

    #[test]
    fn product_codes_map_to_product_types() {
        let codes = ["C", "I", "M", "F", "V", "B", "X"];
        let got: Vec<_> = codes
            .iter()
            .map(|c| {
                order(&format!(
                    r#"{{"Type":"order_alert","Data":{{"Product":"{c}"}}}}"#
                ))
                .product()
            })
            .collect();
        assert_eq!(
            got,
            [
                Some(ProductType::Cnc),
                Some(ProductType::Intraday),
                Some(ProductType::Margin),
                Some(ProductType::Mtf),
                Some(ProductType::Co),
                Some(ProductType::Bo),
                None
            ]
        );
    }

    #[test]
    fn other_messages_are_kept_raw() {
        let OrderUpdateEvent::Other { kind, raw } =
            parse_order_update(br#"{"Status":"ok"}"#).unwrap()
        else {
            panic!("expected Other")
        };
        assert_eq!(
            (kind, raw),
            (None, RawJson(serde_json::json!({"Status": "ok"})))
        );
    }

    #[test]
    fn malformed_json_and_malformed_orders_are_json_errors() {
        let json = DecodeError {
            kind: DecodeErrorKind::Json,
            offset: 0,
            packet_code: None,
        };
        assert_eq!(parse_order_update(b"{not json").unwrap_err(), json);
        assert_eq!(
            parse_order_update(br#"{"Type":"order_alert"}"#).unwrap_err(),
            json
        );
        assert_eq!(
            parse_order_update(br#"{"Type":"order_alert","Data":{"Status":{"a":1}}}"#).unwrap_err(),
            json
        );
    }
}
