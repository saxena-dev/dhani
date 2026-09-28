use serde_json::json;

use super::*;

fn security() -> SecurityId {
    SecurityId::new("1333").unwrap()
}

fn limit(price: Option<f64>) -> PlaceOrderRequest {
    let req = PlaceOrderRequest::new(
        ExchangeSegment::NseEq,
        security(),
        TransactionType::Buy,
        10,
        OrderType::Limit,
        ProductType::Cnc,
        Validity::Day,
    );
    match price {
        Some(p) => req.with_price(p),
        None => req,
    }
}

fn of_type(order_type: OrderType) -> PlaceOrderRequest {
    PlaceOrderRequest::new(
        ExchangeSegment::NseEq,
        security(),
        TransactionType::Sell,
        5,
        order_type,
        ProductType::Intraday,
        Validity::Day,
    )
}

fn invalid(field: &'static str, reason: ValidationReason) -> Result<(), ValidationError> {
    Err(ValidationError { field, reason })
}

// ---- PlaceOrderRequest validation --------------------------------------------------------

#[test]
fn a_limit_order_without_a_price_is_refused() {
    assert_eq!(
        limit(None).validate(),
        invalid("price", ValidationReason::Missing)
    );
    assert_eq!(
        limit(Some(0.0)).validate(),
        invalid("price", ValidationReason::NotPositive)
    );
    assert_eq!(limit(Some(1642.5)).validate(), Ok(()));
}

#[test]
fn stop_loss_orders_need_a_trigger_price() {
    assert_eq!(
        of_type(OrderType::StopLoss).with_price(100.0).validate(),
        invalid("trigger_price", ValidationReason::Missing)
    );
    assert_eq!(
        of_type(OrderType::StopLossMarket)
            .with_trigger_price(0.0)
            .validate(),
        invalid("trigger_price", ValidationReason::NotPositive)
    );
    // A stop-loss limit order also needs its price.
    assert_eq!(
        of_type(OrderType::StopLoss)
            .with_trigger_price(99.0)
            .validate(),
        invalid("price", ValidationReason::Missing)
    );
    assert_eq!(
        of_type(OrderType::StopLoss)
            .with_price(100.0)
            .with_trigger_price(99.0)
            .validate(),
        Ok(())
    );
    assert_eq!(
        of_type(OrderType::StopLossMarket)
            .with_trigger_price(99.0)
            .validate(),
        Ok(())
    );
    assert_eq!(of_type(OrderType::Market).validate(), Ok(()));
}

#[test]
fn an_amo_time_without_the_amo_flag_is_inconsistent() {
    let mut req = limit(Some(10.0));
    req.amo_time = Some(AmoTime::Open);
    assert_eq!(
        req.validate(),
        invalid(
            "amo_time",
            ValidationReason::Inconsistent("requires after_market_order = true")
        )
    );
    req.after_market_order = Some(false);
    assert_eq!(
        req.validate(),
        invalid(
            "amo_time",
            ValidationReason::Inconsistent("requires after_market_order = true")
        )
    );
    // PRE_OPEN is accepted, as the documentation lists it; the Python SDK does not.
    assert_eq!(
        limit(Some(10.0)).with_amo(AmoTime::PreOpen).validate(),
        Ok(())
    );
}

#[test]
fn a_31_character_correlation_id_is_refused() {
    let long = "a".repeat(31);
    assert_eq!(
        CorrelationId::new(long.clone()),
        Err(ValidationError {
            field: "correlation_id",
            reason: ValidationReason::TooLong { max: 30 }
        })
    );
    // One taken from a response is rechecked when reused in a request.
    let reused: CorrelationId = serde_json::from_value(json!(long)).unwrap();
    assert_eq!(
        limit(Some(10.0)).with_correlation_id(reused).validate(),
        invalid("correlation_id", ValidationReason::TooLong { max: 30 })
    );
    let ok = CorrelationId::new("a".repeat(30)).unwrap();
    assert_eq!(limit(Some(10.0)).with_correlation_id(ok).validate(), Ok(()));
}

#[test]
fn cover_and_bracket_products_cannot_be_placed() {
    for product in [ProductType::Co, ProductType::Bo] {
        let mut req = limit(Some(10.0));
        req.product_type = product;
        assert_eq!(
            req.validate(),
            invalid("product_type", ValidationReason::UnknownEnumValue)
        );
    }
}

#[test]
fn index_and_global_segments_cannot_be_traded() {
    for segment in [ExchangeSegment::IdxI, ExchangeSegment::InxEq] {
        let mut req = limit(Some(10.0));
        req.exchange_segment = segment;
        assert_eq!(
            req.validate(),
            invalid("exchange_segment", ValidationReason::UnknownEnumValue)
        );
    }
}

