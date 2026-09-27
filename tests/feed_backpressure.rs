//! Backpressure scenarios against the loopback WebSocket harness: a stalled consumer under
//! `OverflowPolicy::Fail` and `OverflowPolicy::DropOldest`, a full lifecycle queue, and a dropped
//! event stream.
//!
//! The consumer stalls simply by not polling `FeedEvents`; progress is observed through
//! `FeedHandle::status`, which never waits on the queues. As in `feed_lifecycle.rs`, a paused
//! clock is driven by spinning on `yield_now` and advancing time only in 10 ms steps.

mod support;

use std::time::Duration;

use dhani::decoder::MarketPacket;
use dhani::feed::{
    FeedError, FeedEvent, FeedEvents, FeedHandle, FeedLimits, FeedState, Lifecycle, MarketFeed,
    MarketSub, OverflowPolicy, TerminalReason,
};
use dhani::{AccessToken, ClientId, Credentials};
use futures_util::{FutureExt, StreamExt};
use support::encode;
use support::ws::{Step, WsConnection, WsHarness};

fn credentials() -> Credentials {
    Credentials::new(
        ClientId::new("9999888877").unwrap(),
        AccessToken::new("eyJhbGciOiJIUzUxMiJ9.eyJzdWIiOiI5OTk5ODg4ODc3In0.U0VOVElORUwtU0lH")
            .unwrap(),
    )
}

/// The smallest data queue, a fixed jitter seed and the given lifecycle capacity.
fn limits(lifecycle_capacity: usize) -> FeedLimits {
    let mut limits = FeedLimits::default();
    limits.queue_capacity = 64;
    limits.lifecycle_capacity = lifecycle_capacity;
    limits.reconnect.jitter_seed = Some(7);
    limits
}

