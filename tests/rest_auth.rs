//! Auth facade contract tests (§9 row A5): generate_access_token sends the documented query to
//! the auth host with no credential headers and no body, decodes the synthesised token, masks
//! the client ID, PIN and TOTP in stored errors, emits auth.token.issued, and is limited to one
//! call per 120 s.

mod support;

use std::sync::Arc;
use std::time::Duration;

use dhani::credentials::{Pin, Totp};
use dhani::error::{RateLimitSource, Stage};
use dhani::rest::{AdmissionLimits, QuotaProfile, RateLimiter, WallClock};
use dhani::{ClientId, DhanClient, ErrorKind};
use support::fixtures::synth;
use support::mock::urls_for;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CLIENT: &str = "1000000009";
const PIN: &str = "482913";
const TOTP: &str = "735162";

fn inputs() -> (ClientId, Pin, Totp) {
    (
        ClientId::new(CLIENT).unwrap(),
        Pin::new(PIN).unwrap(),
        Totp::new(TOTP).unwrap(),
    )
}

/// A client with no credentials: the auth host needs none.
fn anonymous(server: &MockServer, limiter: RateLimiter) -> DhanClient {
    DhanClient::builder()
        .urls(urls_for(server))
        .rate_limiter(limiter)
        .build()
        .unwrap()
}

async fn token_server(reply: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/app/generateAccessToken"))
        .and(query_param("dhanClientId", CLIENT))
        .and(query_param("pin", PIN))
        .and(query_param("totp", TOTP))
        .respond_with(reply)
        .mount(&server)
        .await;
    server
}

fn issued() -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_bytes(synth("auth_issued_token.json"))
}

#[tokio::test]
async fn a5_generate_access_token() {
    let (capture, _guard) = support::trace::install();
    let server = token_server(issued()).await;
    let (client_id, pin, totp) = inputs();
    let token = anonymous(&server, RateLimiter::disabled())
        .auth()
        .generate_access_token(&client_id, &pin, &totp)
        .await
        .unwrap();
    assert_eq!(token.dhan_client_id.expose_secret(), "string");
    assert_eq!(
        token.access_token.expose_secret(),
        "eyJhbGciOiJIUzUxMiJ9.eyJzdWIiOiJTRU5USU5FTC1KV1QifQ.U0VOVElORUwtU0lHTkFUVVJF"
    );
    assert_eq!(token.dhan_client_name.as_deref(), Some("string"));
    assert_eq!(token.dhan_client_ucc.as_deref(), Some("string"));
    assert_eq!(token.given_power_of_attorney, Some(true));
    assert_eq!(
        token.expiry_time.as_ref().map(|t| t.as_str()),
        Some("2024-09-11 14:39:29")
    );
    // Debug never shows the token.
    assert!(!format!("{token:?}").contains("eyJ"));

    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    let request = &received[0];
    for header in ["access-token", "client-id", "dhanclientid", "content-type"] {
        assert!(request.headers.get(header).is_none(), "{header}");
    }
    assert!(request.body.is_empty());

    let issued = capture.events_named("auth.token.issued");
    assert_eq!(issued.len(), 1);
    assert_eq!(issued[0].field("method"), Some("totp"));
    assert_eq!(
        (issued[0].level, issued[0].target),
        (tracing::Level::INFO, "dhani::auth")
    );
    assert_eq!(issued[0].field("expiry_present"), Some("true"));
}

