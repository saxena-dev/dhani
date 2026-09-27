//! Live Order Update feed scenarios against the loopback WebSocket harness: the login message on
//! open, the 20 s client ping, and the decoding of order alerts, other messages and stray binary
//! frames.
//!
//! As in `feed_lifecycle.rs`, a paused clock is driven by spinning on `yield_now` and advancing
//! time only in 10 ms steps.

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dhani::credentials::{PartnerCredentials, PartnerId, PartnerSecret};
use dhani::decoder::DecodeErrorKind;
use dhani::feed::{FeedEvent, FeedEvents, FeedState, OrderUpdateEvent, OrderUpdateFeed};
use dhani::types::{Inbound, OrderStatus};
use dhani::{AccessToken, ClientId, Credentials};
use futures_util::StreamExt;
use support::ws::{ClientFrame, Step, WsConnection, WsHarness};

const CLIENT_ID: &str = "9999888877";
const TOKEN: &str = "eyJhbGciOiJIUzUxMiJ9.eyJzdWIiOiI5OTk5ODg4ODc3In0.U0VOVElORUwtU0lH";

fn credentials() -> Credentials {
    Credentials::new(
        ClientId::new(CLIENT_ID).unwrap(),
        AccessToken::new(TOKEN).unwrap(),
    )
}

/// The individual login message of DOC:6083-6092, with this test's credentials.
fn individual_login() -> serde_json::Value {
    serde_json::json!({
        "LoginReq": {"MsgCode": 42, "ClientId": CLIENT_ID, "Token": TOKEN},
        "UserType": "SELF"
    })
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

/// What the consumer read.
#[derive(Debug, PartialEq)]
enum Seen {
    Order(Option<Inbound<OrderStatus>>, Option<String>),
    Other(Option<String>),
    DecodeError(DecodeErrorKind),
    Life,
}

type Log = Arc<Mutex<Vec<Seen>>>;

fn collect(mut events: FeedEvents<OrderUpdateEvent>) -> Log {
    let log: Log = Arc::default();
    let sink = Arc::clone(&log);
    tokio::spawn(async move {
        while let Some(Ok(event)) = events.next().await {
            let seen = match event {
                FeedEvent::Data(d) => match d.value {
                    OrderUpdateEvent::Order(o) => Seen::Order(o.status, o.order_no),
                    OrderUpdateEvent::Other { kind, .. } => Seen::Other(kind),
                    other => panic!("unexpected {other:?}"),
                },
                FeedEvent::DecodeError(e) => Seen::DecodeError(e.kind),
                FeedEvent::Lifecycle(_) => Seen::Life,
                other => panic!("unexpected {other:?}"),
            };
            sink.lock().unwrap().push(seen);
        }
    });
    log
}

fn pings(harness: &WsHarness) -> usize {
    harness
        .frames(0)
        .iter()
        .filter(|f| **f == ClientFrame::Ping)
        .count()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_individual_login_is_sent_on_open_and_pings_follow_every_20_seconds() {
    let harness = WsHarness::start(vec![WsConnection::accept(vec![Step::ExpectText(
        individual_login(),
    )])])
    .await;
    let (handle, events, _task) = OrderUpdateFeed::builder(credentials())
        .url(harness.url())
        .spawn()
        .unwrap();
    let _log = collect(events);
    until("active", || handle.status().state == FeedState::Active).await;
    let opened = tokio::time::Instant::now();
    until("login", || harness.texts(0).len() == 1).await;
    assert_eq!(harness.texts(0), [individual_login()]);

    until("first ping", || pings(&harness) == 1).await;
    let first = tokio::time::Instant::now() - opened;
    until("second ping", || pings(&harness) == 2).await;
    let second = tokio::time::Instant::now() - opened;
    until("third ping", || pings(&harness) == 3).await;
    let third = tokio::time::Instant::now() - opened;
    // 10 ms is the driver's time step.
    let near = |d: Duration, s: u64| {
        (Duration::from_secs(s)..=Duration::from_secs(s) + Duration::from_millis(10)).contains(&d)
    };
    assert!(near(first, 20), "{first:?}");
    assert!(near(second, 40), "{second:?}");
    assert!(near(third, 60), "{third:?}");
    assert!(
        harness.mismatches().is_empty(),
        "{:?}",
        harness.mismatches()
    );
    // Past the 45 s liveness timeout: the pongs answering the pings kept the connection alive.
    assert_eq!((harness.connections(), handle.status().epoch), (1, 1));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_partner_login_is_sent_on_open() {
    let login = serde_json::json!({
        "LoginReq": {"MsgCode": 42, "ClientId": "PARTNER-7"},
        "UserType": "PARTNER",
        "Secret": "S3CRET-7"
    });
    let harness = WsHarness::start(vec![WsConnection::accept(vec![Step::ExpectText(
        login.clone(),
    )])])
    .await;
    let partner = PartnerCredentials {
        partner_id: PartnerId::new("PARTNER-7").unwrap(),
        partner_secret: PartnerSecret::new("S3CRET-7").unwrap(),
    };
    let (_handle, _events, _task) = OrderUpdateFeed::partner(partner)
        .url(harness.url())
        .spawn()
        .unwrap();
    until("login", || harness.texts(0).len() == 1).await;
    assert_eq!(harness.texts(0), [login]);
    assert!(
        harness.mismatches().is_empty(),
        "{:?}",
        harness.mismatches()
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn messages_decode_and_a_binary_frame_does_not_end_the_connection() {
    let sample = String::from_utf8(support::fixtures::raw_bytes(
        "tests/fixtures/synth/order_update.json",
    ))
    .unwrap();
    let harness = WsHarness::start(vec![WsConnection::accept(vec![
        Step::ExpectText(individual_login()),
        Step::SendText(sample.clone()),
        Step::SendText(r#"{"Type":"heartbeat_ack","Data":{}}"#.to_owned()),
        Step::SendBinary(vec![1, 2, 3]),
        Step::SendText(sample),
    ])])
    .await;
    let (handle, events, _task) = OrderUpdateFeed::builder(credentials())
        .url(harness.url())
        .spawn()
        .unwrap();
    let log = collect(events);
    let data = |log: &Log| {
        log.lock()
            .unwrap()
            .iter()
            .filter(|s| !matches!(s, Seen::Life))
            .count()
    };
    until("four items", || data(&log) == 4).await;
    let seen: Vec<Seen> = std::mem::take(&mut *log.lock().unwrap())
        .into_iter()
        .filter(|s| !matches!(s, Seen::Life))
        .collect();
    let order = || {
        Seen::Order(
            Some(Inbound::Known(OrderStatus::Cancelled)),
            Some("1124091136546".to_owned()),
        )
    };
    assert_eq!(
        seen,
        [
            order(),
            Seen::Other(Some("heartbeat_ack".to_owned())),
            Seen::DecodeError(DecodeErrorKind::UnexpectedBinary),
            order()
        ]
    );
    assert_eq!((harness.connections(), handle.status().epoch), (1, 1));
}
