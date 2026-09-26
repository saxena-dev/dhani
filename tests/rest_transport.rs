//! Transport pipeline tests. Against a wiremock server: credential headers by auth mode, the
//! client-id body injection, and nothing sent without credentials. Against the raw-TCP fault
//! harness: response loss, connection failures, retry and rate-limit timing, timeouts, truncated
//! and oversized bodies, and cancellation (architecture §10.4 cases 1–10). Requests are issued
//! through the client's test hook until each endpoint's facade exists.

mod support;

use std::future::Future;
use std::time::Duration;

use dhani::config::{Environment, Urls};
use dhani::error::{RateLimitSource, Stage};
use dhani::labels::EndpointId;
use dhani::rest::{BodyLimits, RateLimiter, Timeouts};
use dhani::{DhanClient, ErrorKind};
use support::fault_http::{FaultHttp, Reply, refused_base_url};
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

// ---- Fault harness (§10.4 cases 1–10) ----

/// Simulated time added per step while driving a call on the paused clock.
const STEP: Duration = Duration::from_millis(50);

const EMPTY_LIST: &str = "[]";

fn fault_client_with(
    base: &str,
    timeouts: Timeouts,
    limits: BodyLimits,
    limiter: RateLimiter,
) -> DhanClient {
    let mut urls = Urls::for_env(Environment::Live);
    urls.rest = url::Url::parse(&format!("{base}/v2")).unwrap();
    urls.auth = url::Url::parse(base).unwrap();
    DhanClient::builder()
        .urls(urls)
        .credentials(support::mock::credentials())
        .timeouts(timeouts)
        .limits(limits)
        .rate_limiter(limiter)
        .build()
        .unwrap()
}

fn fault_client(server: &FaultHttp) -> DhanClient {
    fault_client_with(
        &server.base_url(),
        Timeouts::default(),
        BodyLimits::default(),
        RateLimiter::disabled(),
    )
}

type Outcome = dhani::Result<Option<dhani::types::RawJson>>;

fn read(client: &DhanClient) -> impl Future<Output = Outcome> + Send + 'static {
    let client = client.clone();
    async move {
        client
            .__execute_for_tests(EndpointId::OrdersList, &[], &[], None, None)
            .await
    }
}

fn mutation(client: &DhanClient) -> impl Future<Output = Outcome> + Send + 'static {
    let client = client.clone();
    async move {
        let body = serde_json::json!({"transactionType": "BUY", "quantity": 1});
        client
            .__execute_for_tests(EndpointId::OrdersPlace, &[], &[], Some(body), None)
            .await
    }
}

/// Runs `call` to completion on the paused clock. The test task spins on `yield_now`, so the
/// runtime never parks and the clock cannot auto-advance while a loopback socket is pending;
/// time moves only through the explicit `STEP` advances. Time still advances one step per
/// round while loopback IO is in flight, so measured gaps can exceed the scheduled delay by a
/// few steps; upper bounds leave a full second of tolerance.
async fn drive<T: Send + 'static>(call: impl Future<Output = T> + Send + 'static) -> T {
    let task = tokio::spawn(call);
    // At most 20 000 steps (1 000 s of simulated time), far beyond any operation deadline.
    for _ in 0..20_000 {
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
        if task.is_finished() {
            return task.await.unwrap();
        }
        tokio::time::advance(STEP).await;
    }
    panic!("the call did not finish");
}