#[test]
fn quantities_and_prices_are_range_checked() {
    let mut req = limit(Some(10.0));
    req.quantity = 0;
    assert_eq!(
        req.validate(),
        invalid("quantity", ValidationReason::NotPositive)
    );
    assert_eq!(
        limit(Some(10.0)).with_disclosed_quantity(11).validate(),
        invalid(
            "disclosed_quantity",
            ValidationReason::Inconsistent("must not exceed quantity")
        )
    );
    assert_eq!(
        limit(Some(10.0)).with_disclosed_quantity(10).validate(),
        Ok(())
    );
    assert_eq!(
        limit(Some(f64::NAN)).validate(),
        invalid("price", ValidationReason::NotFinite)
    );
    assert_eq!(
        limit(Some(-1.0)).validate(),
        invalid("price", ValidationReason::OutOfRange)
    );
    assert_eq!(
        of_type(OrderType::Market)
            .with_trigger_price(f64::INFINITY)
            .validate(),
        invalid("trigger_price", ValidationReason::NotFinite)
    );
    let placeholder: SecurityId = serde_json::from_value(json!("string!")).unwrap();
    let mut req = limit(Some(10.0));
    req.security_id = placeholder;
    assert_eq!(
        req.validate(),
        invalid("security_id", ValidationReason::InvalidCharacters)
    );
}

#[test]
fn slicing_requires_a_price() {
    assert_eq!(
        of_type(OrderType::Market).validate_for_slice(),
        invalid("price", ValidationReason::Missing)
    );
    assert_eq!(limit(Some(10.0)).validate_for_slice(), Ok(()));
    // The ordinary rules still apply first.
    assert_eq!(
        limit(Some(-1.0)).validate_for_slice(),
        invalid("price", ValidationReason::OutOfRange)
    );
}

// ---- Request bodies ------------------------------------------------------------------------

#[test]
fn a_market_order_sends_no_price_or_bracket_keys() {
    let body = serde_json::to_value(of_type(OrderType::Market)).unwrap();
    assert_eq!(
        body,
        json!({
            "transactionType": "SELL",
            "exchangeSegment": "NSE_EQ",
            "productType": "INTRADAY",
            "orderType": "MARKET",
            "validity": "DAY",
            "securityId": "1333",
            "quantity": 5
        })
    );
    for key in ["price", "triggerPrice", "boProfitValue", "boStopLossValue"] {
        assert!(body.get(key).is_none(), "{key}");
    }
}

#[test]
fn with_amo_sets_both_amo_keys() {
    let body = serde_json::to_value(limit(Some(10.0)).with_amo(AmoTime::PreOpen)).unwrap();
    assert_eq!(body["afterMarketOrder"], json!(true));
    assert_eq!(body["amoTime"], json!("PRE_OPEN"));
}

#[test]
fn every_place_order_field_uses_its_wire_name() {
    let req = of_type(OrderType::StopLoss)
        .with_price(100.5)
        .with_trigger_price(99.5)
        .with_disclosed_quantity(2)
        .with_correlation_id(CorrelationId::new("run-7_a").unwrap())
        .with_amo(AmoTime::Open30);
    assert_eq!(
        serde_json::to_value(req).unwrap(),
        json!({
            "correlationId": "run-7_a",
            "transactionType": "SELL",
            "exchangeSegment": "NSE_EQ",
            "productType": "INTRADAY",
            "orderType": "STOP_LOSS",
            "validity": "DAY",
            "securityId": "1333",
            "quantity": 5,
            "disclosedQuantity": 2,
            "price": 100.5,
            "triggerPrice": 99.5,
            "afterMarketOrder": true,
            "amoTime": "OPEN_30"
        })
    );
}

#[test]
fn a_modify_request_sends_only_what_is_set() {
    let order_id = OrderId::new("112111182198").unwrap();
    let req =
        ModifyOrderRequest::new(order_id.clone(), OrderType::Limit, Validity::Day).with_price(1.0);
    assert_eq!(
        serde_json::to_value(&req).unwrap(),
        json!({"orderId": "112111182198", "orderType": "LIMIT", "validity": "DAY", "price": 1.0})
    );
    let full = ModifyOrderRequest::new(order_id, OrderType::StopLoss, Validity::Ioc)
        .with_quantity(3)
        .with_price(2.0)
        .with_disclosed_quantity(1)
        .with_trigger_price(1.5)
        .with_leg_name(LegName::StopLossLeg);
    assert_eq!(
        serde_json::to_value(&full).unwrap(),
        json!({
            "orderId": "112111182198",
            "orderType": "STOP_LOSS",
            "validity": "IOC",
            "quantity": 3,
            "price": 2.0,
            "disclosedQuantity": 1,
            "triggerPrice": 1.5,
            "legName": "STOP_LOSS_LEG"
        })
    );
}