/// One frame of `n` Ticker packets for NSE_EQ 1333 with `ltt` 1..=n.
fn burst(n: u32) -> Vec<u8> {
    let at = encode::Instrument {
        segment_code: 1,
        security_id: 1333,
    };
    let packets: Vec<Vec<u8>> = (1..=n).map(|i| encode::ticker(at, 1642.5, i)).collect();
    encode::frame(&packets)
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

/// What the consumer read, reduced to comparable labels: `ltt@seq` for Ticker data.
#[derive(Debug, PartialEq)]
enum Seen {
    Life(Lifecycle),
    Tick { ltt: u32, seq: u64 },
    Failed(FeedError),
}

/// Reads every item already queued, up to the end of the stream, without waiting.
fn drain(events: &mut FeedEvents<MarketPacket>) -> Vec<Seen> {
    let mut seen = Vec::new();
    while let Some(item) = events.next().now_or_never() {
        match item {
            Some(Ok(FeedEvent::Lifecycle(l))) => seen.push(Seen::Life(l)),
            Some(Ok(FeedEvent::Data(d))) => match d.value {
                MarketPacket::Ticker(t) => seen.push(Seen::Tick {
                    ltt: t.ltt,
                    seq: d.seq,
                }),
                other => panic!("unexpected packet {other:?}"),
            },
            Some(Ok(other)) => panic!("unexpected event {other:?}"),
            Some(Err(e)) => seen.push(Seen::Failed(e)),
            None => break,
        }
    }
    seen
}

fn terminal(handle: &FeedHandle<MarketSub>) -> Option<TerminalReason> {
    handle.status().terminal
}

/// The lifecycle events of `seen`, as short labels.
fn labels(seen: &[Seen]) -> Vec<String> {
    seen.iter()
        .filter_map(|s| match s {
            Seen::Life(l) => Some(match l {
                Lifecycle::Connecting { epoch, .. } => format!("Connecting{{{epoch}}}"),
                Lifecycle::Connected { epoch } => format!("Connected{{{epoch}}}"),
                Lifecycle::Active { epoch, .. } => format!("Active{{{epoch}}}"),
                Lifecycle::Disconnected { reason, .. } => format!("Disconnected{{{reason:?}}}"),
                Lifecycle::Backoff { .. } => "Backoff".to_owned(),
                Lifecycle::Gap { previous_epoch, .. } => format!("Gap{{{previous_epoch}}}"),
                Lifecycle::Lagged { dropped } => format!("Lagged{{{dropped}}}"),
                other => format!("{other:?}"),
            }),
            Seen::Tick { .. } => None,
            Seen::Failed(e) => Some(format!("Failed{{{:?}}}", e.0)),
        })
        .collect()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn fail_ends_with_delivery_overload_after_delivery_wait() {
    let (capture, _guard) = support::trace::install();
    let harness = WsHarness::start(vec![WsConnection::accept(vec![
        Step::Wait(Duration::from_secs(1)),
        Step::SendBinary(burst(100)),
    ])])
    .await;
    let (handle, mut events, _task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .limits(limits(256))
        .spawn()
        .unwrap();
    until("active", || handle.status().state == FeedState::Active).await;
    // The stall starts when the queue fills; the owner then waits `delivery_wait` (1 s).
    // Full: 64 data items plus the three opening lifecycle items.
    until("queue full", || handle.status().queue_len == 67).await;
    let stalled = tokio::time::Instant::now();
    until("terminal", || terminal(&handle).is_some()).await;
    let waited = tokio::time::Instant::now() - stalled;
    assert!(
        (Duration::from_secs(1)..=Duration::from_millis(1200)).contains(&waited),
        "{waited:?}"
    );
    assert_eq!(terminal(&handle), Some(TerminalReason::DeliveryOverload));
    assert_eq!(handle.status().state, FeedState::Failed);

    // Nothing was dropped: the first 64 ticks, then the terminal error.
    let seen = drain(&mut events);
    let ticks: Vec<u32> = seen
        .iter()
        .filter_map(|s| match s {
            Seen::Tick { ltt, .. } => Some(*ltt),
            _ => None,
        })
        .collect();
    assert_eq!(ticks, (1..=64).collect::<Vec<_>>());
    assert_eq!(
        labels(&seen),
        [
            "Connecting{1}",
            "Connected{1}",
            "Active{1}",
            "Failed{DeliveryOverload}"
        ]
    );
    assert_eq!(events.next().now_or_never(), Some(None));
    assert_eq!(handle.status().dropped_total, 0);
    let overload = capture.events_named("ws.overflow.terminal");
    assert_eq!(overload.len(), 1);
    assert_eq!(
        (overload[0].field("feed"), overload[0].field("queued")),
        (Some("market"), Some("64"))
    );
    // The oldest item waited the whole `delivery_wait`.
    assert_eq!(overload[0].field("oldest_ms"), Some("1000"));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn drop_oldest_reports_lagged_and_keeps_every_lifecycle_event() {
    let (capture, _guard) = support::trace::install();
    let harness = WsHarness::start(vec![
        WsConnection::accept(vec![Step::SendBinary(burst(100)), Step::Close(Some(1001))]),
        WsConnection::accept(vec![]),
    ])
    .await;
    let (handle, mut events, _task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .limits(limits(256))
        .overflow(OverflowPolicy::DropOldest)
        .spawn()
        .unwrap();
    until("second active", || {
        let s = handle.status();
        s.epoch == 2 && s.state == FeedState::Active
    })
    .await;

    let seen = drain(&mut events);
    // 100 ticks into 64 slots: the 36 oldest were dropped, the newest 64 kept in order. The
    // three opening lifecycle items took seq 0..=2, so tick `ltt` has seq `ltt + 2`.
    let ticks: Vec<(u32, u64)> = seen
        .iter()
        .filter_map(|s| match s {
            Seen::Tick { ltt, seq } => Some((*ltt, *seq)),
            _ => None,
        })
        .collect();
    assert_eq!(
        ticks,
        (37..=100)
            .map(|ltt| (ltt, u64::from(ltt) + 2))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        labels(&seen),
        [
            "Lagged{36}",
            "Connecting{1}",
            "Connected{1}",
            "Active{1}",
            "Disconnected{RemoteClose { code: Some(1001) }}",
            "Backoff",
            "Connecting{2}",
            "Connected{2}",
            "Gap{1}",
            "Active{2}"
        ]
    );
    // The merged stream keeps production order: every tick sits between Active{1} and the
    // Disconnected that followed it.
    let position = |label: &str| labels(&seen).iter().position(|l| l == label).unwrap();
    let index_of_life = |n: usize| {
        seen.iter()
            .enumerate()
            .filter(|(_, s)| !matches!(s, Seen::Tick { .. }))
            .nth(n)
            .unwrap()
            .0
    };
    let (active, disconnected) = (
        index_of_life(position("Active{1}")),
        index_of_life(position("Disconnected{RemoteClose { code: Some(1001) }}")),
    );
    let tick_indices: Vec<usize> = seen
        .iter()
        .enumerate()
        .filter(|(_, s)| matches!(s, Seen::Tick { .. }))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        (tick_indices.first(), tick_indices.last()),
        (Some(&(active + 1)), Some(&(disconnected - 1)))
    );

    let lagged: u64 = seen
        .iter()
        .filter_map(|s| match s {
            Seen::Life(Lifecycle::Lagged { dropped }) => Some(*dropped),
            _ => None,
        })
        .sum();
    assert_eq!((lagged, handle.status().dropped_total), (36, 36));
    // Throttled: one event for the whole burst.
    let dropped = capture.events_named("ws.overflow.dropped");
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0].field("feed"), Some("market"));
    assert_eq!(terminal(&handle), None);
}

/// Repeated server closes with a stalled consumer and a 16-item lifecycle queue.
async fn full_lifecycle_queue(policy: OverflowPolicy) {
    let closes = (0..4).map(|_| WsConnection::accept(vec![Step::Close(Some(1001))]));
    let harness = WsHarness::start(closes.collect()).await;
    let (handle, mut events, _task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .limits(limits(16))
        .overflow(policy)
        .spawn()
        .unwrap();
    until("terminal", || terminal(&handle).is_some()).await;
    assert_eq!(terminal(&handle), Some(TerminalReason::DeliveryOverload));
    let seen = drain(&mut events);
    // Opening (3) plus two full reconnects (6 each) fill 15 slots; the third reconnect's
    // Disconnected takes the 16th and its Backoff finds no room.
    let close = "Disconnected{RemoteClose { code: Some(1001) }}";
    let reconnect = |epoch: u64| {
        [
            close.to_owned(),
            "Backoff".to_owned(),
            format!("Connecting{{{epoch}}}"),
            format!("Connected{{{epoch}}}"),
            format!("Gap{{{}}}", epoch - 1),
            format!("Active{{{epoch}}}"),
        ]
    };
    let mut expected: Vec<String> = ["Connecting{1}", "Connected{1}", "Active{1}"]
        .map(str::to_owned)
        .to_vec();
    expected.extend(reconnect(2));
    expected.extend(reconnect(3));
    expected.push(close.to_owned());
    expected.push("Failed{DeliveryOverload}".to_owned());
    assert_eq!(labels(&seen), expected);
    assert_eq!(events.next().now_or_never(), Some(None));
    assert_eq!(harness.connections(), 3);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_full_lifecycle_queue_is_terminal_under_fail() {
    full_lifecycle_queue(OverflowPolicy::Fail).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_full_lifecycle_queue_is_terminal_under_drop_oldest() {
    full_lifecycle_queue(OverflowPolicy::DropOldest).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn dropping_the_event_stream_ends_the_feed() {
    let harness = WsHarness::start(vec![WsConnection::accept(vec![
        Step::Wait(Duration::from_secs(1)),
        Step::SendBinary(burst(1)),
    ])])
    .await;
    let (handle, events, task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .limits(limits(256))
        .spawn()
        .unwrap();
    until("active", || handle.status().state == FeedState::Active).await;
    drop(events);
    until("terminal", || terminal(&handle).is_some()).await;
    assert_eq!(terminal(&handle), Some(TerminalReason::ReceiverDropped));
    // The owner notices at its next push: the tick the harness sends a second after Active.
    let outcome = tokio::spawn(task.join());
    until("task ended", || outcome.is_finished()).await;
    assert_eq!(
        outcome.await.unwrap(),
        dhani::feed::TaskOutcome::Terminal(TerminalReason::ReceiverDropped)
    );
}
