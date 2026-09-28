//! Portfolio facade contract tests (§9 rows P1–P4): method, exact path, credential headers and
//! hand-written body; fixtures decoded to hand-written values. Convert and exit-all accept both
//! a `202` with no body and a `200 {}`. An invalid conversion sends nothing.

mod support;

use dhani::error::ValidationReason;
use dhani::rest::ConvertPositionRequest;
use dhani::types::{
    ExchangeSegment, Inbound, Isin, OptionType, PositionType, ProductType, SecurityId,
};
use dhani::{DhanClient, ErrorKind};
use serde_json::json;
use support::fixtures::{synth, upstream_payload};
use support::mock::{CLIENT_ID, body_json_eq, client_for, expect_headers};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn json_reply(status: u16, body: Vec<u8>) -> ResponseTemplate {
    ResponseTemplate::new(status)
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

/// The two success replies documented for convert and exit-all (OQ-18).
fn empty_replies() -> Vec<(&'static str, ResponseTemplate)> {
    vec![
        ("202, no body", ResponseTemplate::new(202)),
        ("200 {}", json_reply(200, b"{}".to_vec())),
    ]
}

// ---- P1–P4 -----------------------------------------------------------------------------------

#[tokio::test]
async fn p1_holdings_from_the_upstream_fixture() {
    let server = serve(
        "GET",
        "/v2/holdings",
        None,
        json_reply(200, upstream_payload("get-current-holdings.json")),
    )
    .await;
    let holdings = client_for(&server).portfolio().holdings().await.unwrap();
    assert_eq!(holdings.len(), 1);
    let h = &holdings[0];
    assert_eq!(h.exchange.as_deref(), Some("NSE"));
    assert_eq!(h.trading_symbol.as_deref(), Some("string"));
    assert_eq!(h.security_id.as_ref().map(|s| s.as_ref()), Some("string"));
    // The placeholder is not a valid ISIN, but decodes and fails only on reuse.
    let isin: &Isin = h.isin.as_ref().unwrap();
    assert_eq!(isin.as_ref(), "string");
    assert!(isin.validate().is_err());
    assert_eq!(
        (h.total_qty, h.dp_qty, h.t1_qty),
        (Some(0), Some(0), Some(0))
    );
    assert_eq!((h.available_qty, h.collateral_qty), (Some(0), Some(0)));
    assert_eq!(
        (h.avg_cost_price, h.last_traded_price),
        (Some(0.0), Some(0.0))
    );
    // The upstream fixture has no MTF keys.
    assert_eq!((h.mtf_t1_qty, h.mtf_qty), (None, None));
    assert_eq!(requests(&server).await, 1);
}

#[tokio::test]
async fn p1_holdings_with_the_mtf_keys() {
    let server = serve(
        "GET",
        "/v2/holdings",
        None,
        json_reply(200, synth("holding_mtf.json")),
    )
    .await;
    let holdings = client_for(&server).portfolio().holdings().await.unwrap();
    let h = &holdings[0];
    assert_eq!((h.mtf_t1_qty, h.mtf_qty), (Some(1), Some(1)));
    assert_eq!((h.total_qty, h.available_qty), (Some(1), Some(1)));
    assert_eq!(
        (h.avg_cost_price, h.last_traded_price),
        (Some(1.5), Some(1.5))
    );
    assert_eq!(requests(&server).await, 1);
}

#[tokio::test]
async fn p2_positions() {
    let server = serve(
        "GET",
        "/v2/positions",
        None,
        json_reply(200, upstream_payload("get_positions.json")),
    )
    .await;
    let positions = client_for(&server).portfolio().positions().await.unwrap();
    assert_eq!(positions.len(), 1);
    let p = &positions[0];
    assert_eq!(p.trading_symbol.as_deref(), Some("string"));
    assert_eq!(p.position_type, Some(Inbound::Known(PositionType::Long)));
    assert_eq!(
        p.exchange_segment,
        Some(Inbound::Known(ExchangeSegment::NseEq))
    );
    assert_eq!(p.product_type, Some(Inbound::Known(ProductType::Cnc)));
    assert_eq!(
        (p.buy_avg, p.cost_price, p.realized_profit),
        (Some(0.0), Some(0.0), Some(0.0))
    );
    assert_eq!(
        (p.buy_qty, p.net_qty, p.multiplier),
        (Some(0), Some(0), Some(0))
    );
    assert_eq!(
        p.drv_expiry_date.as_ref().map(|t| t.as_str()),
        Some("string")
    );
    assert_eq!(p.drv_option_type, Some(Inbound::Known(OptionType::Call)));
    assert_eq!(p.cross_currency, Some(true));
    assert!(p.dhan_client_id.is_some());
    assert_eq!(requests(&server).await, 1);
}

#[tokio::test]
async fn p3_convert_position_accepts_202_empty_and_200_empty_object() {
    // The fixture body for the 200 case is the upstream `{}`.
    assert_eq!(upstream_payload("convert_position.json"), b"{}");
    let body = json!({
        "dhanClientId": CLIENT_ID,
        "fromProductType": "INTRADAY",
        "exchangeSegment": "NSE_EQ",
        "positionType": "LONG",
        "securityId": "11536",
        "convertQty": 40,
        "toProductType": "CNC"
    });
    for (label, reply) in empty_replies() {
        let server = serve("POST", "/v2/positions/convert", Some(body.clone()), reply).await;
        let result = client_for(&server)
            .portfolio()
            .convert_position(&convert())
            .await;
        assert_eq!(result.map_err(|e| e.kind()), Ok(()), "{label}");
        assert_eq!(requests(&server).await, 1, "{label}");
    }
}

#[tokio::test]
async fn p3_convert_position_sends_a_trading_symbol_when_set() {
    let body = json!({
        "dhanClientId": CLIENT_ID,
        "fromProductType": "INTRADAY",
        "exchangeSegment": "NSE_EQ",
        "positionType": "LONG",
        "securityId": "11536",
        "tradingSymbol": "TCS",
        "convertQty": 40,
        "toProductType": "CNC"
    });
    let server = serve(
        "POST",
        "/v2/positions/convert",
        Some(body),
        ResponseTemplate::new(202),
    )
    .await;
    let req = convert().with_trading_symbol("TCS");
    client_for(&server)
        .portfolio()
        .convert_position(&req)
        .await
        .unwrap();
    assert_eq!(requests(&server).await, 1);
}

#[tokio::test]
async fn p4_exit_all_accepts_202_empty_and_200_empty_object() {
    // The OpenAPI spec shows exit-all answering with a status object (OQ-18).
    let status = br#"{"status": "SUCCESS", "message": "All positions exited"}"#.to_vec();
    let mut replies = empty_replies();
    replies.push(("200 status object", json_reply(200, status)));
    for (label, reply) in replies {
        let server = serve("DELETE", "/v2/positions", None, reply).await;
        let result = client_for(&server).portfolio().exit_all().await;
        assert_eq!(result.map_err(|e| e.kind()), Ok(()), "{label}");
        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1, "{label}");
        assert!(received[0].body.is_empty(), "{label}");
    }
}

