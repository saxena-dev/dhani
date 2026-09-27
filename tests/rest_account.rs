//! Account facade contract tests (§9 rows A6–A7): renew_token and profile send the access-token
//! and dhanClientId headers; the token flow from a credential-less client through
//! generate_access_token and with_credentials to renew_token shares one rate limiter.

mod support;

use dhani::credentials::{Pin, Totp};
use dhani::rest::RateLimiter;
use dhani::{ClientId, DhanClient, ErrorKind};
use serde_json::json;
use support::fixtures::synth;
use support::mock::{ACCESS_TOKEN, CLIENT_ID, client_for, expect_headers, urls_for};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ISSUED: &str = "eyJhbGciOiJIUzUxMiJ9.eyJzdWIiOiJTRU5USU5FTC1KV1QifQ.U0VOVElORUwtU0lHTkFUVVJF";

fn json_reply(body: Vec<u8>) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_bytes(body)
}

async fn serve(route: &str, fixture: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(route))
        .and(expect_headers())
        .and(header("dhanClientId", CLIENT_ID))
        .respond_with(json_reply(synth(fixture)))
        .expect(1)
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn a6_renew_token_sends_both_headers() {
    let (capture, _guard) = support::trace::install();
    let server = serve("/v2/RenewToken", "auth_issued_token.json").await;
    let token = client_for(&server).account().renew_token().await.unwrap();
    assert_eq!(token.access_token.expose_secret(), ISSUED);
    assert_eq!(token.given_power_of_attorney, Some(true));
    assert_eq!(
        token.expiry_time.as_ref().map(|t| t.as_str()),
        Some("2024-09-11 14:39:29")
    );
    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(
        received[0]
            .headers
            .get("access-token")
            .and_then(|v| v.to_str().ok()),
        Some(ACCESS_TOKEN)
    );
    assert!(received[0].body.is_empty());
    let issued = capture.events_named("auth.token.issued");
    assert_eq!(issued.len(), 1);
    assert_eq!(issued[0].field("method"), Some("renew"));
}

#[tokio::test]
async fn a7_profile_is_raw_json() {
    let server = serve("/v2/profile", "profile.json").await;
    let profile = client_for(&server).account().profile().await.unwrap();
    assert_eq!(profile.0.0, json!({}));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn renew_without_credentials_is_a_config_error_and_sends_nothing() {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let anonymous = DhanClient::builder()
        .urls(urls_for(&server))
        .rate_limiter(RateLimiter::disabled())
        .build()
        .unwrap();
    let err = anonymous.account().renew_token().await.unwrap_err();
    assert_eq!((err.kind(), err.attempts()), (ErrorKind::Config, 0));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_generated_token_rotates_into_the_client_and_renews() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/app/generateAccessToken"))
        .respond_with(json_reply(
            serde_json::to_vec(&json!({"dhanClientId": CLIENT_ID, "accessToken": ACCESS_TOKEN}))
                .unwrap(),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/RenewToken"))
        .and(expect_headers())
        .and(header("dhanClientId", CLIENT_ID))
        .respond_with(json_reply(synth("auth_issued_token.json")))
        .expect(1)
        .mount(&server)
        .await;

    // Build without credentials, generate a token, then rotate it in.
    let anonymous = DhanClient::builder()
        .urls(urls_for(&server))
        .rate_limiter(RateLimiter::disabled())
        .build()
        .unwrap();
    let token = anonymous
        .auth()
        .generate_access_token(
            &ClientId::new(CLIENT_ID).unwrap(),
            &Pin::new("482913").unwrap(),
            &Totp::new("735162").unwrap(),
        )
        .await
        .unwrap();
    let rotated = anonymous.with_credentials(token.credentials());
    assert!(rotated.rate_limiter().ptr_eq(anonymous.rate_limiter()));
    let renewed = rotated.account().renew_token().await.unwrap();
    assert_eq!(renewed.access_token.expose_secret(), ISSUED);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}
