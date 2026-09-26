//! Transport pipeline tests against a wiremock server: credential headers by auth mode, the
//! client-id body injection, and nothing sent without credentials. Requests are issued through
//! the client's test hook until each endpoint's facade exists.

mod support;

use dhani::error::Stage;
use dhani::labels::EndpointId;
use dhani::{DhanClient, ErrorKind};
use support::mock::{ACCESS_TOKEN, CLIENT_ID, body_json_eq, client_for, expect_headers, urls_for};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn received(server: &MockServer) -> Vec<wiremock::Request> {
    server
        .received_requests()
        .await
        .expect("request recording is on")
}

#[tokio::test]
async fn get_requests_carry_the_credential_headers_and_no_content_type() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/orders"))
        .and(expect_headers())
        .respond_with(ResponseTemplate::new(200).set_body_bytes(
            support::fixtures::upstream_payload("get-current-orders-list.json"),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let body = client_for(&server)
        .__execute_for_tests(EndpointId::OrdersList, &[], &[], None, None)
        .await
        .unwrap();
    assert!(body.is_some());
    let requests = received(&server).await;
    assert_eq!(requests.len(), 1);
    let h = &requests[0].headers;
    assert_eq!(h.get("access-token").unwrap(), ACCESS_TOKEN);
    assert_eq!(h.get("client-id").unwrap(), CLIENT_ID);
    assert_eq!(h.get("accept").unwrap(), "application/json");
    assert!(h.get("content-type").is_none());
    assert!(h.get("dhanclientid").is_none());
    let agent = h.get("user-agent").unwrap().to_str().unwrap();
    assert!(agent.starts_with("dhani/"), "{agent}");
    // Sensitivity of the credential header values is asserted on the built request in the
    // transport's unit tests (headers_follow_the_auth_mode_and_are_sensitive).
}

#[tokio::test]
async fn json_bodies_get_the_client_id_at_the_top_level() {
    let server = MockServer::start().await;
    let expected = serde_json::json!({
        "transactionType": "BUY",
        "quantity": 5,
        "dhanClientId": CLIENT_ID,
    });
    Mock::given(method("POST"))
        .and(path("/v2/orders"))
        .and(expect_headers())
        .and(body_json_eq(expected))
        .respond_with(
            ResponseTemplate::new(200).set_body_bytes(support::fixtures::synth("order_ack.json")),
        )
        .expect(1)
        .mount(&server)
        .await;
    // An existing dhanClientId in the input is overwritten.
    let input =
        serde_json::json!({"transactionType": "BUY", "quantity": 5, "dhanClientId": "0000000000"});
    let ack = client_for(&server)
        .__execute_for_tests(EndpointId::OrdersPlace, &[], &[], Some(input), None)
        .await
        .unwrap();
    assert_eq!(ack.unwrap().0["orderId"], "string");
    let requests = received(&server).await;
    assert_eq!(
        requests[0].headers.get("content-type").unwrap(),
        "application/json"
    );
}

#[tokio::test]
async fn missing_credentials_send_nothing() {
    let server = MockServer::start().await;
    let client = DhanClient::builder()
        .urls(urls_for(&server))
        .build()
        .unwrap();
    let err = client
        .__execute_for_tests(EndpointId::OrdersList, &[], &[], None, None)
        .await
        .unwrap_err();
    assert_eq!(
        (err.kind(), err.stage()),
        (ErrorKind::Config, Stage::NotSent)
    );
    assert!(!err.may_have_reached_server());
    assert!(received(&server).await.is_empty());
}

#[tokio::test]
async fn renew_token_adds_the_dhan_client_id_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/RenewToken"))
        .and(expect_headers())
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(support::fixtures::synth("auth_issued_token.json")),
        )
        .expect(1)
        .mount(&server)
        .await;
    client_for(&server)
        .__execute_for_tests(EndpointId::AccountRenewToken, &[], &[], None, None)
        .await
        .unwrap();
    let requests = received(&server).await;
    assert_eq!(requests[0].headers.get("dhanclientid").unwrap(), CLIENT_ID);
}

#[tokio::test]
async fn token_generation_sends_no_auth_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/app/generateAccessToken"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(support::fixtures::synth("auth_issued_token.json")),
        )
        .expect(1)
        .mount(&server)
        .await;
    let query = [
        ("dhanClientId", CLIENT_ID.to_owned()),
        ("pin", "1234".to_owned()),
        ("totp", "123456".to_owned()),
    ];
    client_for(&server)
        .__execute_for_tests(EndpointId::AuthGenerateAccessToken, &[], &query, None, None)
        .await
        .unwrap();
    let requests = received(&server).await;
    let h = &requests[0].headers;
    assert!(
        h.get("access-token").is_none()
            && h.get("client-id").is_none()
            && h.get("dhanclientid").is_none()
    );
    assert!(h.get("content-type").is_none());
    assert_eq!(
        requests[0].url.query(),
        Some("dhanClientId=9999888877&pin=1234&totp=123456")
    );
}

#[test]
fn fixture_loader_unwraps_envelopes_and_keeps_raw_bytes() {
    let payload: serde_json::Value =
        serde_json::from_slice(&support::fixtures::upstream_payload("get-order-by-id.json"))
            .unwrap();
    assert!(
        payload
            .as_object()
            .is_some_and(|o| o.contains_key("orderId")),
        "{payload}"
    );
    let raw = support::fixtures::raw_bytes("tests/fixtures/synth/order_update.json");
    let text = String::from_utf8(raw).unwrap();
    assert_eq!(text.matches("\"Remarks\"").count(), 2);
    let entry = support::fixtures::manifest_entry("tests/fixtures/synth/order_update.json");
    assert_eq!(
        (entry.class.as_str(), entry.endpoint.as_str()),
        ("synthesized", "W2")
    );
}
