//! Limiter integration through the client: the quote window, a zero admission wait and the
//! per-order modification cap, against a wiremock server on a paused clock.
//!
//! The mock server runs on its own threads, so the test runtime's clock must not auto-advance
//! while a response is in flight. Calls are therefore driven by spinning on `yield_now` (the
//! runtime never parks, so the paused clock only moves when a test advances it) and time is
//! advanced explicitly only while a call is parked in admission.

mod support;

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use dhani::error::{RateLimitSource, Stage};
use dhani::labels::EndpointId;
use dhani::rest::{AdmissionLimits, QuotaProfile, RateLimiter, WallClock};
use dhani::types::{OrderId, RawJson};
use dhani::{DhanClient, ErrorKind};
use support::mock::{credentials, urls_for};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ORDER_ID: &str = "112111182198";

/// A fixed wall clock at noon IST, so no IST midnight falls inside a test.
struct Noon;

impl WallClock for Noon {
    fn unix_seconds(&self) -> i64 {
        // 2024-10-04 12:00:00 IST.
        1_728_023_400
    }
}

fn limiter(limits: AdmissionLimits) -> RateLimiter {
    RateLimiter::with_clock(QuotaProfile::dhan_v2(), limits, Arc::new(Noon))
}

async fn server_answering(verb: &str, route: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method(verb))
        .and(path(route))
        .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
        .mount(&server)
        .await;
    server
}

fn client(server: &MockServer, limiter: RateLimiter) -> DhanClient {
    DhanClient::builder()
        .urls(urls_for(server))
        .credentials(credentials())
        .rate_limiter(limiter)
        .build()
        .unwrap()
}

type Outcome = dhani::Result<Option<RawJson>>;

fn ltp(client: &DhanClient) -> impl Future<Output = Outcome> + Send + 'static {
    let client = client.clone();
    async move {
        let body = serde_json::json!({"NSE_EQ": [1333]});
        client
            .__execute_for_tests(EndpointId::MarketQuoteLtp, &[], &[], Some(body), None)
            .await
    }
}

fn modify(client: &DhanClient) -> impl Future<Output = Outcome> + Send + 'static {
    let client = client.clone();
    async move {
        let order = OrderId::new(ORDER_ID).unwrap();
        let body = serde_json::json!({"orderId": ORDER_ID, "orderType": "LIMIT", "quantity": 1});
        client
            .__execute_for_tests(
                EndpointId::OrdersModify,
                &[ORDER_ID],
                &[],
                Some(body),
                Some(&order),
            )
            .await
    }
}

/// Yields until `task` finishes, without advancing the clock.
async fn settle<T>(task: &JoinHandle<T>) -> bool {
    for _ in 0..2_000_000 {
        if task.is_finished() {
            return true;
        }
        tokio::task::yield_now().await;
    }
    false
}

/// Yields `rounds` times; whether `task` finished meanwhile.
async fn settle_for<T>(task: &JoinHandle<T>, rounds: usize) -> bool {
    for _ in 0..rounds {
        if task.is_finished() {
            return true;
        }
        tokio::task::yield_now().await;
    }
    task.is_finished()
}

/// Runs `call` to completion without advancing the clock.
async fn finish<T: Send + 'static>(call: impl Future<Output = T> + Send + 'static) -> T {
    let task = tokio::spawn(call);
    assert!(settle(&task).await, "the call did not finish");
    task.await.unwrap()
}

async fn requests(server: &MockServer) -> usize {
    server.received_requests().await.unwrap().len()
}

#[tokio::test(start_paused = true)]
async fn a_second_ltp_waits_for_the_quote_window() {
    let server = server_answering("POST", "/v2/marketfeed/ltp").await;
    let client = client(&server, limiter(AdmissionLimits::default()));
    let start = Instant::now();
    finish(ltp(&client)).await.unwrap();
    let second = tokio::spawn(ltp(&client));
    // Parked in admission: nothing completes, and nothing is sent, until the window frees.
    assert!(!settle_for(&second, 10_000).await);
    assert_eq!(requests(&server).await, 1);
    tokio::time::advance(Duration::from_millis(999)).await;
    assert!(!settle_for(&second, 10_000).await);
    assert_eq!(requests(&server).await, 1);
    // The proof of the 1 s wait: nothing moved until the window's last millisecond passed.
    tokio::time::advance(Duration::from_millis(1)).await;
    assert!(settle(&second).await, "the second call did not finish");
    second.await.unwrap().unwrap();
    assert!(Instant::now() - start >= Duration::from_secs(1));
    assert_eq!(requests(&server).await, 2);
}

#[tokio::test(start_paused = true)]
async fn with_no_admission_wait_the_second_ltp_is_refused() {
    let server = server_answering("POST", "/v2/marketfeed/ltp").await;
    let limits = AdmissionLimits::new(Duration::ZERO, 256).unwrap();
    let client = client(&server, limiter(limits));
    finish(ltp(&client)).await.unwrap();
    let err = finish(ltp(&client)).await.unwrap_err();
    assert_eq!(
        (err.kind(), err.stage(), err.attempts()),
        (ErrorKind::RateLimited, Stage::NotSent, 0)
    );
    assert_eq!(
        err.rate_limit().map(|r| r.source),
        Some(RateLimitSource::LocalWaitExceeded)
    );
    assert!(!err.may_have_reached_server());
    assert_eq!(requests(&server).await, 1);
}

#[tokio::test(start_paused = true)]
async fn the_twenty_sixth_modify_of_an_order_is_refused_locally() {
    let server = server_answering("PUT", &format!("/v2/orders/{ORDER_ID}")).await;
    let client = client(&server, limiter(AdmissionLimits::default()));
    // The order windows allow ten per second: send in batches and step the clock between them.
    for batch in [10, 10, 5] {
        for _ in 0..batch {
            finish(modify(&client)).await.unwrap();
        }
        tokio::time::advance(Duration::from_secs(1)).await;
    }
    assert_eq!(requests(&server).await, 25);
    let err = finish(modify(&client)).await.unwrap_err();
    assert_eq!(
        (err.kind(), err.stage(), err.attempts()),
        (ErrorKind::RateLimited, Stage::NotSent, 0)
    );
    assert_eq!(
        err.rate_limit().map(|r| r.source),
        Some(RateLimitSource::LocalCeiling)
    );
    assert!(!err.may_have_reached_server());
    assert_eq!(requests(&server).await, 25);
}
