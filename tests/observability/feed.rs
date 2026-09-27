//! WS observability (architecture §6.7 items 2, 4, 5 and 6) against the loopback WebSocket
//! harness. Every test runs on a current-thread runtime so the spawned feed actor records its
//! spans and events on the test thread; a paused clock is driven by spinning on `yield_now` and
//! advancing only in 10 ms steps.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dhani::decoder::MarketPacket;
use dhani::feed::{
    FeedEvent, FeedEvents, FeedHandle, FeedLimits, FeedState, Instrument, MarketFeed, MarketSub,
    Mode, OrderUpdateEvent, OrderUpdateFeed, OverflowPolicy, TerminalReason,
};
use dhani::labels::FeedKind;
use dhani::obs::{EVENT_CATALOGUE, SPAN_CATALOGUE, spans};
use dhani::types::{ExchangeSegment, SecurityId};
use dhani::{AccessToken, ClientId, Credentials};
use futures_util::StreamExt;
use tracing::Level;

use crate::support::encode;
use crate::support::trace::{Capture, install, install_with_max_level};
use crate::support::ws::{Step, WsConnection, WsHarness};

const CLIENT: &str = "9999888877";
const TOKEN: &str = "eyJhbGciOiJIUzUxMiJ9.eyJzdWIiOiI5OTk5ODg4ODc3In0.U0VOVElORUwtU0lH";
const SESSION: &str = "dhani.ws.session";
const CONNECTION: &str = "dhani.ws.connection";
const RESTORE: &str = "dhani.ws.restore";

fn credentials() -> Credentials {
    Credentials::new(
        ClientId::new(CLIENT).unwrap(),
        AccessToken::new(TOKEN).unwrap(),
    )
}

fn limits() -> FeedLimits {
    let mut limits = FeedLimits::default();
    limits.reconnect.jitter_seed = Some(7);
    limits
}

const AT: encode::Instrument = encode::Instrument {
    segment_code: 1,
    security_id: 1333,
};

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..200_000 {
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

/// Reads the stream in the background; counts data items.
fn drain<T: Send + 'static>(mut events: FeedEvents<T>) -> Arc<Mutex<usize>> {
    let seen = Arc::new(Mutex::new(0));
    let sink = Arc::clone(&seen);
    tokio::spawn(async move {
        while let Some(Ok(event)) = events.next().await {
            if matches!(event, FeedEvent::Data(_)) {
                *sink.lock().unwrap() += 1;
            }
        }
    });
    seen
}

fn market(
    harness: &WsHarness,
    limits: FeedLimits,
) -> (FeedHandle<MarketSub>, FeedEvents<MarketPacket>) {
    let (handle, events, _task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .limits(limits)
        .spawn()
        .unwrap();
    (handle, events)
}

fn active(handle: &FeedHandle<MarketSub>, epoch: u64) -> bool {
    let s = handle.status();
    s.epoch == epoch && s.state == FeedState::Active
}

fn events_named(capture: &Capture, name: &str) -> Vec<crate::support::trace::EventRecord> {
    capture.events_named(name)
}

// ---- Catalogue conformance -------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn ws_span_constructors_match_the_catalogue() {
    let (capture, _guard) = install();
    let session = spans::ws_session(FeedKind::Market, 1);
    let connection = session.in_scope(|| spans::ws_connection(FeedKind::Market, 1, 1, "start"));
    let _restore = connection.in_scope(|| spans::ws_restore(1, 1, 10, 1));
    let _command = spans::ws_command("subscribe");
    let _frame = spans::ws_frame(FeedKind::Market, 162);
    let captured = capture.spans();
    let names: Vec<_> = captured.iter().map(|s| s.name).collect();
    assert_eq!(
        names,
        [
            SESSION,
            CONNECTION,
            RESTORE,
            "dhani.ws.command",
            "dhani.ws.frame"
        ]
    );
    for span in &captured {
        let spec = SPAN_CATALOGUE
            .iter()
            .find(|c| c.name == span.name)
            .unwrap_or_else(|| panic!("uncatalogued span {}", span.name));
        assert_eq!(
            (span.level, span.target, span.field_names.as_slice()),
            (spec.level, spec.target, spec.fields),
            "{}",
            span.name
        );
    }
}

