use serde_json::json;

use super::*;

fn id(s: &str) -> SecurityId {
    SecurityId::new(s).unwrap()
}

fn invalid(field: &'static str, reason: ValidationReason) -> Result<(), ValidationError> {
    Err(ValidationError { field, reason })
}

#[test]
fn the_body_maps_each_segment_to_integer_ids() {
    let mut req = QuoteRequest::new();
    req.add(ExchangeSegment::NseFno, id("49082"))
        .add(ExchangeSegment::NseEq, id("11536"))
        .add(ExchangeSegment::NseFno, id("49081"))
        .add(ExchangeSegment::NseFno, id("49081"));
    assert_eq!(req.len(), 3);
    assert_eq!(req.validate(), Ok(()));
    assert_eq!(
        req.to_body(),
        json!({"NSE_EQ": [11536], "NSE_FNO": [49081, 49082]})
    );
}

#[test]
fn a_request_holds_one_to_a_thousand_instruments() {
    assert_eq!(
        QuoteRequest::new().validate(),
        invalid("instruments", ValidationReason::Empty)
    );
    let mut req = QuoteRequest::new();
    for n in 1..=1000 {
        req.add(ExchangeSegment::NseEq, id(&n.to_string()));
    }
    assert_eq!(req.validate(), Ok(()));
    req.add(ExchangeSegment::BseEq, id("1"));
    assert_eq!(req.len(), 1001);
    assert_eq!(
        req.validate(),
        invalid("instruments", ValidationReason::TooMany { max: 1000 })
    );
}

#[test]
fn security_ids_must_be_numeric() {
    let mut req = QuoteRequest::new();
    req.add(ExchangeSegment::NseEq, id("AAPL"));
    assert_eq!(
        req.validate(),
        invalid("security_id", ValidationReason::InvalidCharacters)
    );
    let mut req = QuoteRequest::new();
    req.add(ExchangeSegment::NseEq, id("99999999999"));
    assert_eq!(
        req.validate(),
        invalid("security_id", ValidationReason::OutOfRange)
    );
}

#[test]
fn quote_data_has_two_levels() {
    let data: QuoteData<LtpQuote> = serde_json::from_value(json!({
        "status": "success",
        "data": {
            "NSE_EQ": {"11536": {"last_price": 4520.0}},
            "NSE_FNO": {"49081": {"last_price": 368.15}, "49082": {"last_price": 694.35}}
        }
    }))
    .unwrap();
    assert_eq!(data.status.as_deref(), Some("success"));
    let price = |seg, s: &str| data.get(seg, &id(s)).and_then(|q| q.last_price);
    assert_eq!(price(ExchangeSegment::NseEq, "11536"), Some(4520.0));
    assert_eq!(price(ExchangeSegment::NseFno, "49082"), Some(694.35));
    assert_eq!(price(ExchangeSegment::BseEq, "11536"), None);
    assert_eq!(price(ExchangeSegment::NseEq, "1"), None);
}

#[test]
fn a_full_quote_decodes_every_key() {
    let q: FullQuote = serde_json::from_value(json!({
        "average_price": 1.5,
        "buy_quantity": 2,
        "sell_quantity": 3,
        "last_price": 4.5,
        "last_quantity": 5,
        "last_trade_time": "2024-09-11 14:39:29",
        "lower_circuit_limit": 6.5,
        "upper_circuit_limit": 7.5,
        "net_change": -8.5,
        "volume": 9,
        "oi": 10,
        "depth": {"buy": [{"quantity": 11, "price": 12.5, "orders": 13}], "sell": []}
    }))
    .unwrap();
    assert_eq!(
        [
            q.average_price,
            q.last_price,
            q.lower_circuit_limit,
            q.upper_circuit_limit,
            q.net_change
        ],
        [1.5, 4.5, 6.5, 7.5, -8.5].map(Some)
    );
    assert_eq!(
        [
            q.buy_quantity,
            q.sell_quantity,
            q.last_quantity,
            q.volume,
            q.oi
        ],
        [2, 3, 5, 9, 10].map(Some)
    );
    assert_eq!(
        q.last_trade_time.as_ref().map(WireTime::as_str),
        Some("2024-09-11 14:39:29")
    );
    let depth = q.depth.unwrap();
    assert_eq!(
        (
            depth.buy[0].quantity,
            depth.buy[0].price,
            depth.buy[0].orders
        ),
        (Some(11), Some(12.5), Some(13))
    );
    assert!(depth.sell.is_empty());
}

#[test]
fn null_collections_are_empty() {
    let data: QuoteData<FullQuote> = serde_json::from_value(json!({
        "status": "success",
        "data": {"NSE_EQ": {"11536": {"depth": {"buy": null, "sell": []}}}}
    }))
    .unwrap();
    let depth = data
        .get(ExchangeSegment::NseEq, &id("11536"))
        .and_then(|q| q.depth.as_ref())
        .unwrap();
    assert!(depth.buy.is_empty() && depth.sell.is_empty());
    let none: QuoteData<LtpQuote> =
        serde_json::from_value(json!({"status": "success", "data": null})).unwrap();
    assert!(none.data.is_empty());
}

#[test]
fn a_leading_zero_id_is_refused() {
    let mut req = QuoteRequest::new();
    req.add(ExchangeSegment::NseEq, id("011536"));
    assert_eq!(
        req.validate(),
        invalid(
            "security_id",
            ValidationReason::Inconsistent("leading zeros are lost on the wire")
        )
    );
    let mut zero = QuoteRequest::new();
    zero.add(ExchangeSegment::IdxI, id("0"));
    assert_eq!(zero.validate(), Ok(()));
}

#[test]
fn the_published_full_quote_extras_decode() {
    let q: FullQuote = serde_json::from_value(json!({
        "last_price": 368.15,
        "last_trade_time": "01/01/1980 00:00:00",
        "ohlc": {"open": 0, "close": 368.15, "high": 0, "low": 0},
        "oi_day_high": 12,
        "oi_day_low": 3
    }))
    .unwrap();
    assert_eq!((q.oi_day_high, q.oi_day_low), (Some(12), Some(3)));
    assert_eq!(q.ohlc.and_then(|o| o.close), Some(368.15));
}

#[test]
fn a_null_segment_is_empty_and_global_stocks_is_refused() {
    let data: QuoteData<LtpQuote> = serde_json::from_value(json!({
        "data": {"NSE_EQ": null, "NSE_FNO": {"49081": {"last_price": 1.0}}}
    }))
    .unwrap();
    assert!(data.data["NSE_EQ"].is_empty());
    assert_eq!(
        data.get(ExchangeSegment::NseFno, &id("49081"))
            .and_then(|q| q.last_price),
        Some(1.0)
    );
    let mut req = QuoteRequest::new();
    req.add(ExchangeSegment::InxEq, id("1234"));
    assert_eq!(
        req.validate(),
        invalid("exchange_segment", ValidationReason::UnknownEnumValue)
    );
}