#[test]
fn modify_validation() {
    let placeholder: OrderId = serde_json::from_value(json!("string!")).unwrap();
    assert_eq!(
        ModifyOrderRequest::new(placeholder, OrderType::Limit, Validity::Day).validate(),
        invalid("order_id", ValidationReason::InvalidCharacters)
    );
    let order_id = OrderId::new("1").unwrap();
    let base = || ModifyOrderRequest::new(order_id.clone(), OrderType::Limit, Validity::Day);
    assert_eq!(
        base().with_quantity(0).validate(),
        invalid("quantity", ValidationReason::NotPositive)
    );
    assert_eq!(
        base().with_price(-0.5).validate(),
        invalid("price", ValidationReason::OutOfRange)
    );
    assert_eq!(
        base().with_trigger_price(f64::NAN).validate(),
        invalid("trigger_price", ValidationReason::NotFinite)
    );
    assert_eq!(base().with_quantity(1).with_price(0.0).validate(), Ok(()));
    // Disclosed quantity is checked against quantity only when both are set.
    assert_eq!(
        base()
            .with_quantity(1)
            .with_disclosed_quantity(100)
            .validate(),
        invalid(
            "disclosed_quantity",
            ValidationReason::Inconsistent("must not exceed quantity")
        )
    );
    assert_eq!(base().with_disclosed_quantity(100).validate(), Ok(()));
}

// ---- Responses -----------------------------------------------------------------------------

#[test]
fn an_ack_accepts_both_order_id_spellings() {
    for key in ["orderId", "order-id"] {
        let ack: OrderAck =
            serde_json::from_value(json!({key: "x", "orderStatus": "TRANSIT"})).unwrap();
        assert_eq!(ack.order_id.as_ref(), "x");
        assert_eq!(ack.order_status, Some(Inbound::Known(OrderStatus::Transit)));
    }
    // The order ID is the ack's identity: a body without it is a decode error.
    assert!(serde_json::from_value::<OrderAck>(json!({"orderStatus": "TRANSIT"})).is_err());
}

#[test]
fn slice_acks_decode_from_an_object_or_an_array() {
    let one: SlicedAcks =
        serde_json::from_value(json!({"orderId": "1", "orderStatus": "PENDING"})).unwrap();
    assert_eq!(one.0.len(), 1);
    let many: SlicedAcks = serde_json::from_value(json!([
        {"orderId": "1", "orderStatus": "TRANSIT"},
        {"orderId": "2", "orderStatus": "TRANSIT"}
    ]))
    .unwrap();
    let ids: Vec<&str> = many.0.iter().map(|a| a.order_id.as_ref()).collect();
    assert_eq!(ids, ["1", "2"]);
    // An empty or null list decodes as no acks, like every response collection.
    for empty in [json!([]), json!(null)] {
        let none: SlicedAcks = serde_json::from_value(empty).unwrap();
        assert!(none.0.is_empty());
    }
}

#[test]
fn an_order_accepts_both_order_id_spellings() {
    for key in ["orderId", "order-id"] {
        let order: Order = serde_json::from_value(json!({key: "x"})).unwrap();
        assert_eq!(order.order_id.as_ref(), "x");
        assert_eq!(order.order_status, None);
    }
    let order: Order =
        serde_json::from_value(json!({"orderId": "x", "correlation-id": "c-1"})).unwrap();
    assert_eq!(order.correlation_id.as_deref(), Some("c-1"));
    assert!(serde_json::from_value::<Order>(json!({"orderStatus": "TRADED"})).is_err());
}

#[test]
fn average_traded_price_accepts_a_number_or_a_string() {
    for value in [json!("1.5"), json!(1.5)] {
        let order: Order =
            serde_json::from_value(json!({"orderId": "x", "averageTradedPrice": value})).unwrap();
        assert_eq!(order.average_traded_price, Some(1.5));
    }
}

#[test]
fn na_option_type_and_null_leg_name_are_none() {
    let order: Order = serde_json::from_value(json!({
        "orderId": "x",
        "drvOptionType": "NA",
        "legName": null
    }))
    .unwrap();
    assert_eq!((order.drv_option_type, order.leg_name), (None, None));
    let order: Order = serde_json::from_value(json!({
        "orderId": "x",
        "drvOptionType": "PUT",
        "legName": "TARGET_LEG"
    }))
    .unwrap();
    assert_eq!(order.drv_option_type, Some(Inbound::Known(OptionType::Put)));
    assert_eq!(order.leg_name, Some(Inbound::Known(LegName::TargetLeg)));
}

