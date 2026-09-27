//! Market quote contract tests (§9 rows Q1–Q3), typed and raw: method, exact path, credential
//! headers and the exact body with the injected dhanClientId; the OpenAPI-derived fixtures
//! decode to hand-written values, and the raw variants return the fixture unchanged. Invalid
//! requests send nothing.

mod support;

use dhani::error::ValidationReason;
use dhani::rest::QuoteRequest;
use dhani::types::{ExchangeSegment, RawJson, SecurityId};
use dhani::{DhanClient, ErrorKind};
use serde_json::json;
use support::fixtures::synth;
use support::mock::{CLIENT_ID, body_json_eq, client_for, expect_headers};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn id(s: &str) -> SecurityId {
    SecurityId::new(s).unwrap()
}

fn request() -> QuoteRequest {
    let mut req = QuoteRequest::new();
    req.add(ExchangeSegment::NseEq, id("11536"));
    req
}

/// A server answering `POST route` with `fixture`, matching the exact body.
async fn serve(route: &str, fixture: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(route))
        .and(expect_headers())
        .and(header("content-type", "application/json"))
        .and(body_json_eq(
            json!({"NSE_EQ": [11536], "dhanClientId": CLIENT_ID}),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_bytes(synth(fixture)),
        )
        .expect(1)
        .mount(&server)
        .await;
    server
}

fn fixture_json(fixture: &str) -> serde_json::Value {
    serde_json::from_slice(&synth(fixture)).unwrap()
}

async fn requests(server: &MockServer) -> usize {
    let received = server.received_requests().await.unwrap();
    assert!(received.iter().all(|r| r.url.query().is_none()));
    received.len()
}

// ---- Q1–Q3, typed ----------------------------------------------------------------------------

#[tokio::test]
async fn q1_ltp() {
    let server = serve("/v2/marketfeed/ltp", "quote_ltp.json").await;
    let data = client_for(&server)
        .market_quote()
        .ltp(&request())
        .await
        .unwrap();
    assert_eq!(data.status.as_deref(), Some("success"));
    let q = data.get(ExchangeSegment::NseEq, &id("11536")).unwrap();
    assert_eq!(q.last_price, Some(1.5));
    assert_eq!(requests(&server).await, 1);
}

#[tokio::test]
async fn q2_ohlc() {
    let server = serve("/v2/marketfeed/ohlc", "quote_ohlc.json").await;
    let data = client_for(&server)
        .market_quote()
        .ohlc(&request())
        .await
        .unwrap();
    let q = data.get(ExchangeSegment::NseEq, &id("11536")).unwrap();
    assert_eq!(q.last_price, Some(1.5));
    let ohlc = q.ohlc.as_ref().unwrap();
    assert_eq!(
        (ohlc.open, ohlc.high, ohlc.low, ohlc.close),
        (Some(1.5), Some(1.5), Some(1.5), Some(1.5))
    );
    assert_eq!(requests(&server).await, 1);
}

#[tokio::test]
async fn q3_quote_has_five_levels_each_side() {
    let server = serve("/v2/marketfeed/quote", "quote_full.json").await;
    let data = client_for(&server)
        .market_quote()
        .quote(&request())
        .await
        .unwrap();
    let q = data.get(ExchangeSegment::NseEq, &id("11536")).unwrap();
    assert_eq!(
        (q.average_price, q.last_price, q.net_change),
        (Some(1.5), Some(1.5), Some(1.5))
    );
    assert_eq!(
        (q.buy_quantity, q.sell_quantity, q.volume, q.oi),
        (Some(1), Some(1), Some(1), Some(1))
    );
    assert_eq!(
        q.last_trade_time.as_ref().map(|t| t.as_str()),
        Some("2024-09-11 14:39:29")
    );
    assert_eq!(
        (q.lower_circuit_limit, q.upper_circuit_limit),
        (Some(1.5), Some(1.5))
    );
    let depth = q.depth.as_ref().unwrap();
    assert_eq!((depth.buy.len(), depth.sell.len()), (5, 5));
    assert_eq!(
        (
            depth.buy[4].quantity,
            depth.buy[4].price,
            depth.buy[4].orders
        ),
        (Some(1), Some(1.5), Some(1))
    );
    assert_eq!(requests(&server).await, 1);
}

// ---- Q1–Q3, raw ------------------------------------------------------------------------------

#[tokio::test]
async fn the_raw_variants_return_the_body_unchanged() {
    for (route, fixture) in [
        ("/v2/marketfeed/ltp", "quote_ltp.json"),
        ("/v2/marketfeed/ohlc", "quote_ohlc.json"),
        ("/v2/marketfeed/quote", "quote_full.json"),
    ] {
        let server = serve(route, fixture).await;
        let quote = client_for(&server);
        let quote = quote.market_quote();
        let raw: RawJson = match fixture {
            "quote_ltp.json" => quote.ltp_raw(&request()).await,
            "quote_ohlc.json" => quote.ohlc_raw(&request()).await,
            _ => quote.quote_raw(&request()).await,
        }
        .unwrap();
        assert_eq!(raw.0, fixture_json(fixture), "{route}");
        assert_eq!(requests(&server).await, 1, "{route}");
    }
}

// ---- Validation: refused locally, nothing sent -----------------------------------------------

async fn refused(req: QuoteRequest) -> (&'static str, ValidationReason) {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let client: DhanClient = client_for(&server);
    let err = client.market_quote().ltp(&req).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(server.received_requests().await.unwrap().is_empty());
    let v = err.validation().expect("a validation error");
    (v.field, v.reason.clone())
}

#[tokio::test]
async fn a_thousand_and_one_instruments_send_nothing() {
    let mut req = QuoteRequest::new();
    for n in 1..=1001 {
        req.add(ExchangeSegment::NseFno, id(&n.to_string()));
    }
    assert_eq!(
        refused(req).await,
        ("instruments", ValidationReason::TooMany { max: 1000 })
    );
}

#[tokio::test]
async fn a_non_numeric_security_id_sends_nothing() {
    let mut req = QuoteRequest::new();
    req.add(ExchangeSegment::NseEq, id("TCS"));
    assert_eq!(
        refused(req).await,
        ("security_id", ValidationReason::InvalidCharacters)
    );
}

#[tokio::test]
async fn an_empty_request_sends_nothing() {
    assert_eq!(
        refused(QuoteRequest::new()).await,
        ("instruments", ValidationReason::Empty)
    );
}
