//! Shows dhani's logs with a standard `tracing` setup (consumer setup example 1): INFO lifecycle
//! lines plus warnings and errors from dhani. Generating a token logs an INFO line; a REST call
//! answered first with a 503 logs a WARN retry line and then succeeds. Everything runs against a
//! local mock server. Against Dhan: drop the `.urls(..)` and `.http_client(..)` lines and pass
//! your own credentials; the logging setup stays the same.
//!
//! Run with `cargo run --example observability --features rest`.

#[path = "support/mod.rs"]
mod support;

use dhani::credentials::{Pin, Totp};
use dhani::{ClientId, DhanClient};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::main]
async fn main() -> dhani::Result<()> {
    // Example 1: INFO lifecycle and WARN/ERROR problems from dhani, nothing else changes.
    tracing_subscriber::fmt()
        .with_env_filter("info,dhani=info,dhani::decode=warn")
        .init();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/fundlimit"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/fundlimit"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"availabelBalance": 98440.0})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/app/generateAccessToken"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dhanClientId": "1000000009",
            "accessToken": "example-issued-token",
            "expiryTime": "2026-09-29T09:15:00"
        })))
        .mount(&server)
        .await;

    let anonymous = DhanClient::builder()
        .urls(support::urls_for(&server))
        .http_client(support::local_http_client())
        .build()?;
    // INFO: auth.token.issued (the token itself is never logged).
    let token = anonymous
        .auth()
        .generate_access_token(
            &ClientId::new("1000000009")?,
            &Pin::new("123456")?,
            &Totp::new("654321")?,
        )
        .await?;
    let client = anonymous.with_credentials(token.credentials());
    // The first answer is a 503: expect a retry line, then success.
    let limits = client.funds().limits().await?;
    println!("available balance: {:?}", limits.available_balance);
    Ok(())
}
