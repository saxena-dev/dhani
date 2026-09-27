use serde_json::json;

use super::*;

fn nifty() -> UnderlyingRef {
    UnderlyingRef::new(13, ExchangeSegment::IdxI)
}

fn expiry() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 10, 31).unwrap()
}

#[test]
fn the_request_uses_pascal_case_keys_and_an_integer_scrip() {
    assert_eq!(
        serde_json::to_value(OptionChainRequest::new(nifty(), expiry())).unwrap(),
        json!({"UnderlyingScrip": 13, "UnderlyingSeg": "IDX_I", "Expiry": "2024-10-31"})
    );
    assert_eq!(
        serde_json::to_value(nifty()).unwrap(),
        json!({"UnderlyingScrip": 13, "UnderlyingSeg": "IDX_I"})
    );
}

#[test]
fn validation() {
    assert_eq!(
        OptionChainRequest::new(nifty(), expiry()).validate(),
        Ok(())
    );
    let zero = UnderlyingRef::new(0, ExchangeSegment::IdxI);
    assert_eq!(
        zero.validate(),
        Err(ValidationError {
            field: "scrip",
            reason: ValidationReason::NotPositive
        })
    );
    let far = NaiveDate::from_ymd_opt(10_000, 1, 1).unwrap();
    assert_eq!(
        OptionChainRequest::new(nifty(), far).validate(),
        Err(ValidationError {
            field: "expiry",
            reason: ValidationReason::OutOfRange
        })
    );
}

#[test]
fn an_option_decodes_every_key() {
    let o: OptionData = serde_json::from_value(json!({
        "average_price": 1.5,
        "greeks": {"delta": 0.53, "theta": -15.1, "gamma": 0.0012, "vega": 12.2},
        "implied_volatility": 9.8,
        "last_price": 134.0,
        "oi": 3786445,
        "security_id": 42528,
        "top_bid_price": 133.55,
        "top_bid_quantity": 1625,
        "top_ask_price": 134.0,
        "top_ask_quantity": 1365,
        "volume": 117567970
    }))
    .unwrap();
    let g = o.greeks.as_ref().unwrap();
    assert_eq!(
        (g.delta, g.theta, g.gamma, g.vega),
        (Some(0.53), Some(-15.1), Some(0.0012), Some(12.2))
    );
    assert_eq!(
        [
            o.average_price,
            o.implied_volatility,
            o.last_price,
            o.top_bid_price,
            o.top_ask_price
        ],
        [1.5, 9.8, 134.0, 133.55, 134.0].map(Some)
    );
    assert_eq!(
        [o.oi, o.top_bid_quantity, o.top_ask_quantity, o.volume],
        [3786445, 1625, 1365, 117567970].map(Some)
    );
    assert_eq!(o.security_id, Some(42528));
}

#[test]
fn strikes_are_parsed_and_sorted_numerically() {
    let chain: OptionChainData = serde_json::from_value(json!({
        "last_price": 24964.25,
        "oc": {
            "25650.000000": {"ce": {"last_price": 1.0}},
            "9500.000000": {"pe": {"last_price": 2.0}},
            "not-a-strike": {}
        }
    }))
    .unwrap();
    // The keys are kept exactly as sent.
    assert!(chain.oc.contains_key("25650.000000"));
    let strikes: Vec<f64> = chain.strikes().iter().map(|(s, _)| *s).collect();
    assert_eq!(strikes, [9500.0, 25650.0]);
    assert!(chain.strikes()[0].1.ce.is_none());
}

#[test]
fn a_null_chain_is_empty() {
    let chain: OptionChainData = serde_json::from_value(json!({"oc": null})).unwrap();
    assert!(chain.oc.is_empty() && chain.last_price.is_none());
}

#[test]
fn the_expiry_list_decodes_dates_and_refuses_anything_else() {
    let list: ExpiryList = serde_json::from_value(json!({
        "status": "success",
        "data": ["2024-10-31", "2024-11-28"]
    }))
    .unwrap();
    assert_eq!(
        list.data,
        [expiry(), NaiveDate::from_ymd_opt(2024, 11, 28).unwrap()]
    );
    let none: ExpiryList = serde_json::from_value(json!({"data": null})).unwrap();
    assert!(none.data.is_empty());
    assert!(serde_json::from_value::<ExpiryList>(json!({"data": ["x"]})).is_err());
}
