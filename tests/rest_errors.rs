//! Negative tests through the client: each documented broker error shape, served by a wiremock
//! server on a read endpoint, maps to the expected error kind, parsed broker error and status.
//! Retries are off so that each test classifies exactly one response.

mod support;

use dhani::error::{ApiErrorCode, DataErrorCode, RateLimitSource, Stage};
use dhani::labels::EndpointId;
use dhani::rest::{RateLimiter, RetryPolicy};
use dhani::{DhanClient, Error, ErrorKind};
use support::mock::{credentials, urls_for};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Serves `status` with `body` on `GET /v2/orders`, calls it once and returns the error.
async fn fail_with(status: u16, content_type: &str, body: &str) -> Error {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/orders"))
        .respond_with(
            ResponseTemplate::new(status)
                .insert_header("content-type", content_type)
                .set_body_string(body),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = DhanClient::builder()
        .urls(urls_for(&server))
        .credentials(credentials())
        .rate_limiter(RateLimiter::disabled())
        .retry(RetryPolicy::none())
        .build()
        .unwrap();
    let err = client
        .__execute_for_tests(EndpointId::OrdersList, &[], &[], None, None)
        .await
        .unwrap_err();
    assert_eq!(err.attempts(), 1);
    assert_eq!(err.endpoint(), Some(EndpointId::OrdersList));
    err
}

fn code(err: &Error) -> Option<&ApiErrorCode> {
    err.api().and_then(|a| a.error_code.as_ref())
}

fn message(err: &Error) -> Option<&str> {
    err.api()
        .and_then(|a| a.error_message.as_ref())
        .map(|m| m.as_str())
}

#[tokio::test]
async fn dh_905_without_a_message_is_an_api_error() {
    // DOC:8872
    let err = fail_with(
        400,
        "application/json",
        r#"{"errorType": "Input_Exception", "errorCode": "DH-905"}"#,
    )
    .await;
    assert_eq!(
        (err.kind(), err.stage(), err.http_status()),
        (ErrorKind::Api, Stage::ResponseReceived, Some(400))
    );
    assert_eq!(code(&err), Some(&ApiErrorCode::Dh905));
    assert_eq!(message(&err), None);
    assert_eq!(
        err.api().unwrap().error_type.as_deref(),
        Some("Input_Exception")
    );
    assert!(err.rate_limit().is_none());
}

#[tokio::test]
async fn dh_901_is_an_auth_error() {
    // DOC:4227
    let err = fail_with(
        401,
        "application/json",
        r#"{"errorType":"Invalid_Authentication","errorCode":"DH-901","errorMessage":"Client ID or user generated access token is invalid or expired"}"#,
    )
    .await;
    assert_eq!(
        (err.kind(), err.stage(), err.http_status()),
        (ErrorKind::Auth, Stage::ResponseReceived, Some(401))
    );
    assert_eq!(code(&err), Some(&ApiErrorCode::Dh901));
    assert_eq!(
        message(&err),
        Some("Client ID or user generated access token is invalid or expired")
    );
}

#[tokio::test]
async fn data_code_805_is_a_remote_rate_limit() {
    // DOC:8895-8898
    let err = fail_with(
        429,
        "application/json",
        r#"{"data": {"805": "Too many requests. Further requests may result in the user being blocked."}}"#,
    )
    .await;
    assert_eq!(
        (err.kind(), err.stage(), err.http_status()),
        (ErrorKind::RateLimited, Stage::ResponseReceived, Some(429))
    );
    assert_eq!(
        err.rate_limit().map(|r| r.source),
        Some(RateLimitSource::Remote)
    );
    assert_eq!(
        code(&err),
        Some(&ApiErrorCode::Data(DataErrorCode::TooManyRequests))
    );
    assert_eq!(
        message(&err),
        Some("Too many requests. Further requests may result in the user being blocked.")
    );
}

#[tokio::test]
async fn the_rl001_shape_is_a_remote_rate_limit() {
    // DOC:5321-5324 (body), DOC:5270 (status 429)
    let err = fail_with(
        429,
        "application/json",
        r#"{"status": "failure", "errorType": "RATE_LIMIT_ERROR", "errorCode": "RL001", "errorMessage": "Too many requests. Please retry after 1 second."}"#,
    )
    .await;
    assert_eq!(
        (err.kind(), err.stage(), err.http_status()),
        (ErrorKind::RateLimited, Stage::ResponseReceived, Some(429))
    );
    assert_eq!(
        err.rate_limit().map(|r| r.source),
        Some(RateLimitSource::Remote)
    );
    assert_eq!(code(&err), Some(&ApiErrorCode::Other("RL001".to_owned())));
}

#[tokio::test]
async fn an_html_500_is_an_http_status_error() {
    let html = "<html><body><h1>500 Internal Server Error</h1></body></html>";
    let err = fail_with(500, "text/html", html).await;
    assert_eq!(
        (err.kind(), err.stage(), err.http_status()),
        (ErrorKind::HttpStatus, Stage::ResponseReceived, Some(500))
    );
    assert!(err.api().is_none());
    assert_eq!(err.detail(), Some("non-JSON error body of 60 bytes"));
}

#[tokio::test]
async fn a_malformed_success_body_is_a_decode_error_without_body_text() {
    let err = fail_with(200, "application/json", r#"{"orderId": 1"#).await;
    assert_eq!(
        (err.kind(), err.stage(), err.http_status()),
        (ErrorKind::Decode, Stage::ResponseReceived, Some(200))
    );
    let detail = err.detail().expect("a decode error has a detail");
    assert!(!detail.contains("orderId"), "{detail}");
    assert_eq!(
        detail,
        "response body does not match the expected shape (eof error at line 1 column 13)"
    );
    let rendered = format!("{err} {err:?}");
    assert!(!rendered.contains("orderId"), "{rendered}");
}

#[tokio::test]
async fn the_validation_error_shape_keeps_its_code() {
    // DOC:5252-5260
    let err = fail_with(
        400,
        "application/json",
        r#"{"status": "failure", "errorType": "VALIDATION_ERROR", "errorCode": "E001", "errorMessage": "Invalid security ID"}"#,
    )
    .await;
    assert_eq!(
        (err.kind(), err.stage(), err.http_status()),
        (ErrorKind::Api, Stage::ResponseReceived, Some(400))
    );
    assert_eq!(code(&err), Some(&ApiErrorCode::Other("E001".to_owned())));
    assert_eq!(message(&err), Some("Invalid security ID"));
    assert_eq!(
        err.api().unwrap().error_type.as_deref(),
        Some("VALIDATION_ERROR")
    );
}

#[tokio::test]
async fn credentials_echoed_in_a_broker_message_are_redacted() {
    let body = serde_json::json!({
        "errorType": "Input_Exception",
        "errorCode": "DH-905",
        "errorMessage": format!("bad request for {} with {}", support::mock::CLIENT_ID, support::mock::ACCESS_TOKEN),
    })
    .to_string();
    let err = fail_with(400, "application/json", &body).await;
    assert_eq!(err.kind(), ErrorKind::Api);
    let message = message(&err).expect("the message is kept, redacted");
    let rendered = format!("{err} {err:?} {message}");
    assert!(!rendered.contains(support::mock::CLIENT_ID), "{rendered}");
    assert!(
        !rendered.contains(support::mock::ACCESS_TOKEN),
        "{rendered}"
    );
    assert!(!rendered.contains("U0VOVElORUwtU0lH"), "{rendered}");
}
