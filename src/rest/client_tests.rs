//! Unit tests of the configuration types and the client builder.

use super::*;
use crate::ErrorKind;
use crate::credentials::{AccessToken, ClientId};

#[test]
fn config_types_reject_out_of_range_values() {
    let err = Timeouts::new(50 * MS, 15 * SEC, 30 * SEC).unwrap_err();
    assert_eq!(err.field, "connect");
    assert_eq!(
        Timeouts::new(SEC, 500 * MS, 30 * SEC).unwrap_err().field,
        "attempt"
    );
    assert_eq!(
        Timeouts::new(SEC, 15 * SEC, 10 * SEC).unwrap_err().field,
        "operation"
    );
    assert!(Timeouts::new(100 * MS, SEC, SEC).is_ok());

    assert_eq!(
        RetryPolicy::new(6, 250 * MS, 4 * SEC).unwrap_err().field,
        "max_attempts"
    );
    assert_eq!(
        RetryPolicy::new(0, 250 * MS, 4 * SEC).unwrap_err().field,
        "max_attempts"
    );
    assert_eq!(
        RetryPolicy::new(3, 10 * MS, 4 * SEC).unwrap_err().field,
        "initial_backoff"
    );
    assert_eq!(
        RetryPolicy::new(3, SEC, 500 * MS).unwrap_err().field,
        "max_backoff"
    );
    let p = RetryPolicy {
        rate_limit_retries: 4,
        ..RetryPolicy::default()
    };
    assert_eq!(p.validate().unwrap_err().field, "rate_limit_retries");
    let p = RetryPolicy {
        rate_limit_initial_backoff: 500 * MS,
        ..RetryPolicy::default()
    };
    assert_eq!(
        p.validate().unwrap_err().field,
        "rate_limit_initial_backoff"
    );

    assert_eq!(
        BodyLimits::new(512, 8 * MIB, 256 * MIB).unwrap_err().field,
        "max_request_bytes"
    );
    assert_eq!(
        BodyLimits::new(MIB, KIB, 256 * MIB).unwrap_err().field,
        "max_json_response_bytes"
    );
    assert_eq!(
        BodyLimits::new(MIB, 8 * MIB, 2048 * MIB).unwrap_err().field,
        "max_csv_response_bytes"
    );
}

#[test]
fn defaults_match_the_documented_values() {
    assert_eq!(
        Timeouts::default(),
        Timeouts {
            connect: 5 * SEC,
            attempt: 15 * SEC,
            operation: 30 * SEC
        }
    );
    let r = RetryPolicy::default();
    assert_eq!(
        (
            r.max_attempts,
            r.initial_backoff,
            r.max_backoff,
            r.rate_limit_retries,
            r.rate_limit_initial_backoff,
            r.jitter_seed
        ),
        (3, 250 * MS, 4 * SEC, 1, SEC, None)
    );
    assert_eq!(
        BodyLimits::default(),
        BodyLimits {
            max_request_bytes: 1 << 20,
            max_json_response_bytes: 8 << 20,
            max_csv_response_bytes: 256 << 20
        }
    );
    assert_eq!(RetryPolicy::none().max_attempts, 1);
    assert_eq!(
        RetryPolicy::default().with_jitter_seed(9).jitter_seed,
        Some(9)
    );
    assert!(Timeouts::default().validate().is_ok() && RetryPolicy::default().validate().is_ok());
    assert!(BodyLimits::default().validate().is_ok());
}

#[test]
fn build_validates_urls_and_config() {
    let mut urls = Urls::for_env(Environment::Live);
    urls.rest = url::Url::parse("http://x").unwrap();
    let err = DhanClient::builder().urls(urls).build().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Config);
    assert_eq!(err.config().map(|c| c.field), Some("urls.rest"));

    let bad = Timeouts {
        attempt: 500 * MS,
        ..Timeouts::default()
    };
    let err = DhanClient::builder().timeouts(bad).build().unwrap_err();
    assert_eq!(err.config().map(|c| c.field), Some("attempt"));

    let err = DhanClient::builder()
        .user_agent_suffix("bad\nsuffix")
        .build()
        .unwrap_err();
    assert_eq!(err.config().map(|c| c.field), Some("user_agent_suffix"));
}

#[test]
fn build_without_credentials_and_rotate_them() {
    let client = DhanClient::builder().build().unwrap();
    assert!(client.credentials().is_none());
    assert_eq!(client.environment(), Environment::Live);
    let creds = Credentials::new(
        ClientId::new("1000000009").unwrap(),
        AccessToken::new("token").unwrap(),
    );
    let rotated = client.with_credentials(creds);
    assert!(rotated.rate_limiter().ptr_eq(client.rate_limiter()));
    assert_eq!(
        rotated
            .credentials()
            .map(|c| c.client_id().expose_secret().to_owned())
            .as_deref(),
        Some("1000000009")
    );
    assert!(client.credentials().is_none());
    let text = format!("{rotated:?}");
    assert!(
        !text.contains("1000000009") && !text.contains("SENTINELTOKEN42"),
        "{text}"
    );

    let shared = RateLimiter::default();
    let a = DhanClient::builder()
        .rate_limiter(shared.clone())
        .build()
        .unwrap();
    let b = DhanClient::builder()
        .environment(Environment::Sandbox)
        .rate_limiter(shared.clone())
        .build()
        .unwrap();
    assert!(a.rate_limiter().ptr_eq(b.rate_limiter()));
    assert_eq!(b.environment(), Environment::Sandbox);
}

#[test]
fn the_user_agent_names_the_crate() {
    let client = DhanClient::builder()
        .user_agent_suffix("my-app/1.0")
        .build()
        .unwrap();
    assert_eq!(
        client.transport.user_agent,
        format!("dhani/{} my-app/1.0", env!("CARGO_PKG_VERSION"))
    );
    let client = DhanClient::builder().build().unwrap();
    assert_eq!(
        client.transport.user_agent,
        format!("dhani/{}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn facades_borrow_the_client() {
    let client = DhanClient::builder().build().unwrap();
    let _ = (
        client.orders(),
        client.super_orders(),
        client.forever_orders(),
        client.conditional(),
    );
    let _ = (
        client.portfolio(),
        client.funds(),
        client.statements(),
        client.trader_control(),
    );
    let _ = (
        client.edis(),
        client.market_quote(),
        client.historical(),
        client.option_chain(),
    );
    let _ = (client.account(), client.auth(), client.global());
    #[cfg(feature = "instruments")]
    let _ = client.instruments();
}

#[tokio::test]
async fn an_authenticated_call_without_credentials_fails_before_sending() {
    let client = DhanClient::builder().build().unwrap();
    let err = client.orders().list().await.unwrap_err();
    assert_eq!(
        (err.kind(), err.stage()),
        (ErrorKind::Config, crate::error::Stage::NotSent)
    );
}