/// Spins, without advancing the clock, until the server has recorded `n` requests.
async fn wait_for_requests(server: &FaultHttp, n: usize) {
    for _ in 0..100_000 {
        if server.requests().len() >= n {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("the server never recorded {n} requests");
}

fn gap(server: &FaultHttp, first: usize, second: usize) -> Duration {
    let r = server.requests();
    r[second].at - r[first].at
}

// Case 1.
#[tokio::test(start_paused = true)]
async fn a_mutation_whose_response_is_lost_is_not_resent() {
    let server = FaultHttp::start(vec![Reply::DropAfterRequest, Reply::json(200, "{}")]).await;
    let err = drive(mutation(&fault_client(&server))).await.unwrap_err();
    assert_eq!(
        (err.kind(), err.stage(), err.attempts()),
        (ErrorKind::Transport, Stage::Sent, 1)
    );
    assert!(err.may_have_reached_server());
    let requests = server.requests();
    assert_eq!((requests.len(), server.connections()), (1, 1));
    assert_eq!(
        (requests[0].method.as_str(), requests[0].target.as_str()),
        ("POST", "/v2/orders")
    );
}

// Case 2.
#[tokio::test(start_paused = true)]
async fn a_read_retries_a_connection_closed_on_accept() {
    let server = FaultHttp::start(vec![Reply::CloseOnAccept, Reply::json(200, EMPTY_LIST)]).await;
    let body = drive(read(&fault_client(&server))).await.unwrap();
    assert_eq!(body.unwrap().0, serde_json::json!([]));
    // Two attempts: the first connection carried no request.
    assert_eq!((server.connections(), server.requests().len()), (2, 1));
}

// Case 3.
#[tokio::test(start_paused = true)]
async fn a_read_gives_up_after_three_503s() {
    let unavailable = || Reply::text(503, "Service Unavailable");
    let server = FaultHttp::start(vec![
        unavailable(),
        unavailable(),
        unavailable(),
        Reply::json(200, EMPTY_LIST),
    ])
    .await;
    let err = drive(read(&fault_client(&server))).await.unwrap_err();
    assert_eq!(
        (err.kind(), err.stage(), err.http_status(), err.attempts()),
        (ErrorKind::HttpStatus, Stage::ResponseReceived, Some(503), 3)
    );
    assert_eq!(server.requests().len(), 3);
}

// Case 4.
#[tokio::test(start_paused = true)]
async fn a_read_does_not_retry_a_500() {
    let server = FaultHttp::start(vec![
        Reply::text(500, "Internal Server Error"),
        Reply::json(200, EMPTY_LIST),
    ])
    .await;
    let err = drive(read(&fault_client(&server))).await.unwrap_err();
    assert_eq!(
        (err.kind(), err.http_status(), err.attempts()),
        (ErrorKind::HttpStatus, Some(500), 1)
    );
    assert_eq!(server.requests().len(), 1);
}

// Case 5.
#[tokio::test(start_paused = true)]
async fn a_read_waits_at_least_a_second_after_a_429() {
    let server = FaultHttp::start(vec![
        Reply::text(429, "Too Many Requests"),
        Reply::json(200, EMPTY_LIST),
    ])
    .await;
    drive(read(&fault_client(&server))).await.unwrap();
    assert_eq!(server.requests().len(), 2);
    let waited = gap(&server, 0, 1);
    assert!(
        waited >= Duration::from_secs(1) && waited < Duration::from_secs(2),
        "{waited:?}"
    );
}

// Case 5.
#[tokio::test(start_paused = true)]
async fn a_second_429_ends_the_read_as_a_remote_rate_limit() {
    let limited = || Reply::text(429, "Too Many Requests");
    let server = FaultHttp::start(vec![limited(), limited(), Reply::json(200, EMPTY_LIST)]).await;
    let err = drive(read(&fault_client(&server))).await.unwrap_err();
    assert_eq!(
        (err.kind(), err.http_status(), err.attempts()),
        (ErrorKind::RateLimited, Some(429), 2)
    );
    assert_eq!(
        err.rate_limit().map(|r| r.source),
        Some(RateLimitSource::Remote)
    );
    assert_eq!(server.requests().len(), 2);
}

// Case 5.
#[tokio::test(start_paused = true)]
async fn a_mutation_is_not_retried_after_a_429() {
    let server = FaultHttp::start(vec![
        Reply::text(429, "Too Many Requests"),
        Reply::json(200, "{}"),
    ])
    .await;
    let err = drive(mutation(&fault_client(&server))).await.unwrap_err();
    assert_eq!((err.kind(), err.attempts()), (ErrorKind::RateLimited, 1));
    assert_eq!(server.requests().len(), 1);
}

// Case 5 (P13).
#[tokio::test(start_paused = true)]
async fn retry_after_is_honoured() {
    let limited = Reply::Respond {
        status: 429,
        headers: vec![("retry-after", "3".to_owned())],
        body: Vec::new(),
    };
    let server = FaultHttp::start(vec![limited, Reply::json(200, EMPTY_LIST)]).await;
    drive(read(&fault_client(&server))).await.unwrap();
    let waited = gap(&server, 0, 1);
    assert!(
        waited >= Duration::from_secs(3) && waited < Duration::from_secs(4),
        "{waited:?}"
    );
}

// Case 6.
#[tokio::test(start_paused = true)]
async fn a_stalled_read_is_retried_until_the_operation_deadline() {
    let server = FaultHttp::start(vec![Reply::Stall, Reply::Stall, Reply::Stall]).await;
    let timeouts = Timeouts::new(
        Duration::from_secs(1),
        Duration::from_secs(1),
        Duration::from_secs(3),
    )
    .unwrap();
    let client = fault_client_with(
        &server.base_url(),
        timeouts,
        BodyLimits::default(),
        RateLimiter::disabled(),
    );
    let started = tokio::time::Instant::now();
    let err = drive(read(&client)).await.unwrap_err();
    let elapsed = started.elapsed();
    assert_eq!(err.kind(), ErrorKind::Timeout);
    assert!(err.is_timeout());
    assert!(err.attempts() >= 2, "{}", err.attempts());
    // Each attempt is bounded by the 1 s attempt timeout; the whole call by the 3 s operation.
    let requests = server.requests();
    assert!(requests.len() >= 2);
    assert!(gap(&server, 0, 1) >= Duration::from_secs(1));
    assert!(elapsed <= Duration::from_secs(3) + STEP, "{elapsed:?}");
}

// Case 6, mutation: the send-stage timeout is final.
#[tokio::test(start_paused = true)]
async fn a_stalled_mutation_times_out_once() {
    let server = FaultHttp::start(vec![Reply::Stall, Reply::Stall]).await;
    let timeouts = Timeouts::new(
        Duration::from_secs(1),
        Duration::from_secs(1),
        Duration::from_secs(3),
    )
    .unwrap();
    let client = fault_client_with(
        &server.base_url(),
        timeouts,
        BodyLimits::default(),
        RateLimiter::disabled(),
    );
    let err = drive(mutation(&client)).await.unwrap_err();
    assert_eq!(
        (err.kind(), err.stage(), err.attempts()),
        (ErrorKind::Timeout, Stage::Sent, 1)
    );
    assert!(err.may_have_reached_server());
    assert_eq!(server.requests().len(), 1);
}

// Case 7.
#[tokio::test(start_paused = true)]
async fn a_truncated_body_is_not_retried() {
    let truncated = Reply::TruncateBody {
        status: 200,
        declared: 64,
        sent: 10,
    };
    let server = FaultHttp::start(vec![truncated, Reply::json(200, EMPTY_LIST)]).await;
    let err = drive(read(&fault_client(&server))).await.unwrap_err();
    assert_eq!(
        (err.kind(), err.stage(), err.http_status(), err.attempts()),
        (ErrorKind::Transport, Stage::ResponseReceived, Some(200), 1)
    );
    assert_eq!(server.requests().len(), 1);
}

// Case 8.
#[tokio::test(start_paused = true)]
async fn a_refused_connection_is_not_sent() {
    let base = refused_base_url().await;
    let client = fault_client_with(
        &base,
        Timeouts::default(),
        BodyLimits::default(),
        RateLimiter::disabled(),
    );
    let err = drive(read(&client)).await.unwrap_err();
    // A read retries the refusal up to the default three attempts.
    assert_eq!(
        (err.kind(), err.stage(), err.attempts()),
        (ErrorKind::Transport, Stage::NotSent, 3)
    );
    assert!(!err.may_have_reached_server());
    let err = drive(mutation(&client)).await.unwrap_err();
    assert_eq!(
        (err.kind(), err.stage(), err.attempts()),
        (ErrorKind::Transport, Stage::NotSent, 1)
    );
}

// Case 9.
#[tokio::test(start_paused = true)]
async fn an_oversized_body_is_a_decode_error() {
    const BOUND: usize = 64 * 1024;
    let limits = BodyLimits::new(1024 * 1024, BOUND, 1024 * 1024).unwrap();
    let oversized = || {
        let mut body = vec![b' '; BOUND];
        body.extend_from_slice(b"[]");
        body
    };
    // Announced by Content-Length, then streamed without one.
    let replies = vec![
        Reply::json(200, oversized()),
        Reply::RespondChunked {
            status: 200,
            body: oversized(),
        },
    ];
    let server = FaultHttp::start(replies).await;
    let client = fault_client_with(
        &server.base_url(),
        Timeouts::default(),
        limits,
        RateLimiter::disabled(),
    );
    for _ in 0..2 {
        let err = drive(read(&client)).await.unwrap_err();
        assert_eq!(
            (err.kind(), err.stage(), err.http_status(), err.attempts()),
            (ErrorKind::Decode, Stage::ResponseReceived, Some(200), 1)
        );
        assert_eq!(err.detail(), Some("response body exceeds its bound"));
    }
    assert_eq!(server.requests().len(), 2);
}

// Case 9, boundary: a body of exactly the bound is accepted.
#[tokio::test(start_paused = true)]
async fn a_body_at_the_bound_is_accepted() {
    const BOUND: usize = 64 * 1024;
    let limits = BodyLimits::new(1024 * 1024, BOUND, 1024 * 1024).unwrap();
    let mut body = vec![b' '; BOUND - 2];
    body.extend_from_slice(b"[]");
    let server = FaultHttp::start(vec![Reply::RespondChunked { status: 200, body }]).await;
    let client = fault_client_with(
        &server.base_url(),
        Timeouts::default(),
        limits,
        RateLimiter::disabled(),
    );
    assert_eq!(
        drive(read(&client)).await.unwrap().unwrap().0,
        serde_json::json!([])
    );
}

// Case 10. The admission shim never waits, so the future is dropped before its first poll;
// a drop while `acquire` is waiting on a saturated limiter is added with the real limiter.
#[tokio::test(start_paused = true)]
async fn a_call_dropped_before_dispatch_sends_nothing() {
    let server = FaultHttp::start(vec![Reply::json(200, EMPTY_LIST)]).await;
    let limiter = RateLimiter::default();
    let client = fault_client_with(
        &server.base_url(),
        Timeouts::default(),
        BodyLimits::default(),
        limiter.clone(),
    );
    let before = limiter.__outstanding();
    let call = read(&client);
    drop(call);
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
    assert_eq!(limiter.__outstanding(), before);
    assert_eq!(server.connections(), 0);
}

// Case 10, after dispatch: a cancellation smoke test. The grant was consumed by the send, so
// nothing is left outstanding and the request was recorded once.
#[tokio::test(start_paused = true)]
async fn a_call_dropped_in_flight_leaves_no_grant_outstanding() {
    let server = FaultHttp::start(vec![Reply::Stall]).await;
    let limiter = RateLimiter::default();
    let client = fault_client_with(
        &server.base_url(),
        Timeouts::default(),
        BodyLimits::default(),
        limiter.clone(),
    );
    let task = tokio::spawn(read(&client));
    wait_for_requests(&server, 1).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(limiter.__outstanding(), 0);
    assert_eq!(server.requests().len(), 1);
}

// Wire level: a bodiless request carries no Content-Type.
#[tokio::test(start_paused = true)]
async fn a_bodiless_delete_has_no_content_type() {
    let server = FaultHttp::start(vec![Reply::json(
        200,
        r#"{"orderId":"112111182198","orderStatus":"CANCELLED"}"#,
    )])
    .await;
    let client = fault_client(&server);
    let call = async move {
        client
            .__execute_for_tests(EndpointId::OrdersCancel, &["112111182198"], &[], None, None)
            .await
    };
    drive(call).await.unwrap();
    let requests = server.requests();
    let request = &requests[0];
    assert_eq!(
        (request.method.as_str(), request.target.as_str()),
        ("DELETE", "/v2/orders/112111182198")
    );
    assert_eq!(request.header("content-type"), None);
    assert!(request.body.is_empty());
    assert_eq!(
        request.header("access-token"),
        Some(support::mock::ACCESS_TOKEN)
    );
}
