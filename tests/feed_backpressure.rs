//! Backpressure scenarios against the loopback WebSocket harness: a stalled consumer under
//! `OverflowPolicy::Fail` and `OverflowPolicy::DropOldest`, a full lifecycle queue, a dropped
//! event stream, and socket writes stalled by a server that stops reading.
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

fn nse(id: u32) -> dhani::feed::Instrument {
    dhani::feed::Instrument::new(
        dhani::types::ExchangeSegment::NseEq,
        dhani::types::SecurityId::new(id.to_string()).unwrap(),
    )
    .unwrap()
}

/// A server that stops reading at once but pings every 5 s for a minute: an idle client would
/// never hit its liveness timeout, so one can only come from a stalled write.
fn stalling_connection() -> WsConnection {
    let mut steps = vec![Step::StopReading];
    for _ in 0..12 {
        steps.push(Step::Wait(Duration::from_secs(5)));
        steps.push(Step::Ping);
    }
    WsConnection::accept(steps)
}

/// A feed stuck in a socket write, with the time the command whose write stalled was issued.
struct Stalled {
    harness: WsHarness,
    handle: FeedHandle<MarketSub>,
    events: FeedEvents<MarketPacket>,
    task: dhani::feed::FeedTask,
    applied_at: tokio::time::Instant,
}

/// Starts a feed on a harness with a 4 KiB receive buffer, then swaps between two
/// 5000-instrument sets (about 100 messages per swap) until a command gets no reply within a
/// second: the owner is then stuck writing the previous swap.
async fn stalled(second: Vec<WsConnection>) -> Stalled {
    let mut connections = vec![stalling_connection()];
    connections.extend(second);
    let harness = WsHarness::start_with_recv_buffer(connections, 4096).await;
    let (handle, events, task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .limits(limits(256))
        .spawn()
        .unwrap();
    until("active", || handle.status().state == FeedState::Active).await;
    let set = |from: u32| {
        (from..from + 5000)
            .map(|id| (nse(id), dhani::feed::Mode::Full))
            .collect::<Vec<_>>()
    };
    let mut applied_at = tokio::time::Instant::now();
    for round in 0..20 {
        // The stuck write, if this round's, starts after this point.
        let issued_at = tokio::time::Instant::now();
        let h = handle.clone();
        let pairs = set(if round % 2 == 0 { 1 } else { 10_001 });
        let reply = tokio::spawn(async move { h.replace(pairs).await });
        let give_up = tokio::time::Instant::now() + Duration::from_secs(1);
        until("reply or stall", || {
            reply.is_finished() || tokio::time::Instant::now() >= give_up
        })
        .await;
        if !reply.is_finished() {
            // Virtual seconds pass in microseconds of real time: give a write that is only
            // briefly blocked real time to finish before calling it stuck.
            // real-time: 200 ms of spinning without advancing the clock.
            let real = std::time::Instant::now() + Duration::from_millis(200);
            while !reply.is_finished() && std::time::Instant::now() < real {
                tokio::task::yield_now().await;
            }
        }
        if !reply.is_finished() {
            return Stalled {
                harness,
                handle,
                events,
                task,
                applied_at,
            };
        }
        applied_at = issued_at;
    }
    panic!("the writes never stalled");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_stalled_write_ends_the_connection_as_a_liveness_loss() {
    let mut feed = stalled(vec![WsConnection::accept(vec![])]).await;
    until("connection lost", || {
        !feed.handle.status().last_failures.is_empty()
    })
    .await;
    // The write that stalled started after the last applied command and is bounded by the 30 s
    // liveness timeout; an idle connection would have lived on the server's pings.
    let lost_after = tokio::time::Instant::now() - feed.applied_at;
    assert!(
        (Duration::from_secs(30)..=Duration::from_secs(33)).contains(&lost_after),
        "{lost_after:?}"
    );
    let lost = feed.handle.status().last_failures[0].clone();
    assert_eq!(lost.reason, dhani::feed::DisconnectReason::LivenessTimeout);
    until("second active", || {
        let s = feed.handle.status();
        s.epoch == 2 && s.state == FeedState::Active
    })
    .await;
    let seen = labels(&drain(&mut feed.events));
    assert!(
        seen.contains(&"Disconnected{LivenessTimeout}".to_owned()),
        "{seen:?}"
    );
    assert_eq!(feed.harness.connections(), 2);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_stop_during_a_stalled_write_ends_with_send_interrupted() {
    let feed = stalled(vec![]).await;
    let status = feed.handle.status();
    assert_eq!((status.epoch, status.state), (1, FeedState::Active));
    let h = feed.handle.clone();
    let asked = tokio::time::Instant::now();
    let shutdown = tokio::spawn(async move { h.shutdown().await });
    until("shutdown", || shutdown.is_finished()).await;
    // The stop cuts the write at once, well inside the 5 s shutdown_timeout.
    assert!(tokio::time::Instant::now() - asked < Duration::from_millis(100));
    assert_eq!(
        shutdown.await.unwrap(),
        Err(FeedError(TerminalReason::SendInterrupted))
    );
    let outcome = tokio::spawn(feed.task.join());
    until("task ended", || outcome.is_finished()).await;
    assert_eq!(
        outcome.await.unwrap(),
        dhani::feed::TaskOutcome::Terminal(TerminalReason::SendInterrupted)
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn dropping_the_stream_of_a_quiet_feed_ends_it_at_the_next_ping() {
    // The server only pings: nothing is ever pushed to the dropped stream.
    let pings = (0..10)
        .flat_map(|_| [Step::Wait(Duration::from_secs(1)), Step::Ping])
        .collect();
    let harness = WsHarness::start(vec![WsConnection::accept(pings)]).await;
    let (handle, events, task) = MarketFeed::builder(credentials())
        .url(harness.url())
        .limits(limits(256))
        .spawn()
        .unwrap();
    until("active", || handle.status().state == FeedState::Active).await;
    drop(events);
    let dropped_at = tokio::time::Instant::now();
    until("terminal", || terminal(&handle).is_some()).await;
    assert_eq!(terminal(&handle), Some(TerminalReason::ReceiverDropped));
    // Noticed at the first ping, a second later, not at the liveness timeout.
    assert!(tokio::time::Instant::now() - dropped_at <= Duration::from_millis(1100));
    let outcome = tokio::spawn(task.join());
    until("task ended", || outcome.is_finished()).await;
    assert_eq!(
        outcome.await.unwrap(),
        dhani::feed::TaskOutcome::Terminal(TerminalReason::ReceiverDropped)
    );
}
