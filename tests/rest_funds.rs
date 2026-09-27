//! Funds facade contract tests (§9 rows M1–M3): method, exact path, credential headers and
//! hand-written body; fixtures decoded to hand-written values, including the Swagger sentinels.
//! The multi-margin response decodes from both the documented camelCase floats and the OpenAPI
//! snake_case strings. Invalid requests send nothing.

mod support;

use dhani::error::ValidationReason;
use dhani::rest::{MarginRequest, MultiMarginRequest};
use dhani::types::{ExchangeSegment, ProductType, SecurityId, TransactionType};
use dhani::{DhanClient, ErrorKind};
use serde_json::json;
use support::fixtures::{synth, upstream_payload};
use support::mock::{CLIENT_ID, body_json_eq, client_for, expect_headers};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MIN: f64 = -3.402823669209385e+38;

fn json_reply(body: Vec<u8>) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_bytes(body)
}

/// A server with one mock: `verb path`, the credential headers, `body` when given.
async fn serve(
    verb: &str,
    at: &str,
    body: Option<serde_json::Value>,
    reply: ResponseTemplate,
) -> MockServer {
    let server = MockServer::start().await;
    let mock = Mock::given(method(verb))
        .and(path(at))
        .and(expect_headers());
    let mock = match body {
        Some(expected) => mock
            .and(header("content-type", "application/json"))
            .and(body_json_eq(expected)),
        None => mock,
    };
    mock.respond_with(reply).expect(1).mount(&server).await;
    server
}

async fn requests(server: &MockServer) -> usize {
    let received = server.received_requests().await.unwrap();
    assert!(received.iter().all(|r| r.url.query().is_none()));
    received.len()
}

fn leg(security: &str, side: TransactionType) -> MarginRequest {
    MarginRequest::new(
        ExchangeSegment::NseFno,
        SecurityId::new(security).unwrap(),
        side,
        75,
        ProductType::Margin,
        101.5,
    )
}

fn leg_body(security: &str, side: &str) -> serde_json::Value {
    json!({
        "exchangeSegment": "NSE_FNO",
        "transactionType": side,
        "quantity": 75,
        "productType": "MARGIN",
        "securityId": security,
        "price": 101.5
    })
}

fn multi() -> MultiMarginRequest {
    MultiMarginRequest::new([
        leg("52175", TransactionType::Sell),
        leg("52176", TransactionType::Buy),
    ])
    .with_include_position(true)
}

fn multi_body() -> serde_json::Value {
    json!({
        "dhanClientId": CLIENT_ID,
        "includePosition": true,
        "includeOrder": false,
        "scripList": [leg_body("52175", "SELL"), leg_body("52176", "BUY")]
    })
}

// ---- M1–M3 -----------------------------------------------------------------------------------

#[tokio::test]
async fn m1_limits_keep_the_misspelled_wire_names() {
    let server = serve(
        "GET",
        "/v2/fundlimit",
        None,
        json_reply(upstream_payload("get_fund_limits.json")),
    )
    .await;
    let limits = client_for(&server).funds().limits().await.unwrap();
    assert_eq!(limits.available_balance, Some(MIN));
    assert_eq!(limits.receivable_amount, Some(MIN));
    assert_eq!(limits.sod_limit, Some(MIN));
    assert_eq!(limits.collateral_amount, Some(MIN));
    assert_eq!(limits.utilized_amount, Some(MIN));
    assert_eq!(limits.blocked_payout_amount, Some(MIN));
    assert_eq!(limits.withdrawable_balance, Some(MIN));
    assert!(limits.dhan_client_id.is_some());
    assert_eq!(requests(&server).await, 1);
}

