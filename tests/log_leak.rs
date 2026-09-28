//! Secrets in dependency logs (M11 epic review): tungstenite logs the handshake request (the
//! market feed URL, with the token and client ID) and every outgoing frame (the order-update
//! login) at TRACE through the `log` crate. With `log` forwarded into `tracing`, this proves the
//! leak is confined to tungstenite's target, never dhani's, and that the documented filter
//! (`tungstenite=debug`) removes it.
//!
//! This test installs a process-wide `log` logger, so it lives in its own binary.

mod support;

use std::time::Duration;

use dhani::feed::{FeedState, MarketFeed, OrderUpdateFeed};
use dhani::{AccessToken, ClientId, Credentials};
use support::trace::{Capture, install_with_filter};
use support::ws::{Step, WsConnection, WsHarness};

const CLIENT: &str = "9999888877";
const TOKEN: &str = "eyJhbGciOiJIUzUxMiJ9.eyJzdWIiOiI5OTk5ODg4ODc3In0.U0VOVElORUwtU0lH";

fn credentials() -> Credentials {
    Credentials::new(
        ClientId::new(CLIENT).unwrap(),
        AccessToken::new(TOKEN).unwrap(),
    )
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..100_000 {
        for _ in 0..64 {
            if done() {
                return;
            }
            tokio::task::yield_now().await;
        }
        tokio::time::advance(Duration::from_millis(10)).await;
    }
    panic!("never reached: {what}");
}

/// Connects a market feed and an order-update feed (which sends its login) under `directives`.
async fn connect_both(directives: &str) -> Capture {
    // Forward `log` records into `tracing`; the first call installs the process-wide logger.
    let _ = tracing_log::LogTracer::init();
    let (capture, _guard) = install_with_filter(directives);
    let harness = WsHarness::start(vec![WsConnection::accept(vec![])]).await;
    let (market, _events, _task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .spawn()
        .unwrap();
    until("market active", || {
        market.status().state == FeedState::Active
    })
    .await;
    let harness = WsHarness::start(vec![WsConnection::accept(vec![Step::Wait(
        Duration::from_secs(60),
    )])])
    .await;
    let (updates, _events, _task) = OrderUpdateFeed::builder(credentials())
        .url(harness.url())
        .spawn()
        .unwrap();
    until("login sent", || !harness.texts(0).is_empty()).await;
    assert_eq!(updates.status().state, FeedState::Active);
    capture
}

/// `(target, text)` of every captured event and span field that contains a secret.
fn leaks(capture: &Capture) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for event in capture.events() {
        // Records forwarded from `log` have the target "log" and carry the original one in
        // the `log.target` field.
        let target = event.field("log.target").unwrap_or(event.target).to_owned();
        for value in event.fields.values() {
            if value.contains(TOKEN) || value.contains(CLIENT) {
                found.push((target.clone(), value.clone()));
            }
        }
    }
    for span in capture.spans() {
        for value in span.fields.values() {
            if value.contains(TOKEN) || value.contains(CLIENT) {
                found.push((span.target.to_owned(), value.clone()));
            }
        }
    }
    found
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn at_trace_only_tungstenite_logs_the_secrets_and_the_filter_removes_them() {
    let capture = connect_both("trace").await;
    let found = leaks(&capture);
    // The dependency leak exists at TRACE (this is why the feed docs warn about it) ...
    assert!(
        found.iter().any(|(t, _)| t.starts_with("tungstenite")),
        "expected tungstenite's TRACE records to carry the secrets"
    );
    // ... and only there: nothing from dhani or any other target.
    for (target, text) in &found {
        assert!(target.starts_with("tungstenite"), "{target}: {text}");
    }

    let capture = connect_both("trace,tungstenite=debug").await;
    assert!(leaks(&capture).is_empty(), "{:?}", leaks(&capture));
}
