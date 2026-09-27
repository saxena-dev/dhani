//! Feed lifecycle scenarios against the scripted loopback WebSocket harness: the handshake
//! target, reconnect and restore, terminal server disconnects and handshake refusals, liveness,
//! and clean shutdown.
//!
//! Every test runs on a current-thread runtime with a paused clock. The driver spins on
//! `yield_now` (so the runtime never parks and the clock cannot auto-advance while a loopback
//! socket is busy) and advances time in small steps only when nothing else can make progress.

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dhani::decoder::MarketPacket;
use dhani::feed::{
    DisconnectReason, FeedError, FeedEvent, FeedLimits, FeedStatus, Instrument, Lifecycle,
    MarketFeed, Mode, TerminalReason,
};
use dhani::types::{ExchangeSegment, SecurityId};
use dhani::{AccessToken, ClientId, Credentials};
use futures_util::{FutureExt, StreamExt};
use support::encode;
use support::ws::{ClientFrame, Step, WsConnection, WsHarness};

const CLIENT_ID: &str = "9999888877";
const TOKEN: &str = "eyJhbGciOiJIUzUxMiJ9.eyJzdWIiOiI5OTk5ODg4ODc3In0.U0VOVElORUwtU0lH";

fn credentials() -> Credentials {
    Credentials::new(
        ClientId::new(CLIENT_ID).unwrap(),
        AccessToken::new(TOKEN).unwrap(),
    )
}

fn instrument() -> Instrument {
    Instrument::new(ExchangeSegment::NseEq, SecurityId::new("1333").unwrap()).unwrap()
}

fn subscribe_ticker() -> serde_json::Value {
    serde_json::json!({
        "RequestCode": 15,
        "InstrumentCount": 1,
        "InstrumentList": [{"ExchangeSegment": "NSE_EQ", "SecurityId": "1333"}]
    })
}

fn limits() -> FeedLimits {
    let mut limits = FeedLimits::default();
    limits.reconnect.jitter_seed = Some(7);
    limits
}

/// What the collector saw.
#[derive(Debug)]
enum Seen {
    Life(Lifecycle),
    Data(MarketPacket),
    Other,
    Failed(FeedError),
    End,
}

type Log = Arc<Mutex<Vec<Seen>>>;

/// Reads the whole event stream into a shared log.
fn collect(mut events: dhani::feed::FeedEvents<MarketPacket>) -> Log {
    let log: Log = Arc::default();
    let sink = Arc::clone(&log);
    tokio::spawn(async move {
        loop {
            let seen = match events.next().await {
                Some(Ok(FeedEvent::Lifecycle(l))) => Seen::Life(l),
                Some(Ok(FeedEvent::Data(d))) => Seen::Data(d.value),
                Some(Ok(_)) => Seen::Other,
                Some(Err(e)) => Seen::Failed(e),
                None => {
                    sink.lock().unwrap().push(Seen::End);
                    return;
                }
            };
            sink.lock().unwrap().push(seen);
        }
    });
    log
}

/// The lifecycle events so far, as short labels.
fn lifecycle(log: &Log) -> Vec<String> {
    log.lock()
        .unwrap()
        .iter()
        .filter_map(|s| match s {
            Seen::Life(l) => Some(match l {
                Lifecycle::Connecting { epoch, .. } => format!("Connecting{{{epoch}}}"),
                Lifecycle::Connected { epoch } => format!("Connected{{{epoch}}}"),
                Lifecycle::Active { epoch, .. } => format!("Active{{{epoch}}}"),
                Lifecycle::Disconnected { reason, .. } => format!("Disconnected{{{reason:?}}}"),
                Lifecycle::Backoff { .. } => "Backoff".to_owned(),
                Lifecycle::Gap { previous_epoch, .. } => format!("Gap{{{previous_epoch}}}"),
                Lifecycle::ServerDisconnect { code, .. } => format!("ServerDisconnect{{{code}}}"),
                Lifecycle::CommandsSent { .. } => "CommandsSent".to_owned(),
                Lifecycle::Stopped => "Stopped".to_owned(),
                other => format!("{other:?}"),
            }),
            _ => None,
        })
        .collect()
}

fn ended(log: &Log) -> bool {
    matches!(log.lock().unwrap().last(), Some(Seen::End))
}

/// Runs the runtime until `done` holds: spins without advancing time, then advances 10 ms when
/// nothing is left to do.
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

