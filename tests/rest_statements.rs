//! Statements facade contract tests (§9 rows T1–T2): method, exact path and query, credential
//! headers; fixtures decoded to hand-written values. The ledger decodes one object or an array;
//! trade-history charges decode from numbers and strings. A reversed date range sends nothing.

mod support;

use chrono::NaiveDate;
use dhani::error::ValidationReason;
use dhani::types::{ExchangeSegment, Inbound, OptionType, ProductType, TransactionType};
use dhani::{DhanClient, ErrorKind};
use serde_json::json;
use support::fixtures::upstream_payload;
use support::mock::{client_for, expect_headers};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn json_reply(body: Vec<u8>) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_bytes(body)
}

async fn ledger_server(reply: Vec<u8>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/ledger"))
        .and(query_param("from-date", "2024-01-01"))
        .and(query_param("to-date", "2024-01-31"))
        .and(expect_headers())
        .respond_with(json_reply(reply))
        .expect(1)
        .mount(&server)
        .await;
    server
}

async fn history_server(reply: Vec<u8>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/trades/2024-01-01/2024-01-31/0"))
        .and(expect_headers())
        .respond_with(json_reply(reply))
        .expect(1)
        .mount(&server)
        .await;
    server
}

// ---- T1 --------------------------------------------------------------------------------------

// row: T1
#[tokio::test]
async fn t1_ledger_from_the_object_fixture() {
    let server = ledger_server(upstream_payload("get_ledger_report.json")).await;
    let entries = client_for(&server)
        .statements()
        .ledger(date(2024, 1, 1), date(2024, 1, 31))
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e.narration.as_deref(), Some("string"));
    assert_eq!(e.voucherdate.as_ref().map(|t| t.as_str()), Some("string"));
    assert_eq!(e.exchange.as_deref(), Some("string"));
    assert_eq!(e.voucherdesc.as_deref(), Some("string"));
    assert_eq!(e.vouchernumber.as_deref(), Some("string"));
    assert_eq!(
        (e.debit.as_deref(), e.credit.as_deref(), e.runbal.as_deref()),
        (Some("string"), Some("string"), Some("string"))
    );
    // The placeholders are not numbers.
    assert_eq!(
        (e.debit_f64(), e.credit_f64(), e.runbal_f64()),
        (None, None, None)
    );
    assert!(e.dhan_client_id.is_some());
    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(
        received[0].url.query(),
        Some("from-date=2024-01-01&to-date=2024-01-31")
    );
}

// row: T1
#[tokio::test]
async fn t1_ledger_from_an_array() {
    let reply = json!([
        {"narration": "FUNDS DEPOSIT", "debit": "0.00", "credit": "5000.00", "runbal": "5000.00"},
        {"narration": "FUNDS WITHDRAWAL", "debit": "1500.50", "credit": "0.00", "runbal": "3499.50"}
    ]);
    let server = ledger_server(serde_json::to_vec(&reply).unwrap()).await;
    let entries = client_for(&server)
        .statements()
        .ledger(date(2024, 1, 1), date(2024, 1, 31))
        .await
        .unwrap();
    let decoded: Vec<_> = entries
        .iter()
        .map(|e| (e.narration.as_deref(), e.debit_f64(), e.runbal_f64()))
        .collect();
    assert_eq!(
        decoded,
        [
            (Some("FUNDS DEPOSIT"), Some(0.0), Some(5000.0)),
            (Some("FUNDS WITHDRAWAL"), Some(1500.5), Some(3499.5))
        ]
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

// ---- T2 --------------------------------------------------------------------------------------

// row: T2
#[tokio::test]
async fn t2_trade_history_from_the_fixture_with_numeric_charges() {
    let server = history_server(upstream_payload("get_trade_history.json")).await;
    let trades = client_for(&server)
        .statements()
        .trade_history(date(2024, 1, 1), date(2024, 1, 31), 0)
        .await
        .unwrap();
    assert_eq!(trades.len(), 1);
    let t = &trades[0];
    assert_eq!(t.order_id.as_ref().map(|o| o.as_ref()), Some("string"));
    assert_eq!(
        t.transaction_type,
        Some(Inbound::Known(TransactionType::Buy))
    );
    assert_eq!(
        t.exchange_segment,
        Some(Inbound::Known(ExchangeSegment::NseEq))
    );
    assert_eq!(t.product_type, Some(Inbound::Known(ProductType::Cnc)));
    assert_eq!(t.isin.as_ref().map(|i| i.as_ref()), Some("string"));
    assert_eq!(t.instrument.as_deref(), Some("string"));
    assert_eq!(
        [
            t.sebi_tax,
            t.stt,
            t.brokerage_charges,
            t.service_tax,
            t.exchange_transaction_charges,
            t.stamp_duty
        ],
        [Some(0.0); 6]
    );
    assert_eq!((t.traded_quantity, t.traded_price), (Some(0), Some(0.0)));
    assert_eq!(t.drv_option_type, Some(Inbound::Known(OptionType::Call)));
    // The fixture has no tradingSymbol (the guide has one).
    assert_eq!(t.trading_symbol, None);
    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].url.path(), "/v2/trades/2024-01-01/2024-01-31/0");
    assert!(received[0].url.query().is_none());
}

