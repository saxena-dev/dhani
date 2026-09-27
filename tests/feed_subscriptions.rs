//! Subscription wire scenarios against the loopback WebSocket harness: chunking, mode changes,
//! minimal diffs, unchanged commands, the capacity bound, commands during backoff and a peer
//! close. Structured so the depth and global feeds add their own scenarios alongside.
//!
//! As in `feed_lifecycle.rs`, a paused clock is driven by spinning on `yield_now` and advancing
//! time only when nothing else can progress.

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dhani::decoder::MarketPacket;
use dhani::feed::{
    CommandError, DisconnectReason, FeedEvent, FeedEvents, FeedHandle, FeedLimits, Instrument,
    Lifecycle, MarketFeed, MarketSub, Mode, Revision, SubscriptionError,
};
use dhani::types::{ExchangeSegment, SecurityId};
use dhani::{AccessToken, ClientId, Credentials};
use futures_util::StreamExt;
use support::ws::{Step, WsConnection, WsHarness};

fn credentials() -> Credentials {
    Credentials::new(
        ClientId::new("9999888877").unwrap(),
        AccessToken::new("eyJhbGciOiJIUzUxMiJ9.eyJzdWIiOiI5OTk5ODg4ODc3In0.U0VOVElORUwtU0lH")
            .unwrap(),
    )
}

fn nse(id: u32) -> Instrument {
    Instrument::new(
        ExchangeSegment::NseEq,
        SecurityId::new(id.to_string()).unwrap(),
    )
    .unwrap()
}

fn limits() -> FeedLimits {
    let mut limits = FeedLimits::default();
    limits.reconnect.jitter_seed = Some(7);
    limits
}

type Log = Arc<Mutex<Vec<Lifecycle>>>;

/// Records the lifecycle events of the stream.
fn collect(mut events: FeedEvents<MarketPacket>) -> Log {
    let log: Log = Arc::default();
    let sink = Arc::clone(&log);
    tokio::spawn(async move {
        while let Some(Ok(event)) = events.next().await {
            if let FeedEvent::Lifecycle(l) = event {
                sink.lock().unwrap().push(l);
            }
        }
    });
    log
}

fn active(log: &Log, epoch: u64) -> bool {
    log.lock()
        .unwrap()
        .iter()
        .any(|l| matches!(l, Lifecycle::Active { epoch: e, .. } if *e == epoch))
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

/// Runs `command` to completion while driving the runtime.
async fn run<T: Send + 'static>(
    command: impl std::future::Future<Output = T> + Send + 'static,
) -> T {
    let task = tokio::spawn(command);
    until("command reply", || task.is_finished()).await;
    task.await.unwrap()
}

