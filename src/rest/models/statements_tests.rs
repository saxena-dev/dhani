use serde_json::json;

use super::*;

#[test]
fn a_ledger_entry_uses_lowercase_keys_and_string_amounts() {
    let entry: LedgerEntry = serde_json::from_value(json!({
        "dhanClientId": "1000000009",
        "narration": "FUNDS WITHDRAWAL",
        "voucherdate": "Jun 22, 2022",
        "exchange": "NSE-CAPITAL",
        "voucherdesc": "PAYBNK",
        "vouchernumber": "202200036701",
        "debit": "20000.00",
        "credit": "0.00",
        "runbal": " -957.29 "
    }))
    .unwrap();
    assert_eq!(entry.narration.as_deref(), Some("FUNDS WITHDRAWAL"));
    assert_eq!(
        entry.voucherdate.as_ref().map(WireTime::as_str),
        Some("Jun 22, 2022")
    );
    assert_eq!(entry.vouchernumber.as_deref(), Some("202200036701"));
    assert_eq!(entry.debit.as_deref(), Some("20000.00"));
    assert_eq!(
        (entry.debit_f64(), entry.credit_f64(), entry.runbal_f64()),
        (Some(20000.0), Some(0.0), Some(-957.29))
    );
    assert!(!format!("{entry:?}").contains("1000000009"));
}

#[test]
fn unparsable_amounts_are_none() {
    let entry: LedgerEntry =
        serde_json::from_value(json!({"debit": "string", "credit": "NaN", "runbal": ""})).unwrap();
    assert_eq!(
        (entry.debit_f64(), entry.credit_f64(), entry.runbal_f64()),
        (None, None, None)
    );
}

#[test]
fn a_ledger_is_one_entry_or_many() {
    let one: LedgerEntries = serde_json::from_value(json!({"narration": "a"})).unwrap();
    assert_eq!(one.0.len(), 1);
    let many: LedgerEntries =
        serde_json::from_value(json!([{"narration": "a"}, {"narration": "b"}])).unwrap();
    let narrations: Vec<_> = many.0.iter().map(|e| e.narration.as_deref()).collect();
    assert_eq!(narrations, [Some("a"), Some("b")]);
}

#[test]
fn trade_history_charges_decode_from_numbers_and_strings() {
    let numbers: HistoricalTrade = serde_json::from_value(json!({
        "sebiTax": 0.01,
        "stt": 1.5,
        "brokerageCharges": 20,
        "serviceTax": 3.6,
        "exchangeTransactionCharges": 0.5,
        "stampDuty": 0.15
    }))
    .unwrap();
    let strings: HistoricalTrade = serde_json::from_value(json!({
        "sebiTax": "0.01",
        "stt": "1.5",
        "brokerageCharges": "20",
        "serviceTax": "3.6",
        "exchangeTransactionCharges": "0.5",
        "stampDuty": "0.15"
    }))
    .unwrap();
    for t in [&numbers, &strings] {
        assert_eq!(
            [
                t.sebi_tax,
                t.stt,
                t.brokerage_charges,
                t.service_tax,
                t.exchange_transaction_charges,
                t.stamp_duty
            ],
            [0.01, 1.5, 20.0, 3.6, 0.5, 0.15].map(Some)
        );
    }
}

#[test]
fn every_trade_history_key_decodes() {
    let t: HistoricalTrade = serde_json::from_value(json!({
        "dhanClientId": "1000000009",
        "orderId": "212212307731",
        "exchangeOrderId": "76036896",
        "exchangeTradeId": "407958",
        "transactionType": "SELL",
        "exchangeSegment": "NSE_FNO",
        "productType": "MARGIN",
        "orderType": "MARKET",
        "tradingSymbol": "NIFTY-Dec2022-18600-CE",
        "customSymbol": "Nifty 29 Dec 18600 Call",
        "securityId": "39273",
        "tradedQuantity": 50,
        "tradedPrice": 11.25,
        "isin": "NA",
        "instrument": "OPTIDX",
        "sebiTax": "0.0002",
        "stt": "0",
        "brokerageCharges": "0",
        "serviceTax": "0.0025",
        "exchangeTransactionCharges": "0.0147",
        "stampDuty": "0",
        "createTime": "2022-12-30 10:00:46",
        "updateTime": "NA",
        "exchangeTime": "2022-12-30 10:00:46",
        "drvExpiryDate": 1672338600,
        "drvOptionType": "CALL",
        "drvStrikePrice": 18600
    }))
    .unwrap();
    assert_eq!(t.order_id, Some(OrderId::new("212212307731").unwrap()));
    assert_eq!(t.exchange_trade_id.as_deref(), Some("407958"));
    assert_eq!(
        t.transaction_type,
        Some(Inbound::Known(TransactionType::Sell))
    );
    assert_eq!(
        t.exchange_segment,
        Some(Inbound::Known(ExchangeSegment::NseFno))
    );
    assert_eq!(t.product_type, Some(Inbound::Known(ProductType::Margin)));
    assert_eq!(t.order_type, Some(Inbound::Known(OrderType::Market)));
    assert_eq!(t.trading_symbol.as_deref(), Some("NIFTY-Dec2022-18600-CE"));
    assert_eq!(t.custom_symbol.as_deref(), Some("Nifty 29 Dec 18600 Call"));
    assert_eq!(t.security_id, Some(SecurityId::new("39273").unwrap()));
    assert_eq!((t.traded_quantity, t.traded_price), (Some(50), Some(11.25)));
    assert_eq!(t.isin.as_ref().map(|i| i.as_ref()), Some("NA"));
    assert_eq!(t.instrument.as_deref(), Some("OPTIDX"));
    assert_eq!(
        (t.sebi_tax, t.service_tax, t.exchange_transaction_charges),
        (Some(0.0002), Some(0.0025), Some(0.0147))
    );
    assert_eq!(
        t.create_time.as_ref().map(WireTime::as_str),
        Some("2022-12-30 10:00:46")
    );
    assert_eq!(t.update_time.as_ref().map(WireTime::as_str), Some("NA"));
    // A numeric expiry is kept as its text.
    assert_eq!(
        t.drv_expiry_date.as_ref().map(WireTime::as_str),
        Some("1672338600")
    );
    assert_eq!(t.drv_option_type, Some(Inbound::Known(OptionType::Call)));
    assert_eq!(t.drv_strike_price, Some(18600.0));
    assert!(!format!("{t:?}").contains("1000000009"));
}