/// Lets simulated `duration` pass while driving the runtime.
async fn pass(duration: Duration) {
    let end = tokio::time::Instant::now() + duration;
    until("time passed", || tokio::time::Instant::now() >= end).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_handshake_target_carries_the_credentials() {
    let harness = WsHarness::start(vec![WsConnection::accept(vec![])]).await;
    let (handle, events, _task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .limits(limits())
        .spawn()
        .unwrap();
    let log = collect(events);
    until("active", || {
        lifecycle(&log).contains(&"Active{1}".to_owned())
    })
    .await;
    assert_eq!(
        harness.targets(),
        [format!(
            "/?version=2&token={TOKEN}&clientId={CLIENT_ID}&authType=2"
        )]
    );
    handle.shutdown().now_or_never();
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_reconnect_after_eof_restores_the_set_and_reports_the_gap() {
    let ticker = encode::ticker(
        encode::Instrument {
            segment_code: 1,
            security_id: 1333,
        },
        1642.5,
        1_726_048_169,
    );
    let harness = WsHarness::start(vec![
        WsConnection::accept(vec![
            Step::ExpectText(subscribe_ticker()),
            Step::SendBinary(ticker),
            Step::Eof,
        ]),
        WsConnection::accept(vec![Step::ExpectText(subscribe_ticker())]),
    ])
    .await;
    let (handle, events, _task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .limits(limits())
        .spawn()
        .unwrap();
    // Queued before the owner first runs, so it is part of the first restore.
    let _ = handle
        .subscribe([instrument()], Mode::Ticker)
        .now_or_never();
    let log = collect(events);
    until("second active", || {
        lifecycle(&log).contains(&"Active{2}".to_owned())
    })
    .await;
    until("second restore", || harness.texts(1).len() == 1).await;
    assert_eq!(
        lifecycle(&log),
        [
            "Connecting{1}",
            "Connected{1}",
            "Active{1}",
            "Disconnected{Eof}",
            "Backoff",
            "Connecting{2}",
            "Connected{2}",
            "Gap{1}",
            "Active{2}"
        ]
    );
    assert!(
        log.lock()
            .unwrap()
            .iter()
            .any(|s| matches!(s, Seen::Data(MarketPacket::Ticker(t)) if t.ltp == 1642.5))
    );
    // The full set was written on both connections, before Active.
    assert_eq!(harness.texts(0), [subscribe_ticker()]);
    assert_eq!(harness.texts(1), [subscribe_ticker()]);
    assert!(
        harness.mismatches().is_empty(),
        "{:?}",
        harness.mismatches()
    );
    let status: FeedStatus = handle.status();
    assert_eq!((status.epoch, status.last_failures.len()), (2, 1));
    assert_eq!(status.last_failures[0].reason, DisconnectReason::Eof);
    let gap_reason = log.lock().unwrap().iter().find_map(|s| match s {
        Seen::Life(Lifecycle::Gap { reason, .. }) => Some(reason.clone()),
        _ => None,
    });
    assert_eq!(gap_reason, Some(DisconnectReason::Eof));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_disconnect_805_is_terminal_without_a_reconnect() {
    let disconnect = encode::disconnect(
        encode::Instrument {
            segment_code: 1,
            security_id: 0,
        },
        805,
    );
    let harness = WsHarness::start(vec![
        WsConnection::accept(vec![Step::SendBinary(disconnect)]),
        WsConnection::accept(vec![]),
    ])
    .await;
    let (_handle, events, task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .limits(limits())
        .spawn()
        .unwrap();
    let log = collect(events);
    until("stream end", || ended(&log)).await;
    pass(Duration::from_secs(5)).await;
    assert_eq!(harness.connections(), 1);
    let life = lifecycle(&log);
    assert_eq!(
        &life[life.len() - 2..],
        [
            "ServerDisconnect{805}",
            "Disconnected{ServerDisconnect { code: Some(805) }}"
        ]
    );
    {
        let entries = log.lock().unwrap();
        assert!(matches!(
            &entries[entries.len() - 2],
            Seen::Failed(FeedError(TerminalReason::ServerDisconnect { code: 805 }))
        ));
    }
    assert_eq!(
        task.join().await,
        dhani::feed::TaskOutcome::Terminal(TerminalReason::ServerDisconnect { code: 805 })
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_handshake_401_is_terminal() {
    let harness = WsHarness::start(vec![
        WsConnection::reject(401),
        WsConnection::accept(vec![]),
    ])
    .await;
    let (_handle, events, _task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .limits(limits())
        .spawn()
        .unwrap();
    let log = collect(events);
    until("stream end", || ended(&log)).await;
    pass(Duration::from_secs(5)).await;
    assert_eq!(harness.connections(), 1);
    let entries = log.lock().unwrap();
    assert!(matches!(
        &entries[entries.len() - 2],
        Seen::Failed(FeedError(TerminalReason::AuthRejected { http_status: 401 }))
    ));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_handshake_429_is_retried() {
    let harness = WsHarness::start(vec![
        WsConnection::reject(429),
        WsConnection::accept(vec![]),
    ])
    .await;
    let (_handle, events, _task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .limits(limits())
        .spawn()
        .unwrap();
    let log = collect(events);
    until("second connection", || {
        lifecycle(&log).contains(&"Connected{2}".to_owned())
    })
    .await;
    assert_eq!(harness.connections(), 2);
    assert_eq!(
        &lifecycle(&log)[..3],
        ["Connecting{1}", "Backoff", "Connecting{2}"]
    );
    assert_eq!(_handle.status().last_failures[0].http_status, Some(429));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn thirty_seconds_of_silence_reconnect_with_liveness_timeout() {
    let harness = WsHarness::start(vec![
        WsConnection::accept(vec![]),
        WsConnection::accept(vec![]),
    ])
    .await;
    let (handle, events, _task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .limits(limits())
        .spawn()
        .unwrap();
    let log = collect(events);
    until("active", || {
        lifecycle(&log).contains(&"Active{1}".to_owned())
    })
    .await;
    let started = tokio::time::Instant::now();
    until("reconnected", || {
        lifecycle(&log).contains(&"Connected{2}".to_owned())
    })
    .await;
    assert!(
        started.elapsed() >= Duration::from_secs(30),
        "{:?}",
        started.elapsed()
    );
    assert!(lifecycle(&log).contains(&"Disconnected{LivenessTimeout}".to_owned()));
    assert_eq!(
        handle.status().last_failures[0].reason,
        DisconnectReason::LivenessTimeout
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn shutdown_sends_the_disconnect_request_then_a_close() {
    let harness = WsHarness::start(vec![WsConnection::accept(vec![])]).await;
    let (handle, events, task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .limits(limits())
        .spawn()
        .unwrap();
    let log = collect(events);
    until("active", || {
        lifecycle(&log).contains(&"Active{1}".to_owned())
    })
    .await;
    let stopping = tokio::spawn({
        let handle = handle.clone();
        async move { handle.shutdown().await }
    });
    until("stream end", || ended(&log)).await;
    assert_eq!(stopping.await.unwrap(), Ok(()));
    let frames = harness.frames(0);
    assert_eq!(
        frames[0],
        ClientFrame::Text(r#"{"RequestCode":12}"#.to_owned())
    );
    assert!(matches!(frames[1], ClientFrame::Close(_)), "{frames:?}");
    let life = lifecycle(&log);
    assert_eq!(
        &life[life.len() - 2..],
        ["Disconnected{Shutdown}", "Stopped"]
    );
    // A clean end: no error before the end of the stream.
    {
        let entries = log.lock().unwrap();
        assert!(!entries.iter().any(|s| matches!(s, Seen::Failed(_))));
    }
    assert_eq!(task.join().await, dhani::feed::TaskOutcome::Clean);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_handshake_503_is_retried() {
    let harness = WsHarness::start(vec![
        WsConnection::reject(503),
        WsConnection::accept(vec![]),
    ])
    .await;
    let (_handle, events, _task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .limits(limits())
        .spawn()
        .unwrap();
    let log = collect(events);
    until("second connection", || {
        lifecycle(&log).contains(&"Connected{2}".to_owned())
    })
    .await;
    assert_eq!(harness.connections(), 2);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn dropping_every_handle_ends_a_live_feed() {
    let harness = WsHarness::start(vec![WsConnection::accept(vec![])]).await;
    let (handle, events, task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .limits(limits())
        .spawn()
        .unwrap();
    let log = collect(events);
    until("active", || {
        lifecycle(&log).contains(&"Active{1}".to_owned())
    })
    .await;
    drop(handle);
    until("stream end", || ended(&log)).await;
    assert_eq!(
        task.join().await,
        dhani::feed::TaskOutcome::Terminal(TerminalReason::HandlesDropped)
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn raw_capture_delivers_each_frame_before_its_packets() {
    let at = encode::Instrument {
        segment_code: 1,
        security_id: 1333,
    };
    let frame = encode::frame(&[encode::ticker(at, 1.5, 1), encode::ticker(at, 2.5, 2)]);
    let harness = WsHarness::start(vec![WsConnection::accept(vec![Step::SendBinary(frame)])]).await;
    let (_handle, mut events, _task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .limits(limits())
        .capture_raw(true)
        .spawn()
        .unwrap();
    let collected = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&collected);
    tokio::spawn(async move {
        while let Some(Ok(event)) = events.next().await {
            let label = match event {
                FeedEvent::Raw(raw) => Some(format!("raw{}@{}", raw.bytes.len(), raw.seq)),
                FeedEvent::Data(d) => Some(format!("data@{}", d.seq)),
                _ => None,
            };
            if let Some(label) = label {
                sink.lock().unwrap().push(label);
            }
        }
    });
    until("two packets", || collected.lock().unwrap().len() == 3).await;
    // The raw frame comes first and takes the seq before its two packets (after the three
    // lifecycle items Connecting, Connected and Active).
    assert_eq!(*collected.lock().unwrap(), ["raw32@3", "data@4", "data@5"]);
}