// ---- Validation: refused locally, nothing sent -----------------------------------------------

async fn refused(req: ConvertPositionRequest) -> (&'static str, ValidationReason) {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;
    let client: DhanClient = client_for(&server);
    let err = client.portfolio().convert_position(&req).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(server.received_requests().await.unwrap().is_empty());
    let v = err.validation().expect("a validation error");
    (v.field, v.reason.clone())
}

#[tokio::test]
async fn a_zero_quantity_conversion_sends_nothing() {
    let mut req = convert();
    req.convert_qty = 0;
    assert_eq!(
        refused(req).await,
        ("convert_qty", ValidationReason::NotPositive)
    );
}

#[tokio::test]
async fn a_conversion_to_cover_order_sends_nothing() {
    let mut req = convert();
    req.to_product_type = ProductType::Co;
    assert_eq!(
        refused(req).await,
        ("to_product_type", ValidationReason::UnknownEnumValue)
    );
}

// ---- A 2xx body that reports a failure (DHQ-ebr) ---------------------------------------------

#[tokio::test]
async fn a_200_exit_all_reporting_failure_is_an_api_error() {
    let body = br#"{"status":"failure","message":"no open positions to exit"}"#.to_vec();
    let server = serve("DELETE", "/v2/positions", None, json_reply(200, body)).await;
    let err = client_for(&server)
        .portfolio()
        .exit_all()
        .await
        .unwrap_err();
    assert_eq!(
        (err.kind(), err.http_status(), err.attempts()),
        (ErrorKind::Api, Some(200), 1)
    );
    assert_eq!(
        err.detail(),
        Some(r#"the response reported status "failure""#)
    );
    assert_eq!(requests(&server).await, 1);
}

#[tokio::test]
async fn a_200_convert_reporting_failure_keeps_the_broker_error() {
    let body = br#"{"status":"failure","errorType":"Input_Exception","errorCode":"DH-905","errorMessage":"Invalid quantity"}"#.to_vec();
    let server = serve("POST", "/v2/positions/convert", None, json_reply(200, body)).await;
    let err = client_for(&server)
        .portfolio()
        .convert_position(&convert())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Api);
    let api = err.api().expect("the broker error is parsed");
    assert_eq!(api.error_code, Some(dhani::error::ApiErrorCode::Dh905));
    assert_eq!(
        api.error_message.as_ref().map(|m| m.as_str()),
        Some("Invalid quantity")
    );
}

#[tokio::test]
async fn success_statuses_in_any_case_and_bodies_without_status_still_succeed() {
    for body in [
        r#"{"status":"SUCCESS","message":"All positions exited"}"#,
        r#"{"status":"success"}"#,
        r#"{"message":"done"}"#,
        "[]",
    ] {
        let server = serve(
            "DELETE",
            "/v2/positions",
            None,
            json_reply(200, body.as_bytes().to_vec()),
        )
        .await;
        let result = client_for(&server).portfolio().exit_all().await;
        assert_eq!(result.map_err(|e| e.kind()), Ok(()), "{body}");
    }
    let server = serve(
        "DELETE",
        "/v2/positions",
        None,
        json_reply(200, b"not json".to_vec()),
    )
    .await;
    let err = client_for(&server)
        .portfolio()
        .exit_all()
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Decode);
}
