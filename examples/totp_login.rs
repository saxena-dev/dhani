//! Generates an access token from a PIN and a TOTP code, switches the client to it and checks
//! it with the profile call, against a local mock server.
//!
//! Run with `cargo run --example totp_login --features rest`. dhani does not compute TOTP codes:
//! take the current code from your authenticator app. Dhan issues one token every two minutes.
//! Against Dhan: drop the `.urls(..)` and `.http_client(..)` lines and pass your own client
//! ID, PIN and current TOTP.

#[path = "support/mod.rs"]
mod support;

use dhani::credentials::{Pin, Totp};
use dhani::{ClientId, DhanClient};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::main]
async fn main() -> dhani::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/app/generateAccessToken"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dhanClientId": "1000000009",
            "dhanClientName": "EXAMPLE USER",
            "accessToken": "example-issued-token",
            "expiryTime": "2026-09-29T09:15:00"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/profile"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"dhanClientId": "1000000009"})),
        )
        .mount(&server)
        .await;

    // The auth host needs no credentials: build the client without them.
    let anonymous = DhanClient::builder()
        .urls(support::urls_for(&server))
        .http_client(support::local_http_client())
        .build()?;
    let token = anonymous
        .auth()
        .generate_access_token(
            &ClientId::new("1000000009")?,
            &Pin::new("123456")?,
            &Totp::new("654321")?,
        )
        .await?;
    // Debug never shows the token.
    println!("issued: {token:?}");
    println!(
        "expires: {}",
        token.expiry_time.as_ref().map_or("?", |t| t.as_str())
    );

    // Same transport and rate limiter, new credentials.
    let client = anonymous.with_credentials(token.credentials());
    let profile = client.account().profile().await?;
    println!("profile keys: {profile:?}");
    Ok(())
}
