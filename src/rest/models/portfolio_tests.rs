use serde_json::json;

use super::*;

fn convert() -> ConvertPositionRequest {
    ConvertPositionRequest::new(
        ProductType::Intraday,
        ProductType::Cnc,
        ExchangeSegment::NseEq,
        PositionType::Long,
        SecurityId::new("11536").unwrap(),
        40,
    )
}

fn invalid(field: &'static str, reason: ValidationReason) -> Result<(), ValidationError> {
    Err(ValidationError { field, reason })
}

#[test]
fn a_valid_conversion_passes() {
    assert_eq!(convert().validate(), Ok(()));
    assert_eq!(convert().with_trading_symbol("TCS").validate(), Ok(()));
    for product in [ProductType::Cnc, ProductType::Margin] {
        let mut req = convert();
        req.to_product_type = product;
        assert_eq!(req.validate(), Ok(()));
    }
}

#[test]
fn a_conversion_to_the_same_product_is_inconsistent() {
    let mut req = convert();
    req.to_product_type = ProductType::Intraday;
    assert_eq!(
        req.validate(),
        invalid(
            "to_product_type",
            ValidationReason::Inconsistent("must differ from from_product_type")
        )
    );
}

#[test]
fn index_and_global_segments_cannot_convert() {
    for segment in [ExchangeSegment::IdxI, ExchangeSegment::InxEq] {
        let mut req = convert();
        req.exchange_segment = segment;
        assert_eq!(
            req.validate(),
            invalid("exchange_segment", ValidationReason::UnknownEnumValue)
        );
    }
}

#[test]
fn a_zero_quantity_is_refused() {
    let mut req = convert();
    req.convert_qty = 0;
    assert_eq!(
        req.validate(),
        invalid("convert_qty", ValidationReason::NotPositive)
    );
}

#[test]
fn only_cnc_intraday_and_margin_convert() {
    for product in [ProductType::Co, ProductType::Bo, ProductType::Mtf] {
        let mut req = convert();
        req.to_product_type = product;
        assert_eq!(
            req.validate(),
            invalid("to_product_type", ValidationReason::UnknownEnumValue)
        );
        let mut req = convert();
        req.from_product_type = product;
        assert_eq!(
            req.validate(),
            invalid("from_product_type", ValidationReason::UnknownEnumValue)
        );
    }
}

#[test]
fn an_empty_trading_symbol_or_invalid_security_is_refused() {
    for blank in ["", "  "] {
        assert_eq!(
            convert().with_trading_symbol(blank).validate(),
            invalid("trading_symbol", ValidationReason::Empty)
        );
    }
    let mut req = convert();
    req.security_id = serde_json::from_value(json!("a b")).unwrap();
    assert_eq!(
        req.validate(),
        invalid("security_id", ValidationReason::InvalidCharacters)
    );
}

#[test]
fn the_body_omits_an_unset_trading_symbol() {
    assert_eq!(
        serde_json::to_value(convert()).unwrap(),
        json!({
            "fromProductType": "INTRADAY",
            "exchangeSegment": "NSE_EQ",
            "positionType": "LONG",
            "securityId": "11536",
            "convertQty": 40,
            "toProductType": "CNC"
        })
    );
    let body = serde_json::to_value(convert().with_trading_symbol("TCS")).unwrap();
    assert_eq!(body["tradingSymbol"], json!("TCS"));
}

#[test]
fn a_holding_decodes_the_snake_case_mtf_keys() {
    let holding: Holding = serde_json::from_value(json!({
        "exchange": "ALL",
        "tradingSymbol": "HDFC",
        "securityId": "1330",
        "isin": "INE001A01036",
        "totalQty": 1000,
        "dpQty": 1000,
        "t1Qty": 0,
        "mtf_t1_qty": 2,
        "mtf_qty": 3,
        "availableQty": 1000,
        "collateralQty": 0,
        "avgCostPrice": 2655.0,
        "lastTradedPrice": 2710.25
    }))
    .unwrap();
    assert_eq!(holding.exchange.as_deref(), Some("ALL"));
    assert_eq!(holding.isin, Some(Isin::new("INE001A01036").unwrap()));
    assert_eq!((holding.mtf_t1_qty, holding.mtf_qty), (Some(2), Some(3)));
    assert_eq!(holding.total_qty, Some(1000));
    assert_eq!(holding.last_traded_price, Some(2710.25));
    // The camelCase spelling is not the wire name.
    let holding: Holding = serde_json::from_value(json!({"mtfT1Qty": 2})).unwrap();
    assert_eq!(holding.mtf_t1_qty, None);
}