#[test]
fn the_documented_order_decodes_every_key() {
    // The keys of the documentation's order object, with the fixture's extreme values.
    let order: Order = serde_json::from_value(json!({
        "dhanClientId": "1000000009",
        "orderId": "112111182198",
        "exchangeOrderId": "1400000000404591",
        "correlationId": "123abc678",
        "orderStatus": "PENDING",
        "transactionType": "BUY",
        "exchangeSegment": "NSE_EQ",
        "productType": "INTRADAY",
        "orderType": "MARKET",
        "validity": "DAY",
        "tradingSymbol": "HDFCBANK",
        "securityId": "1333",
        "quantity": -2147483648i64,
        "disclosedQuantity": 0,
        "price": -3.402823669209385e38,
        "triggerPrice": 0.0,
        "afterMarketOrder": false,
        "boProfitValue": 0.0,
        "boStopLossValue": 0.0,
        "legName": "ENTRY_LEG",
        "createTime": "2021-11-24 13:33:03",
        "updateTime": "2021-11-24 13:33:03",
        "exchangeTime": "2021-11-24 13:33:03",
        "drvExpiryDate": null,
        "drvOptionType": null,
        "drvStrikePrice": 0.0,
        "omsErrorCode": null,
        "omsErrorDescription": null,
        "algoId": "0",
        "remainingQuantity": 5,
        "averageTradedPrice": 0,
        "filledQty": 0,
        "someFutureKey": {"ignored": true}
    }))
    .unwrap();
    assert_eq!(
        order
            .dhan_client_id
            .as_ref()
            .map(|c| c.expose_secret().to_owned()),
        Some("1000000009".to_owned())
    );
    assert_eq!(order.exchange_order_id.as_deref(), Some("1400000000404591"));
    assert_eq!(
        order.order_status,
        Some(Inbound::Known(OrderStatus::Pending))
    );
    assert_eq!(
        order.product_type,
        Some(Inbound::Known(ProductType::Intraday))
    );
    assert_eq!(order.security_id, Some(SecurityId::new("1333").unwrap()));
    assert_eq!(order.quantity, Some(-2_147_483_648));
    assert_eq!(order.price, Some(-3.402823669209385e38));
    assert_eq!(order.after_market_order, Some(false));
    assert_eq!(order.leg_name, Some(Inbound::Known(LegName::EntryLeg)));
    assert_eq!(
        order.create_time.as_ref().map(WireTime::as_str),
        Some("2021-11-24 13:33:03")
    );
    assert_eq!((order.drv_expiry_date, order.drv_option_type), (None, None));
    assert_eq!(order.algo_id.as_deref(), Some("0"));
    assert_eq!(
        (
            order.remaining_quantity,
            order.average_traded_price,
            order.filled_qty
        ),
        (Some(5), Some(0.0), Some(0))
    );
    // The client ID is redacted in Debug.
    assert!(!format!("{:?}", order.dhan_client_id).contains("1000000009"));
}

#[test]
fn an_unknown_status_is_kept() {
    let order: Order =
        serde_json::from_value(json!({"orderId": "x", "orderStatus": "PARKED"})).unwrap();
    assert_eq!(
        order.order_status.as_ref().map(Inbound::as_wire),
        Some("PARKED")
    );
    assert_eq!(order.order_status.and_then(|s| s.known()), None);
}

#[test]
fn a_trade_decodes_with_its_custom_symbol() {
    let trade: Trade = serde_json::from_value(json!({
        "dhanClientId": "1000000009",
        "orderId": "112111182198",
        "exchangeOrderId": "15112111182938",
        "exchangeTradeId": "15112111182938",
        "transactionType": "BUY",
        "exchangeSegment": "NSE_EQ",
        "productType": "INTRADAY",
        "orderType": "LIMIT",
        "tradingSymbol": "TCS",
        "customSymbol": "Tata Consultancy Services",
        "securityId": "11536",
        "tradedQuantity": 40,
        "tradedPrice": 3345.8,
        "createTime": "2021-03-10 11:20:06",
        "updateTime": "2021-11-25 17:35:12",
        "exchangeTime": "2021-11-25 17:35:12",
        "drvExpiryDate": "NA",
        "drvOptionType": "NA",
        "drvStrikePrice": 0.0
    }))
    .unwrap();
    assert_eq!(trade.order_id, Some(OrderId::new("112111182198").unwrap()));
    assert_eq!(
        trade.custom_symbol.as_deref(),
        Some("Tata Consultancy Services")
    );
    assert_eq!(
        (trade.traded_quantity, trade.traded_price),
        (Some(40), Some(3345.8))
    );
    assert_eq!(trade.order_type, Some(Inbound::Known(OrderType::Limit)));
    assert_eq!(trade.drv_option_type, None);
    assert_eq!(
        trade.drv_expiry_date.as_ref().map(WireTime::as_str),
        Some("NA")
    );
    let trade: Trade = serde_json::from_value(json!({"order-id": "9"})).unwrap();
    assert_eq!(trade.order_id, Some(OrderId::new("9").unwrap()));
}