/// A live market feed on one accepted connection with `steps`, after its first Active.
async fn live(steps: Vec<Step>) -> (WsHarness, FeedHandle<MarketSub>, Log) {
    let harness = WsHarness::start(vec![WsConnection::accept(steps)]).await;
    let (handle, events, _task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .limits(limits())
        .spawn()
        .unwrap();
    let log = collect(events);
    until("active", || active(&log, 1)).await;
    (harness, handle, log)
}

/// `(RequestCode, security IDs)` of each text frame on connection `index`.
fn requests(harness: &WsHarness, index: usize) -> Vec<(u64, Vec<String>)> {
    harness
        .texts(index)
        .iter()
        .map(|v| {
            let ids = v["InstrumentList"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .map(|i| i["SecurityId"].as_str().expect("a string id").to_owned())
                        .collect()
                })
                .unwrap_or_default();
            (v["RequestCode"].as_u64().unwrap(), ids)
        })
        .collect()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_hundred_and_fifty_full_instruments_go_out_in_two_messages() {
    let (harness, handle, log) = live(vec![]).await;
    let h = handle.clone();
    let revision = run(async move { h.subscribe((1..=150).map(nse), Mode::Full).await })
        .await
        .unwrap();
    until("two messages", || harness.texts(0).len() == 2).await;
    let sent = requests(&harness, 0);
    assert_eq!((sent[0].0, sent[0].1.len()), (21, 100));
    assert_eq!((sent[1].0, sent[1].1.len()), (21, 50));
    assert_eq!(
        (sent[0].1[0].as_str(), sent[1].1[49].as_str()),
        ("1", "150")
    );
    let texts = harness.texts(0);
    assert_eq!(
        (
            texts[0]["InstrumentCount"].as_u64(),
            texts[1]["InstrumentCount"].as_u64()
        ),
        (Some(100), Some(50))
    );
    assert_eq!(texts[1]["InstrumentList"][0]["ExchangeSegment"], "NSE_EQ");
    until("commands sent", || {
        log.lock()
            .unwrap()
            .iter()
            .any(|l| matches!(l, Lifecycle::CommandsSent { revision: r } if *r == revision))
    })
    .await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_mode_change_unsubscribes_then_subscribes() {
    let (harness, handle, _log) = live(vec![]).await;
    let h = handle.clone();
    run(async move { h.subscribe([nse(1333)], Mode::Ticker).await })
        .await
        .unwrap();
    let h = handle.clone();
    run(async move { h.set_mode([nse(1333)], Mode::Quote).await })
        .await
        .unwrap();
    until("three messages", || harness.texts(0).len() == 3).await;
    let one = vec!["1333".to_owned()];
    assert_eq!(
        requests(&harness, 0),
        [(15, one.clone()), (16, one.clone()), (17, one)]
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn replace_sends_the_minimal_diff() {
    let (harness, handle, _log) = live(vec![]).await;
    let h = handle.clone();
    run(async move { h.subscribe([nse(1), nse(2)], Mode::Ticker).await })
        .await
        .unwrap();
    let h = handle.clone();
    run(async move {
        h.replace([(nse(2), Mode::Ticker), (nse(3), Mode::Full)])
            .await
    })
    .await
    .unwrap();
    until("three messages", || harness.texts(0).len() == 3).await;
    // Instrument 2 is unchanged and never re-sent.
    assert_eq!(
        requests(&harness, 0),
        [
            (15, vec!["1".to_owned(), "2".to_owned()]),
            (16, vec!["1".to_owned()]),
            (21, vec!["3".to_owned()])
        ]
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_unchanged_subscribe_keeps_the_revision_and_sends_nothing() {
    let (harness, handle, _log) = live(vec![]).await;
    let h = handle.clone();
    let first = run(async move { h.subscribe([nse(1333)], Mode::Full).await })
        .await
        .unwrap();
    until("one message", || harness.texts(0).len() == 1).await;
    let h = handle.clone();
    let again = run(async move { h.subscribe([nse(1333)], Mode::Full).await })
        .await
        .unwrap();
    // Give the owner every chance to write something.
    let end = tokio::time::Instant::now() + Duration::from_secs(1);
    until("a second passed", || tokio::time::Instant::now() >= end).await;
    assert_eq!(first, again);
    assert_eq!(harness.texts(0).len(), 1);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_five_thousand_and_first_instrument_is_refused_without_a_frame() {
    let (capture, _guard) = support::trace::install();
    let (harness, handle, _log) = live(vec![]).await;
    let h = handle.clone();
    run(async move { h.subscribe((1..=5000).map(nse), Mode::Ticker).await })
        .await
        .unwrap();
    until("fifty messages", || harness.texts(0).len() == 50).await;
    let h = handle.clone();
    let refused = run(async move { h.subscribe([nse(5001)], Mode::Ticker).await }).await;
    assert_eq!(
        refused,
        Err(CommandError::Invalid(SubscriptionError::CapacityExceeded {
            max: 5000
        }))
    );
    let end = tokio::time::Instant::now() + Duration::from_secs(1);
    until("a second passed", || tokio::time::Instant::now() >= end).await;
    assert_eq!(harness.texts(0).len(), 50);
    let rejected = capture.events_named("ws.subscription.rejected");
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0].field("feed"), Some("market"));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_command_during_backoff_is_written_by_the_next_restore() {
    let subscribe = serde_json::json!({
        "RequestCode": 15,
        "InstrumentCount": 1,
        "InstrumentList": [{"ExchangeSegment": "NSE_EQ", "SecurityId": "1333"}]
    });
    let harness = WsHarness::start(vec![
        WsConnection::accept(vec![Step::Eof]),
        WsConnection::accept(vec![Step::ExpectText(subscribe)]),
    ])
    .await;
    let (handle, events, _task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .limits(limits())
        .spawn()
        .unwrap();
    let log = collect(events);
    until("backoff", || {
        log.lock()
            .unwrap()
            .iter()
            .any(|l| matches!(l, Lifecycle::Backoff { .. }))
    })
    .await;
    let h = handle.clone();
    let revision = run(async move { h.subscribe([nse(1333)], Mode::Ticker).await })
        .await
        .unwrap();
    // The reply came while still in Backoff, before the second connection attempt.
    assert!(
        !log.lock()
            .unwrap()
            .iter()
            .any(|l| matches!(l, Lifecycle::Connecting { epoch: 2, .. }))
    );
    until("second active", || active(&log, 2)).await;
    until("restore written", || harness.texts(1).len() == 1).await;
    assert!(
        harness.mismatches().is_empty(),
        "{:?}",
        harness.mismatches()
    );
    let entries = log.lock().unwrap().clone();
    let sent = entries
        .iter()
        .position(|l| matches!(l, Lifecycle::CommandsSent { revision: r } if *r == revision))
        .expect("CommandsSent after the restore");
    let second_active = entries
        .iter()
        .position(|l| matches!(l, Lifecycle::Active { epoch: 2, .. }))
        .unwrap();
    assert_eq!(sent + 1, second_active);
    let commands_sent = entries
        .iter()
        .filter(|l| matches!(l, Lifecycle::CommandsSent { .. }));
    assert_eq!(commands_sent.count(), 1);
    assert!(harness.texts(0).is_empty());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_peer_close_reconnects_with_its_close_code() {
    let harness = WsHarness::start(vec![
        WsConnection::accept(vec![Step::Close(Some(1001))]),
        WsConnection::accept(vec![]),
    ])
    .await;
    let (_handle, events, _task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .limits(limits())
        .spawn()
        .unwrap();
    let log = collect(events);
    until("second active", || active(&log, 2)).await;
    assert!(log.lock().unwrap().iter().any(|l| matches!(
        l,
        Lifecycle::Disconnected {
            epoch: 1,
            reason: DisconnectReason::RemoteClose { code: Some(1001) }
        }
    )));
    // The restore wrote nothing new, so it reports no CommandsSent.
    assert!(
        !log.lock()
            .unwrap()
            .iter()
            .any(|l| matches!(l, Lifecycle::CommandsSent { .. }))
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn commands_queued_together_are_written_by_one_reconcile() {
    let (harness, handle, log) = live(vec![]).await;
    // One poll of the join queues all three before the owner next runs.
    let h = handle.clone();
    let (a, b, c) = run(async move {
        tokio::join!(
            h.subscribe([nse(1)], Mode::Ticker),
            h.subscribe([nse(2)], Mode::Ticker),
            h.subscribe([nse(3)], Mode::Ticker)
        )
    })
    .await;
    assert_eq!(
        [a.unwrap(), b.unwrap(), c.unwrap()],
        [Revision(1), Revision(2), Revision(3)]
    );
    let end = tokio::time::Instant::now() + Duration::from_secs(1);
    until("a second passed", || tokio::time::Instant::now() >= end).await;
    assert_eq!(
        requests(&harness, 0),
        [(15, vec!["1".to_owned(), "2".to_owned(), "3".to_owned()])]
    );
    let sent: Vec<_> = log
        .lock()
        .unwrap()
        .iter()
        .filter_map(|l| match l {
            Lifecycle::CommandsSent { revision } => Some(*revision),
            _ => None,
        })
        .collect();
    assert_eq!(sent, [Revision(3)]);
}
