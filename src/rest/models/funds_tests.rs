use serde_json::json;

use super::*;

fn leg() -> MarginRequest {
    MarginRequest::new(
        ExchangeSegment::NseEq,
        SecurityId::new("1333").unwrap(),
        TransactionType::Buy,
        5,
        ProductType::Cnc,
        1428.0,
    )
}

fn invalid(field: &'static str, reason: ValidationReason) -> Result<(), ValidationError> {
    Err(ValidationError { field, reason })
}

#[test]
fn fund_limits_keep_the_misspelled_wire_names() {
    let limits: FundLimits = serde_json::from_value(json!({
        "dhanClientId": "1000000009",
        "availabelBalance": 98440.0,
        "sodLimit": 113642.0,
        "collateralAmount": 0.0,
        "receiveableAmount": 5.5,
        "utilizedAmount": 15202.0,
        "blockedPayoutAmount": 0.0,
        "withdrawableBalance": 98310.0
    }))
    .unwrap();
    assert_eq!(limits.available_balance, Some(98440.0));
    assert_eq!(limits.receivable_amount, Some(5.5));
    assert_eq!(limits.sod_limit, Some(113642.0));
    assert_eq!(limits.utilized_amount, Some(15202.0));
    assert_eq!(limits.withdrawable_balance, Some(98310.0));
    assert!(!format!("{limits:?}").contains("1000000009"));
    // The corrected spellings are not the wire names.
    let limits: FundLimits =
        serde_json::from_value(json!({"availableBalance": 1.0, "receivableAmount": 2.0})).unwrap();
    assert_eq!(
        (limits.available_balance, limits.receivable_amount),
        (None, None)
    );
}

#[test]
fn multi_margin_accepts_camel_case_numbers_and_snake_case_strings() {
    for body in [json!({"totalMargin": 1.5}), json!({"total_margin": "1.5"})] {
        let m: MultiMargin = serde_json::from_value(body).unwrap();
        assert_eq!(m.total_margin, Some(1.5));
    }
    let doc: MultiMargin = serde_json::from_value(json!({
        "clientId": "1000000009",
        "totalMargin": 10.5,
        "spanMargin": 1.5,
        "exposure": 2.5,
        "equityMargin": 3.5,
        "foMargin": 4.5,
        "commodity": 5.5,
        "currency": 6.5
    }))
    .unwrap();
    let oas: MultiMargin = serde_json::from_value(json!({
        "total_margin": "10.5",
        "span_margin": "1.5",
        "exposure_margin": "2.5",
        "equity_margin": "3.5",
        "fo_margin": "4.5",
        "commodity_margin": "5.5",
        "currency": "6.5",
        "hedge_benefit": "7.5"
    }))
    .unwrap();
    for m in [&doc, &oas] {
        assert_eq!(
            [
                m.total_margin,
                m.span_margin,
                m.exposure_margin,
                m.equity_margin,
                m.fo_margin,
                m.commodity_margin,
                m.currency
            ],
            [10.5, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5].map(Some)
        );
    }
    assert_eq!((doc.hedge_benefit, oas.hedge_benefit), (None, Some(7.5)));
    assert_eq!(
        doc.client_id.as_ref().map(|c| c.expose_secret()),
        Some("1000000009")
    );
    assert!(!format!("{doc:?}").contains("1000000009"));
}

#[test]
fn margin_keeps_leverage_as_a_string() {
    let m: Margin = serde_json::from_value(json!({
        "totalMargin": 2800.0,
        "spanMargin": 1200.0,
        "exposureMargin": 1003.0,
        "availableBalance": 10500.0,
        "variableMargin": 1000.0,
        "insufficientBalance": 0.0,
        "brokerage": 20.0,
        "leverage": "4.00"
    }))
    .unwrap();
    assert_eq!(m.leverage.as_deref(), Some("4.00"));
    assert_eq!(
        (m.total_margin, m.exposure_margin, m.brokerage),
        (Some(2800.0), Some(1003.0), Some(20.0))
    );
}

#[test]
fn a_margin_request_body_uses_the_wire_names() {
    assert_eq!(
        serde_json::to_value(leg().with_trigger_price(1427.0)).unwrap(),
        json!({
            "exchangeSegment": "NSE_EQ",
            "transactionType": "BUY",
            "quantity": 5,
            "productType": "CNC",
            "securityId": "1333",
            "price": 1428.0,
            "triggerPrice": 1427.0
        })
    );
    assert!(
        serde_json::to_value(leg())
            .unwrap()
            .get("triggerPrice")
            .is_none()
    );
}

#[test]
fn a_multi_margin_body_uses_include_order_and_scrip_list() {
    let body =
        serde_json::to_value(MultiMarginRequest::new([leg()]).with_include_order(true)).unwrap();
    assert_eq!(body["includePosition"], json!(false));
    assert_eq!(body["includeOrder"], json!(true));
    assert_eq!(body["scripList"][0]["securityId"], json!("1333"));
    for key in ["includeOrders", "scripts"] {
        assert!(body.get(key).is_none(), "{key}");
    }
}

#[test]
fn margin_request_validation() {
    assert_eq!(leg().validate(), Ok(()));
    let mut req = leg();
    req.quantity = 0;
    assert_eq!(
        req.validate(),
        invalid("quantity", ValidationReason::NotPositive)
    );
    for segment in [ExchangeSegment::IdxI, ExchangeSegment::InxEq] {
        let mut req = leg();
        req.exchange_segment = segment;
        assert_eq!(
            req.validate(),
            invalid("exchange_segment", ValidationReason::UnknownEnumValue)
        );
    }
    let mut req = leg();
    req.product_type = ProductType::Bo;
    assert_eq!(
        req.validate(),
        invalid("product_type", ValidationReason::UnknownEnumValue)
    );
    let mut req = leg();
    req.price = f64::NAN;
    assert_eq!(
        req.validate(),
        invalid("price", ValidationReason::NotFinite)
    );
    assert_eq!(
        leg().with_trigger_price(-1.0).validate(),
        invalid("trigger_price", ValidationReason::OutOfRange)
    );
}

#[test]
fn a_multi_margin_request_holds_one_to_fifty_legs() {
    assert_eq!(
        MultiMarginRequest::new([]).validate(),
        invalid("scrip_list", ValidationReason::Empty)
    );
    assert_eq!(MultiMarginRequest::new(vec![leg(); 50]).validate(), Ok(()));
    assert_eq!(
        MultiMarginRequest::new(vec![leg(); 51]).validate(),
        invalid("scrip_list", ValidationReason::TooMany { max: 50 })
    );
    let mut bad = leg();
    bad.quantity = 0;
    assert_eq!(
        MultiMarginRequest::new([leg(), bad]).validate(),
        invalid("quantity", ValidationReason::NotPositive)
    );
}