#[tokio::test]
async fn a_401_is_an_auth_error_after_one_attempt_with_the_inputs_masked() {
    // The broker echoes the inputs in its message; the stored text must not.
    let body = format!(
        r#"{{"errorType":"Invalid_Authentication","errorCode":"DH-901","errorMessage":"client {CLIENT} pin {PIN} totp {TOTP} rejected"}}"#
    );
    let server = token_server(
        ResponseTemplate::new(401)
            .insert_header("content-type", "application/json")
            .set_body_string(body),
    )
    .await;
    let (client_id, pin, totp) = inputs();
    let err = anonymous(&server, RateLimiter::disabled())
        .auth()
        .generate_access_token(&client_id, &pin, &totp)
        .await
        .unwrap_err();
    assert_eq!((err.kind(), err.attempts()), (ErrorKind::Auth, 1));
    let rendered = format!(
        "{err} {err:?} {:?}",
        err.api().and_then(|a| a.error_message.as_ref())
    );
    for sentinel in [CLIENT, PIN, TOTP] {
        assert!(!rendered.contains(sentinel), "{sentinel} in {rendered}");
    }
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

// ---- One token every 120 s -------------------------------------------------------------------

/// A fixed wall clock at noon IST, so no IST midnight falls inside the test.
struct Noon;

impl WallClock for Noon {
    fn unix_seconds(&self) -> i64 {
        1_728_023_400
    }
}

/// Runs `call` to completion on the paused clock: spins so the runtime never parks, advancing
/// time in 100 ms steps only when the call is waiting.
async fn drive<T: Send + 'static>(
    call: impl std::future::Future<Output = T> + Send + 'static,
) -> T {
    let task = tokio::spawn(call);
    for _ in 0..10_000 {
        for _ in 0..64 {
            if task.is_finished() {
                return task.await.unwrap();
            }
            tokio::task::yield_now().await;
        }
        tokio::time::advance(Duration::from_millis(100)).await;
    }
    panic!("the call did not finish");
}

/// Runs `call` without moving the clock, for calls that must not wait.
async fn spin<T: Send + 'static>(call: impl std::future::Future<Output = T> + Send + 'static) -> T {
    let task = tokio::spawn(call);
    for _ in 0..2_000_000 {
        if task.is_finished() {
            return task.await.unwrap();
        }
        tokio::task::yield_now().await;
    }
    panic!("the call did not finish without the clock moving");
}

#[tokio::test(start_paused = true)]
async fn a_second_token_within_120_seconds_is_refused_locally() {
    let server = token_server(issued()).await;
    let limiter = RateLimiter::with_clock(
        QuotaProfile::dhan_v2(),
        AdmissionLimits::default(),
        Arc::new(Noon),
    );
    let client = anonymous(&server, limiter);
    let call = |client: &DhanClient| {
        let client = client.clone();
        async move {
            let (client_id, pin, totp) = inputs();
            client
                .auth()
                .generate_access_token(&client_id, &pin, &totp)
                .await
        }
    };
    let start = tokio::time::Instant::now();
    drive(call(&client)).await.unwrap();

    let refused = |err: dhani::Error| {
        assert_eq!(
            (err.kind(), err.stage(), err.attempts()),
            (ErrorKind::RateLimited, Stage::NotSent, 0)
        );
        assert_eq!(
            err.rate_limit().map(|r| r.source),
            Some(RateLimitSource::LocalWaitExceeded)
        );
    };
    // Refused at once: no local wait happened.
    let before = tokio::time::Instant::now();
    refused(spin(call(&client)).await.unwrap_err());
    assert_eq!(tokio::time::Instant::now(), before);

    // Still refused just short of the window (more than the 5 s admission wait remains).
    tokio::time::advance(Duration::from_secs(110) - (tokio::time::Instant::now() - start)).await;
    refused(spin(call(&client)).await.unwrap_err());
    assert_eq!(server.received_requests().await.unwrap().len(), 1);

    // Once the window has passed, a new token can be generated.
    tokio::time::advance(Duration::from_secs(10)).await;
    drive(call(&client)).await.unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_client_with_credentials_still_sends_none_to_the_auth_host() {
    let server = token_server(issued()).await;
    let client = support::mock::client_for(&server);
    let (client_id, pin, totp) = inputs();
    client
        .auth()
        .generate_access_token(&client_id, &pin, &totp)
        .await
        .unwrap();
    let received = server.received_requests().await.unwrap();
    for header in ["access-token", "client-id", "dhanclientid"] {
        assert!(received[0].headers.get(header).is_none(), "{header}");
    }
}