// ---- WS scenarios ----------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn session_connection_and_restore_nest_and_a_reconnect_records_its_cause() {
    let (capture, _guard) = install();
    let harness = WsHarness::start(vec![
        WsConnection::accept(vec![Step::Eof]),
        WsConnection::accept(vec![]),
    ])
    .await;
    let (handle, events) = market(&harness, limits());
    let _seen = drain(events);
    until("second active", || active(&handle, 2)).await;

    let sessions = capture.spans_named(SESSION);
    assert_eq!(sessions.len(), 1);
    let connections = capture.spans_named(CONNECTION);
    let first = &connections[0];
    assert_eq!(first.parent, Some(sessions[0].id));
    assert_eq!(
        (first.field("epoch"), first.field("cause")),
        (Some("1"), Some("start"))
    );
    let restores = capture.spans_named(RESTORE);
    assert_eq!(restores[0].parent, Some(first.id));
    let second = connections
        .iter()
        .find(|c| c.field("epoch") == Some("2"))
        .expect("an epoch-2 connection");
    assert_eq!(second.field("cause"), Some("eof"));
    assert_eq!(second.parent, Some(sessions[0].id));
    for name in [
        "ws.connecting",
        "ws.connected",
        "ws.restored",
        "ws.disconnected",
        "ws.reconnect.scheduled",
    ] {
        assert!(!events_named(&capture, name).is_empty(), "{name}");
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_disconnect_805_is_an_error_event_and_terminal() {
    let (capture, _guard) = install();
    let harness = WsHarness::start(vec![WsConnection::accept(vec![Step::SendBinary(
        encode::disconnect(AT, 805),
    )])])
    .await;
    let (handle, events) = market(&harness, limits());
    let _seen = drain(events);
    until("terminal", || handle.status().terminal.is_some()).await;
    let disconnect = events_named(&capture, "ws.server_disconnect");
    assert_eq!(disconnect.len(), 1);
    assert_eq!(disconnect[0].level, Level::ERROR);
    assert_eq!(disconnect[0].target, "dhani::ws");
    let status = handle.status();
    assert_eq!(status.state, FeedState::Failed);
    assert!(matches!(
        status.terminal,
        Some(TerminalReason::ServerDisconnect { .. })
    ));
    assert!(!events_named(&capture, "ws.stopped").is_empty());
}

/// 2000 Ticker packets in 20 frames, delivered and counted.
async fn two_thousand_packets(level: Level) -> Capture {
    let (capture, _guard) = install_with_max_level(level);
    let frame = encode::frame(
        &(0..100)
            .map(|i| encode::ticker(AT, 1.5, i))
            .collect::<Vec<_>>(),
    );
    let steps = (0..20).map(|_| Step::SendBinary(frame.clone())).collect();
    let harness = WsHarness::start(vec![WsConnection::accept(steps)]).await;
    let mut big = limits();
    big.queue_capacity = 4096;
    let (_handle, events) = market(&harness, big);
    let seen = drain(events);
    until("2000 packets", || *seen.lock().unwrap() == 2000).await;
    capture
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn packets_add_no_spans_unless_trace_is_enabled() {
    let debug = two_thousand_packets(Level::DEBUG).await;
    let names: Vec<_> = debug.spans().iter().map(|s| s.name).collect();
    assert_eq!(names, [SESSION, CONNECTION, RESTORE]);

    let trace = two_thousand_packets(Level::TRACE).await;
    let frames = trace.spans_named("dhani.ws.frame").len();
    assert_eq!((trace.spans().len(), frames), (3 + 20, 20));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn overflow_drops_are_reported_once_per_window_with_the_running_total() {
    let (capture, _guard) = install();
    let burst = encode::frame(
        &(0..100)
            .map(|i| encode::ticker(AT, 1.5, i))
            .collect::<Vec<_>>(),
    );
    let harness = WsHarness::start(vec![WsConnection::accept(vec![
        Step::SendBinary(burst.clone()),
        Step::Wait(Duration::from_secs(2)),
        Step::SendBinary(burst),
    ])])
    .await;
    let mut small = limits();
    small.queue_capacity = 64;
    // The stream is never read, so the queue stays full.
    let (handle, _events, _task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .limits(small)
        .overflow(OverflowPolicy::DropOldest)
        .spawn()
        .unwrap();
    until("both bursts", || handle.status().dropped_total == 136).await;
    let dropped = events_named(&capture, "ws.overflow.dropped");
    let totals: Vec<_> = dropped
        .iter()
        .map(|e| e.field("dropped").unwrap().to_owned())
        .collect();
    // The first drop of each one-second window reports the total dropped so far: 1 in the first
    // burst (36 dropped), 37 at the start of the second (100 more).
    assert_eq!(totals, ["1", "37"]);
    assert!(
        dropped
            .iter()
            .all(|e| e.level == Level::WARN && e.field("feed") == Some("market"))
    );
    assert!(
        dropped
            .iter()
            .all(|e| e.field("policy") == Some("drop_oldest")),
        "{dropped:?}"
    );
}

// ---- Every ws.* event ------------------------------------------------------------------------

fn ids(range: std::ops::RangeInclusive<u32>) -> impl Iterator<Item = Instrument> {
    range.map(|n| {
        Instrument::new(
            ExchangeSegment::NseEq,
            SecurityId::new(n.to_string()).unwrap(),
        )
        .unwrap()
    })
}

/// Drives, one after another, the scenarios that emit every catalogued `ws.*` event.
async fn every_ws_scenario() {
    // Connect, restore with a subscription, lose the connection, reconnect; a subscription
    // over capacity; a stray and an unknown packet; a silent server hitting liveness.
    let harness = WsHarness::start(vec![
        WsConnection::accept(vec![
            // Long enough for the subscription commands below to run on epoch 1.
            Step::Wait(Duration::from_secs(1)),
            Step::SendBinary(vec![1, 2, 3]),
            // Decode events are sampled per connection; wait out the window.
            Step::Wait(Duration::from_secs(5)),
            // An undocumented code whose length field overruns the frame: unknown_packet (an
            // undocumented code with a valid length is delivered as MarketPacket::Other).
            Step::SendBinary({
                let mut p = encode::other(99, AT, &[0; 8]);
                p.truncate(p.len() - 4);
                p
            }),
            Step::Eof,
        ]),
        WsConnection::accept(vec![]),
        WsConnection::accept(vec![]),
    ])
    .await;
    let (handle, events) = market(&harness, limits());
    let _seen = drain(events);
    until("active", || active(&handle, 1)).await;
    let h = handle.clone();
    let task = tokio::spawn(async move {
        h.subscribe(ids(1..=5000), Mode::Ticker).await.unwrap();
        h.subscribe(ids(5001..=5001), Mode::Ticker).await
    });
    until("commands", || task.is_finished()).await;
    assert!(task.await.unwrap().is_err());
    until("second active", || active(&handle, 2)).await;
    // Nothing arrives on epoch 2: after the liveness timeout the feed reconnects.
    until("third connection", || handle.status().epoch == 3).await;
    handle.shutdown().await.ok();

    // Every attempt refused: the reconnect budget runs out.
    let harness = WsHarness::start((0..3).map(|_| WsConnection::reject(503)).collect()).await;
    let mut once = limits();
    once.reconnect.max_attempts = 2;
    let (handle, events) = market(&harness, once);
    let _seen = drain(events);
    until("exhausted", || handle.status().terminal.is_some()).await;

    // A burst into a small queue, dropping the oldest, then the same under Fail.
    for policy in [OverflowPolicy::DropOldest, OverflowPolicy::Fail] {
        let burst = encode::frame(
            &(0..100)
                .map(|i| encode::ticker(AT, 1.5, i))
                .collect::<Vec<_>>(),
        );
        let harness =
            WsHarness::start(vec![WsConnection::accept(vec![Step::SendBinary(burst)])]).await;
        let mut small = limits();
        small.queue_capacity = 64;
        let (handle, _events) = MarketFeed::builder(credentials())
            .url(harness.url())
            .limits(small)
            .overflow(policy)
            .spawn()
            .map(|(h, e, _)| (h, e))
            .unwrap();
        until("burst queued", || {
            handle.status().queue_len >= 64 || handle.status().terminal.is_some()
        })
        .await;
        if policy == OverflowPolicy::Fail {
            until("overload", || handle.status().terminal.is_some()).await;
        }
    }

    // A server disconnect packet.
    let harness = WsHarness::start(vec![WsConnection::accept(vec![Step::SendBinary(
        encode::disconnect(AT, 805),
    )])])
    .await;
    let (handle, events) = market(&harness, limits());
    let _seen = drain(events);
    until("server disconnect", || handle.status().terminal.is_some()).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn every_catalogued_ws_event_is_emitted() {
    let (capture, _guard) = install();
    every_ws_scenario().await;
    let seen: Vec<String> = capture
        .dhani_events()
        .iter()
        .map(|e| e.name().to_owned())
        .collect();
    let missing: Vec<&str> = EVENT_CATALOGUE
        .iter()
        .filter(|e| e.name.starts_with("ws."))
        .map(|e| e.name)
        .filter(|name| !seen.iter().any(|s| s == name))
        .collect();
    assert!(
        missing.is_empty(),
        "never emitted: {missing:?}; seen {seen:?}"
    );
}

// ---- Redaction sentinel sweep ----------------------------------------------------------------

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn no_sentinel_reaches_ws_spans_events_status_or_errors() {
    let (capture, _guard) = install();

    // A handshake rejection: the market feed URL carries the client ID and token.
    let harness = WsHarness::start(vec![WsConnection::reject(401)]).await;
    let (handle, events, task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .limits(limits())
        .spawn()
        .unwrap();
    let _seen = drain(events);
    until("rejected", || handle.status().terminal.is_some()).await;
    let status = format!("{:?}", handle.status());
    let outcome = tokio::spawn(task.join());
    until("task ended", || outcome.is_finished()).await;
    let outcome = format!("{:?}", outcome.await.unwrap());
    assert_eq!(events_named(&capture, "ws.handshake.rejected").len(), 1);

    // An order-update message that is not an order and echoes the client ID.
    let echo = format!(r#"{{"Type":"login_ack","Data":{{"clientId":"{CLIENT}"}}}}"#);
    let harness = WsHarness::start(vec![WsConnection::accept(vec![
        Step::ExpectText(serde_json::json!({
            "LoginReq": {"MsgCode": 42, "ClientId": CLIENT, "Token": TOKEN},
            "UserType": "SELF"
        })),
        Step::SendText(echo),
    ])])
    .await;
    let (updates, mut update_events, _task) = OrderUpdateFeed::builder(credentials())
        .url(harness.url())
        .spawn()
        .unwrap();
    let got = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&got);
    tokio::spawn(async move {
        while let Some(Ok(event)) = update_events.next().await {
            if let FeedEvent::Data(d) = event
                && matches!(d.value, OrderUpdateEvent::Other { .. })
            {
                *sink.lock().unwrap() = Some(());
            }
        }
    });
    until("unknown message", || got.lock().unwrap().is_some()).await;
    let update_status = format!("{:?}", updates.status());

    let mut rendered = vec![status, outcome, update_status];
    for span in capture.spans() {
        rendered.extend(span.fields.values().cloned());
    }
    for event in capture.events() {
        rendered.extend(event.fields.values().cloned());
    }
    for text in &rendered {
        for sentinel in [CLIENT, TOKEN, "eyJ"] {
            assert!(!text.contains(sentinel), "{sentinel} in {text}");
        }
    }
}

// ---- Metrics ---------------------------------------------------------------------------------

#[cfg(feature = "metrics")]
mod metrics {
    use metrics_util::debugging::DebuggingRecorder;

    use super::*;
    use dhani::obs::metrics as names;

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn every_ws_metric_is_recorded_including_inside_the_actor() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _guard = ::metrics::set_default_local_recorder(&recorder);
        every_ws_scenario().await;
        // Frames, bytes and packets.
        let _capture = two_thousand_packets(Level::DEBUG).await;
        let seen: Vec<String> = snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .map(|(key, _, _, _)| key.key().name().to_owned())
            .collect();
        for name in [
            names::WS_CONNECTIONS_ACTIVE,
            names::WS_CONNECT_ATTEMPTS_TOTAL,
            names::WS_RECONNECTS_TOTAL,
            names::WS_SERVER_DISCONNECTS_TOTAL,
            names::WS_FRAMES_TOTAL,
            names::WS_BYTES_TOTAL,
            names::WS_PACKETS_TOTAL,
            names::WS_DECODE_FAILURES_TOTAL,
            names::WS_QUEUE_DEPTH,
            names::WS_DROPPED_TOTAL,
        ] {
            assert!(
                seen.iter().any(|s| s == name),
                "{name} never recorded: {seen:?}"
            );
        }
    }
}
