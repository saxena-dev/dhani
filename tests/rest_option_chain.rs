//! Option chain contract tests (§9 rows X1–X2): method, exact path, credential headers and the
//! exact PascalCase body with the injected dhanClientId; the fixtures decode to hand-written
//! values. The keyed window holds a repeat of the same underlying and expiry for three seconds
//! and lets a different key through.
//!
//! The keyed-window test drives calls by spinning on `yield_now` so the paused clock moves only
//! when the test advances it (as in `rate_limit.rs`).

mod support;

use std::sync::Arc;
use std::time::Duration;

use chrono::NaiveDate;
use dhani::rest::{
    AdmissionLimits, OptionChainRequest, QuotaProfile, RateLimiter, UnderlyingRef, WallClock,
};
use dhani::types::{ExchangeSegment, RawJson};
use dhani::{DhanClient, ErrorKind};
use serde_json::json;
use support::fixtures::{synth, upstream_payload};
use support::mock::{CLIENT_ID, body_json_eq, client_for, credentials, expect_headers, urls_for};
use tokio::task::JoinHandle;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn expiry() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 10, 31).unwrap()
}

fn nifty() -> UnderlyingRef {
    UnderlyingRef::new(13, ExchangeSegment::IdxI)
}

fn chain_request() -> OptionChainRequest {
    OptionChainRequest::new(nifty(), expiry())
}

fn json_reply(body: Vec<u8>) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_bytes(body)
}

async fn serve(route: &str, body: serde_json::Value, reply: Vec<u8>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(route))
        .and(expect_headers())
        .and(header("content-type", "application/json"))
        .and(body_json_eq(body))
        .respond_with(json_reply(reply))
        .expect(1)
        .mount(&server)
        .await;
    server
}

fn chain_body() -> serde_json::Value {
    json!({
        "UnderlyingScrip": 13,
        "UnderlyingSeg": "IDX_I",
        "Expiry": "2024-10-31",
        "dhanClientId": CLIENT_ID
    })
}

// ---- X1 --------------------------------------------------------------------------------------