// row: T2
#[tokio::test]
async fn t2_trade_history_with_string_charges() {
    let reply = json!([{
        "orderId": "212212307731",
        "sebiTax": "0.0002",
        "stt": "1.25",
        "brokerageCharges": "20",
        "serviceTax": "3.6",
        "exchangeTransactionCharges": "0.0147",
        "stampDuty": "0.15",
        "drvExpiryDate": 1672338600
    }]);
    let server = history_server(serde_json::to_vec(&reply).unwrap()).await;
    let trades = client_for(&server)
        .statements()
        .trade_history(date(2024, 1, 1), date(2024, 1, 31), 0)
        .await
        .unwrap();
    let t = &trades[0];
    assert_eq!(
        [
            t.sebi_tax,
            t.stt,
            t.brokerage_charges,
            t.service_tax,
            t.exchange_transaction_charges,
            t.stamp_duty
        ],
        [0.0002, 1.25, 20.0, 3.6, 0.0147, 0.15].map(Some)
    );
    assert_eq!(
        t.drv_expiry_date.as_ref().map(|d| d.as_str()),
        Some("1672338600")
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

// ---- Validation: refused locally, nothing sent -----------------------------------------------

async fn any_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    server
}

fn reason(err: &dhani::Error) -> (&'static str, ValidationReason) {
    assert_eq!(err.kind(), ErrorKind::Validation);
    let v = err.validation().expect("a validation error");
    (v.field, v.reason.clone())
}

#[tokio::test]
async fn a_reversed_range_sends_nothing() {
    let server = any_server().await;
    let client: DhanClient = client_for(&server);
    let expected = (
        "from",
        ValidationReason::Inconsistent("must not be after to"),
    );
    let err = client
        .statements()
        .ledger(date(2024, 2, 1), date(2024, 1, 1))
        .await
        .unwrap_err();
    assert_eq!(reason(&err), expected);
    let err = client
        .statements()
        .trade_history(date(2024, 2, 1), date(2024, 1, 1), 0)
        .await
        .unwrap_err();
    assert_eq!(reason(&err), expected);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_date_beyond_year_9999_sends_nothing() {
    let server = any_server().await;
    let err = client_for(&server)
        .statements()
        .ledger(date(2024, 1, 1), date(10_000, 1, 1))
        .await
        .unwrap_err();
    assert_eq!(reason(&err), ("to", ValidationReason::OutOfRange));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_one_day_range_is_accepted() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/trades/2024-01-05/2024-01-05/3"))
        .and(expect_headers())
        .respond_with(json_reply(b"[]".to_vec()))
        .expect(1)
        .mount(&server)
        .await;
    let trades = client_for(&server)
        .statements()
        .trade_history(date(2024, 1, 5), date(2024, 1, 5), 3)
        .await
        .unwrap();
    assert!(trades.is_empty());
}