#[tokio::test]
async fn m2_margin() {
    let mut body = leg_body("52175", "SELL");
    body["dhanClientId"] = json!(CLIENT_ID);
    body["triggerPrice"] = json!(100.0);
    let server = serve(
        "POST",
        "/v2/margincalculator",
        Some(body),
        json_reply(upstream_payload("margin_calculator.json")),
    )
    .await;
    let req = leg("52175", TransactionType::Sell).with_trigger_price(100.0);
    let m = client_for(&server).funds().margin(&req).await.unwrap();
    assert_eq!(
        [
            m.total_margin,
            m.span_margin,
            m.exposure_margin,
            m.available_balance,
            m.variable_margin,
            m.insufficient_balance,
            m.brokerage
        ],
        [Some(MIN); 7]
    );
    assert_eq!(m.leverage.as_deref(), Some("string"));
    assert_eq!(requests(&server).await, 1);
}

#[tokio::test]
async fn m3_margin_multi_from_the_documented_camel_case() {
    let server = serve(
        "POST",
        "/v2/margincalculator/multi",
        Some(multi_body()),
        json_reply(synth("multi_margin.json")),
    )
    .await;
    let m = client_for(&server)
        .funds()
        .margin_multi(&multi())
        .await
        .unwrap();
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
        [Some(1.5); 7]
    );
    assert_eq!(m.hedge_benefit, None);
    assert_eq!(
        m.client_id.as_ref().map(|c| c.expose_secret()),
        Some("string")
    );
    assert_eq!(requests(&server).await, 1);
}

#[tokio::test]
async fn m3_margin_multi_from_the_openapi_snake_case_strings() {
    let reply = json!({
        "total_margin": "2150.25",
        "span_margin": "1500.5",
        "exposure_margin": "649.75",
        "equity_margin": "0",
        "fo_margin": "2150.25",
        "commodity_margin": "0",
        "currency": "0",
        "hedge_benefit": "310.5"
    });
    let server = serve(
        "POST",
        "/v2/margincalculator/multi",
        Some(multi_body()),
        json_reply(serde_json::to_vec(&reply).unwrap()),
    )
    .await;
    let m = client_for(&server)
        .funds()
        .margin_multi(&multi())
        .await
        .unwrap();
    assert_eq!(
        (m.total_margin, m.span_margin, m.exposure_margin),
        (Some(2150.25), Some(1500.5), Some(649.75))
    );
    assert_eq!(
        (m.equity_margin, m.fo_margin, m.commodity_margin, m.currency),
        (Some(0.0), Some(2150.25), Some(0.0), Some(0.0))
    );
    assert_eq!(m.hedge_benefit, Some(310.5));
    assert!(m.client_id.is_none());
    assert_eq!(requests(&server).await, 1);
}

// ---- Validation: refused locally, nothing sent -----------------------------------------------

async fn refused(req: MultiMarginRequest) -> (&'static str, ValidationReason) {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let client: DhanClient = client_for(&server);
    let err = client.funds().margin_multi(&req).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(server.received_requests().await.unwrap().is_empty());
    let v = err.validation().expect("a validation error");
    (v.field, v.reason.clone())
}

#[tokio::test]
async fn a_51_leg_request_sends_nothing() {
    let legs = vec![leg("52175", TransactionType::Buy); 51];
    assert_eq!(
        refused(MultiMarginRequest::new(legs)).await,
        ("scrip_list", ValidationReason::TooMany { max: 50 })
    );
}

#[tokio::test]
async fn a_request_without_legs_sends_nothing() {
    assert_eq!(
        refused(MultiMarginRequest::new([])).await,
        ("scrip_list", ValidationReason::Empty)
    );
}

#[tokio::test]
async fn an_invalid_single_margin_request_sends_nothing() {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let mut req = leg("52175", TransactionType::Buy);
    req.quantity = 0;
    let err = client_for(&server).funds().margin(&req).await.unwrap_err();
    let v = err.validation().expect("a validation error");
    assert_eq!(
        (v.field, v.reason.clone()),
        ("quantity", ValidationReason::NotPositive)
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}