// row: X1
#[tokio::test]
async fn x1_chain_decodes_two_strikes_with_greeks() {
    let server = serve(
        "/v2/optionchain",
        chain_body(),
        synth("option_chain_data.json"),
    )
    .await;
    let chain = client_for(&server)
        .option_chain()
        .chain(&chain_request())
        .await
        .unwrap();
    assert_eq!(chain.last_price, Some(1.5));
    let strikes: Vec<f64> = chain.strikes().iter().map(|(s, _)| *s).collect();
    assert_eq!(strikes, [25000.0, 25100.0]);
    for (_, row) in chain.strikes() {
        let ce = row.ce.as_ref().unwrap();
        assert_eq!(ce.greeks.as_ref().and_then(|g| g.delta), Some(1.5));
        assert_eq!(
            (ce.oi, ce.security_id, ce.volume),
            (Some(1), Some(1), Some(1))
        );
        assert_eq!(
            (ce.top_bid_price, ce.top_ask_quantity),
            (Some(1.5), Some(1))
        );
        let pe = row.pe.as_ref().unwrap();
        assert_eq!(pe.implied_volatility, Some(1.5));
    }
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

// row: X1
#[tokio::test]
async fn x1_chain_raw_returns_the_body_unchanged() {
    let server = serve(
        "/v2/optionchain",
        chain_body(),
        synth("option_chain_data.json"),
    )
    .await;
    let raw: RawJson = client_for(&server)
        .option_chain()
        .chain_raw(&chain_request())
        .await
        .unwrap();
    let expected: serde_json::Value =
        serde_json::from_slice(&synth("option_chain_data.json")).unwrap();
    assert_eq!(raw.0, expected);
}

// row: X1
#[tokio::test]
async fn x1_chain_raw_reads_the_upstream_placeholder() {
    // The Python SDK's fixture payload (its `data` member, as the loader serves it) is not a
    // chain shape; the raw call returns it unchanged.
    let server = serve(
        "/v2/optionchain",
        chain_body(),
        upstream_payload("option_chain.json"),
    )
    .await;
    let raw = client_for(&server)
        .option_chain()
        .chain_raw(&chain_request())
        .await
        .unwrap();
    assert_eq!(
        raw.0,
        json!({"last_price": 20000.0, "implied_volatility": 12.5, "oi": 100000, "volume": 50000})
    );
}

// ---- X2 --------------------------------------------------------------------------------------

fn expiries_body() -> serde_json::Value {
    json!({"UnderlyingScrip": 13, "UnderlyingSeg": "IDX_I", "dhanClientId": CLIENT_ID})
}

// row: X2
#[tokio::test]
async fn x2_expiries_decode_as_dates() {
    let server = serve(
        "/v2/optionchain/expirylist",
        expiries_body(),
        synth("expiry_list.json"),
    )
    .await;
    let dates = client_for(&server)
        .option_chain()
        .expiries(&nifty())
        .await
        .unwrap();
    assert_eq!(
        dates,
        [expiry(), NaiveDate::from_ymd_opt(2024, 11, 28).unwrap()]
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

// row: X2
#[tokio::test]
async fn x2_an_unparsable_expiry_is_a_decode_error() {
    let server = serve(
        "/v2/optionchain/expirylist",
        expiries_body(),
        br#"{"data":["x"]}"#.to_vec(),
    )
    .await;
    let err = client_for(&server)
        .option_chain()
        .expiries(&nifty())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Decode);
}

#[tokio::test]
async fn a_zero_scrip_sends_nothing() {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let client: DhanClient = client_for(&server);
    let zero = UnderlyingRef::new(0, ExchangeSegment::IdxI);
    let err = client.option_chain().expiries(&zero).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    let err = client
        .option_chain()
        .chain(&OptionChainRequest::new(zero, expiry()))
        .await
        .unwrap_err();
    assert_eq!(err.validation().map(|v| v.field), Some("scrip"));
    assert!(server.received_requests().await.unwrap().is_empty());
}

// ---- The keyed window ------------------------------------------------------------------------

/// A fixed wall clock at noon IST, so no IST midnight falls inside the test.
struct Noon;

impl WallClock for Noon {
    fn unix_seconds(&self) -> i64 {
        1_728_023_400
    }
}

/// Yields until `task` finishes or `rounds` pass, without advancing the clock.
async fn settle_for<T>(task: &JoinHandle<T>, rounds: usize) -> bool {
    for _ in 0..rounds {
        if task.is_finished() {
            return true;
        }
        tokio::task::yield_now().await;
    }
    task.is_finished()
}

#[tokio::test(start_paused = true)]
async fn a_repeat_of_the_same_chain_waits_three_seconds_and_another_key_does_not() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/optionchain"))
        .respond_with(json_reply(
            br#"{"data":{"last_price":1.0,"oc":{}}}"#.to_vec(),
        ))
        .mount(&server)
        .await;
    let limiter = RateLimiter::with_clock(
        QuotaProfile::dhan_v2(),
        AdmissionLimits::default(),
        Arc::new(Noon),
    );
    let client = DhanClient::builder()
        .urls(urls_for(&server))
        .credentials(credentials())
        .rate_limiter(limiter)
        .build()
        .unwrap();
    let call = |client: &DhanClient, req: OptionChainRequest| {
        let client = client.clone();
        tokio::spawn(async move { client.option_chain().chain(&req).await })
    };
    let requests = || async { server.received_requests().await.unwrap().len() };

    let first = call(&client, chain_request());
    assert!(settle_for(&first, 2_000_000).await);
    first.await.unwrap().unwrap();

    // Same key: parked in admission until three seconds have passed.
    let repeat = call(&client, chain_request());
    // Different expiry: a different key, sent at once (the Data class still has room).
    let other_expiry = NaiveDate::from_ymd_opt(2024, 11, 28).unwrap();
    let other = call(&client, OptionChainRequest::new(nifty(), other_expiry));
    assert!(
        settle_for(&other, 2_000_000).await,
        "the other key was held"
    );
    other.await.unwrap().unwrap();
    assert!(!settle_for(&repeat, 10_000).await);
    assert_eq!(requests().await, 2);

    tokio::time::advance(Duration::from_millis(2_999)).await;
    assert!(!settle_for(&repeat, 10_000).await);
    assert_eq!(requests().await, 2);
    tokio::time::advance(Duration::from_millis(1)).await;
    assert!(settle_for(&repeat, 2_000_000).await, "the repeat never ran");
    repeat.await.unwrap().unwrap();
    assert_eq!(requests().await, 3);
}
