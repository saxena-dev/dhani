//! Historical candles contract tests (§9 rows H1–H2): method, exact path, credential headers
//! and the exact body with the injected dhanClientId; the empty upstream fixtures and the
//! synthesised candles decode to hand-written values. Invalid requests send nothing.

mod support;

use chrono::NaiveDate;
use dhani::error::ValidationReason;
use dhani::rest::{Candle, DailyRequest, IntradayInterval, IntradayRequest};
use dhani::types::{ExchangeSegment, ExpiryCode, InstrumentKind, SecurityId};
use dhani::{DhanClient, ErrorKind};
use serde_json::json;
use support::fixtures::{synth, upstream_payload};
use support::mock::{CLIENT_ID, body_json_eq, client_for, expect_headers};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

async fn serve(route: &str, body: serde_json::Value, reply: Vec<u8>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(route))
        .and(expect_headers())
        .and(header("content-type", "application/json"))
        .and(body_json_eq(body))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_bytes(reply),
        )
        .expect(1)
        .mount(&server)
        .await;
    server
}

fn daily() -> DailyRequest {
    DailyRequest::new(
        ExchangeSegment::NseFno,
        SecurityId::new("52175").unwrap(),
        InstrumentKind::Futidx,
        date(2025, 9, 1),
        date(2025, 9, 2),
    )
    .with_expiry_code(ExpiryCode::Near)
    .with_oi(true)
}

fn daily_body() -> serde_json::Value {
    json!({
        "dhanClientId": CLIENT_ID,
        "securityId": "52175",
        "exchangeSegment": "NSE_FNO",
        "instrument": "FUTIDX",
        "expiryCode": 1,
        "oi": true,
        "fromDate": "2025-09-01",
        "toDate": "2025-09-02"
    })
}

fn intraday(interval: IntradayInterval) -> IntradayRequest {
    IntradayRequest::new(
        ExchangeSegment::NseEq,
        SecurityId::new("1333").unwrap(),
        InstrumentKind::Equity,
        interval,
        date(2025, 9, 1),
        date(2025, 9, 1),
    )
}

fn intraday_body(interval: u8) -> serde_json::Value {
    json!({
        "dhanClientId": CLIENT_ID,
        "securityId": "1333",
        "exchangeSegment": "NSE_EQ",
        "instrument": "EQUITY",
        "interval": interval,
        "oi": false,
        "fromDate": "2025-09-01",
        "toDate": "2025-09-01"
    })
}

fn assert_synth_candles(candles: &dhani::rest::Candles) {
    let all: Vec<Candle> = candles.iter().collect();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].ts, 1756698300);
    assert_eq!(
        (all[0].open, all[0].high, all[0].low, all[0].close),
        (1.5, 2.5, 1.0, 2.0)
    );
    assert_eq!((all[0].volume, all[0].open_interest), (100, Some(10)));
    assert_eq!(
        (all[1].ts, all[1].close, all[1].volume),
        (1756699200, 3.0, 200)
    );
    assert_eq!(all[1].open_interest, Some(20));
}

// ---- H1–H2 -----------------------------------------------------------------------------------

#[tokio::test]
async fn h1_daily_from_the_empty_upstream_fixture() {
    let server = serve(
        "/v2/charts/historical",
        daily_body(),
        upstream_payload("historical_daily_data.json"),
    )
    .await;
    let candles = client_for(&server)
        .historical()
        .daily(&daily())
        .await
        .unwrap();
    assert!(candles.is_empty());
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn h1_daily_candles() {
    let server = serve("/v2/charts/historical", daily_body(), synth("candles.json")).await;
    let candles = client_for(&server)
        .historical()
        .daily(&daily())
        .await
        .unwrap();
    assert_synth_candles(&candles);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn h2_intraday_from_the_empty_upstream_fixture() {
    let server = serve(
        "/v2/charts/intraday",
        intraday_body(5),
        upstream_payload("intraday_minute_data.json"),
    )
    .await;
    let candles = client_for(&server)
        .historical()
        .intraday(&intraday(IntradayInterval::Min5))
        .await
        .unwrap();
    assert!(candles.is_empty());
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn h2_intraday_sends_25_as_an_integer() {
    let server = serve(
        "/v2/charts/intraday",
        intraday_body(25),
        synth("candles.json"),
    )
    .await;
    let candles = client_for(&server)
        .historical()
        .intraday(&intraday(IntradayInterval::Min25))
        .await
        .unwrap();
    assert_synth_candles(&candles);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn mismatched_columns_are_a_decode_error() {
    let reply = json!({
        "open": [1.0, 2.0], "high": [1.0], "low": [1.0], "close": [1.0],
        "volume": [1], "timestamp": [1]
    });
    let server = serve(
        "/v2/charts/historical",
        daily_body(),
        serde_json::to_vec(&reply).unwrap(),
    )
    .await;
    let err = client_for(&server)
        .historical()
        .daily(&daily())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Decode);
    // The detail never quotes the body.
    assert!(!err.detail().unwrap_or_default().contains("2.0"));
}

// ---- Validation: refused locally, nothing sent -----------------------------------------------

#[tokio::test]
async fn a_daily_range_without_a_full_day_sends_nothing() {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let client: DhanClient = client_for(&server);
    let mut req = daily();
    req.to_date = req.from_date;
    let err = client.historical().daily(&req).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    let v = err.validation().unwrap();
    assert_eq!(
        (v.field, v.reason.clone()),
        (
            "from_date",
            ValidationReason::Inconsistent("must be before to_date (to_date is not inclusive)")
        )
    );
    let mut currency = intraday(IntradayInterval::Min1);
    currency.exchange_segment = ExchangeSegment::NseCurrency;
    let err = client.historical().intraday(&currency).await.unwrap_err();
    let v = err.validation().unwrap();
    assert_eq!(
        (v.field, v.reason.clone()),
        ("exchange_segment", ValidationReason::UnknownEnumValue)
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}