#[test]
fn a_position_decodes_na_option_types() {
    let position: Position = serde_json::from_value(json!({
        "positionType": "SHORT",
        "drvOptionType": "NA",
        "netQty": -25,
        "crossCurrency": false
    }))
    .unwrap();
    assert_eq!(
        position.position_type,
        Some(Inbound::Known(PositionType::Short))
    );
    assert_eq!(position.drv_option_type, None);
    assert_eq!(
        (position.net_qty, position.cross_currency),
        (Some(-25), Some(false))
    );
}

#[test]
fn every_position_key_decodes_into_its_field() {
    // Distinct non-zero values, so a misspelled key cannot pass as a default.
    let p: Position = serde_json::from_value(json!({
        "dhanClientId": "1000000009",
        "tradingSymbol": "TCS",
        "securityId": "11536",
        "positionType": "LONG",
        "exchangeSegment": "NSE_FNO",
        "productType": "MARGIN",
        "buyAvg": 1.5,
        "costPrice": 2.5,
        "buyQty": 3,
        "sellAvg": 4.5,
        "sellQty": 5,
        "netQty": 6,
        "realizedProfit": 7.5,
        "unrealizedProfit": 8.5,
        "rbiReferenceRate": 9.5,
        "multiplier": 10,
        "carryForwardBuyQty": 11,
        "carryForwardSellQty": 12,
        "carryForwardBuyValue": 13.5,
        "carryForwardSellValue": 14.5,
        "dayBuyQty": 15,
        "daySellQty": 16,
        "dayBuyValue": 17.5,
        "daySellValue": 18.5,
        "drvExpiryDate": "2026-10-29",
        "drvOptionType": "PUT",
        "drvStrikePrice": 19.5,
        "crossCurrency": true
    }))
    .unwrap();
    assert_eq!(
        p.dhan_client_id.as_ref().map(|c| c.expose_secret()),
        Some("1000000009")
    );
    assert_eq!(p.trading_symbol.as_deref(), Some("TCS"));
    assert_eq!(p.security_id, Some(SecurityId::new("11536").unwrap()));
    assert_eq!(p.position_type, Some(Inbound::Known(PositionType::Long)));
    assert_eq!(
        p.exchange_segment,
        Some(Inbound::Known(ExchangeSegment::NseFno))
    );
    assert_eq!(p.product_type, Some(Inbound::Known(ProductType::Margin)));
    assert_eq!(
        [
            p.buy_avg,
            p.cost_price,
            p.sell_avg,
            p.realized_profit,
            p.unrealized_profit,
            p.rbi_reference_rate,
            p.carry_forward_buy_value,
            p.carry_forward_sell_value,
            p.day_buy_value,
            p.day_sell_value,
            p.drv_strike_price
        ],
        [1.5, 2.5, 4.5, 7.5, 8.5, 9.5, 13.5, 14.5, 17.5, 18.5, 19.5].map(Some)
    );
    assert_eq!(
        [
            p.buy_qty,
            p.sell_qty,
            p.net_qty,
            p.multiplier,
            p.carry_forward_buy_qty,
            p.carry_forward_sell_qty,
            p.day_buy_qty,
            p.day_sell_qty
        ],
        [3, 5, 6, 10, 11, 12, 15, 16].map(Some)
    );
    assert_eq!(
        p.drv_expiry_date.as_ref().map(WireTime::as_str),
        Some("2026-10-29")
    );
    assert_eq!(p.drv_option_type, Some(Inbound::Known(OptionType::Put)));
    assert_eq!(p.cross_currency, Some(true));
    // The client ID is redacted in Debug.
    assert!(!format!("{p:?}").contains("1000000009"));
}
