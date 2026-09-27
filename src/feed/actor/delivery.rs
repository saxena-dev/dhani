//! Two-queue event delivery and the `FeedEvents` stream.
//!
//! Data items (data, decode errors, raw frames) and lifecycle items travel in two bounded queues
//! behind one mutex. Delivery stamps every item with an insertion ordinal under that mutex, and
//! the stream always yields the head with the lower ordinal, so consumers see exact production
//! order, across reconnects too (the per-epoch `seq` restarts at 0 and cannot order items from
//! different epochs). Lifecycle items are never dropped and never wait behind data: if their
//! queue fills, the consumer has stopped reading and the feed ends with `DeliveryOverload`.
//!
//! When the data queue is full, [`OverflowPolicy::Fail`] makes the producer wait (it stops
//! reading the socket) for up to `delivery_wait` and then ends the feed; nothing is dropped.
//! [`OverflowPolicy::DropOldest`] drops the oldest data item instead and reports the loss with
//! `Lifecycle::Lagged` before the next item delivered.
//!
//! Items leave the queues only when the stream returns them, so dropping a pending `next()`
//! future loses nothing. The stream ends with `None` after a clean shutdown, or with exactly one
//! `Err(FeedError)` and then `None`.
//!
//! The stream is woken through an `AtomicWaker` (register, then re-check under the lock), the
//! `Stream` equivalent of a `Notify`; a producer waiting for room uses a `Notify`, whose stored
//! permit covers room freed between its check and its wait.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::Stream;
use futures_util::task::AtomicWaker;
use tokio::sync::Notify;
use tokio::time::Instant;
use tracing::Level;

use super::super::{FeedError, FeedEvent, Lifecycle, OverflowPolicy};
use crate::labels::FeedKind;
use crate::obs::events::{self, emit};
use crate::obs::metrics;

/// Why an item could not be delivered.
#[allow(dead_code, reason = "handled by the feed owner task")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PushError {
    /// The consumer is not keeping up (terminal `DeliveryOverload`).
    Overload,
    /// The event stream was dropped (terminal `ReceiverDropped`).
    ReceiverDropped,
}

/// How the stream ends once the queues drain.
enum Finish {
    Clean,
    Failed(FeedError),
    /// The error has been yielded; only `None` remains.
    Done,
}

/// A queued item: insertion ordinal, enqueue time, item.
type Slot<E> = (u64, Instant, E);

struct State<T> {
    data: VecDeque<Slot<FeedEvent<T>>>,
    lifecycle: VecDeque<Slot<Lifecycle>>,
    next_ordinal: u64,
    /// Data items dropped since the last `Lagged` was yielded.
    dropped: u64,
    dropped_total: u64,
    /// When the last `ws.overflow.dropped` event was emitted.
    last_drop_event: Option<Instant>,
    finish: Option<Finish>,
    receiver_alive: bool,
}

impl<T> State<T> {
    fn ordinal(&mut self) -> u64 {
        let n = self.next_ordinal;
        self.next_ordinal += 1;
        n
    }

    fn depth(&self) -> usize {
        self.data.len() + self.lifecycle.len()
    }
}

fn oldest_ms<E>(queue: &VecDeque<Slot<E>>, now: Instant) -> u64 {
    queue
        .front()
        .map_or(0, |(_, at, _)| millis(now.saturating_duration_since(*at)))
}

struct Shared<T> {
    state: Mutex<State<T>>,
    /// Wakes the stream after a push or `finish`.
    consumer: AtomicWaker,
    /// Wakes a producer waiting for room.
    producer: Notify,
    depth: AtomicUsize,
    feed: FeedKind,
    data_capacity: usize,
    lifecycle_capacity: usize,
}

impl<T> Shared<T> {
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Publishes the depth; called with the lock held so stores cannot reorder.
    fn store_depth(&self, state: &State<T>) {
        let depth = state.depth();
        self.depth.store(depth, Ordering::Relaxed);
        metrics::record_ws_queue_depth(self.feed, depth);
    }
}

/// The owner's side of delivery.
#[allow(dead_code, reason = "owned by the feed owner task")]
pub(crate) struct DeliverySender<T> {
    shared: Arc<Shared<T>>,
}

/// A feed's event stream: data, lifecycle events and decode errors in production order.
///
/// It yields `None` only after a clean shutdown; otherwise it yields exactly one
/// `Err(FeedError)` with the terminal reason, then `None`.
pub struct FeedEvents<T> {
    shared: Arc<Shared<T>>,
}

/// Creates the delivery queues for one feed.
#[allow(dead_code, reason = "used by the feed builder")]
pub(crate) fn channel<T>(
    feed: FeedKind,
    data_capacity: usize,
    lifecycle_capacity: usize,
) -> (DeliverySender<T>, FeedEvents<T>) {
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            data: VecDeque::new(),
            lifecycle: VecDeque::new(),
            next_ordinal: 0,
            dropped: 0,
            dropped_total: 0,
            last_drop_event: None,
            finish: None,
            receiver_alive: true,
        }),
        consumer: AtomicWaker::new(),
        producer: Notify::new(),
        depth: AtomicUsize::new(0),
        feed,
        data_capacity,
        lifecycle_capacity,
    });
    (
        DeliverySender {
            shared: Arc::clone(&shared),
        },
        FeedEvents { shared },
    )
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// What a push attempt did.
enum Attempt<T> {
    /// Queued; `Some(total)` when a drop warrants a (throttled) `ws.overflow.dropped` event.
    Queued(Option<u64>),
    /// The queue is full under `Fail`; the item comes back.
    Full(FeedEvent<T>),
}

#[allow(dead_code, reason = "used by the feed owner task")]
impl<T> DeliverySender<T> {
    fn try_push(
        &self,
        event: FeedEvent<T>,
        policy: OverflowPolicy,
    ) -> Result<Attempt<T>, PushError> {
        let mut state = self.shared.lock();
        if !state.receiver_alive {
            return Err(PushError::ReceiverDropped);
        }
        let now = Instant::now();
        let mut drop_event = None;
        if state.data.len() >= self.shared.data_capacity {
            if policy != OverflowPolicy::DropOldest {
                return Ok(Attempt::Full(event));
            }
            state.data.pop_front();
            state.dropped += 1;
            state.dropped_total += 1;
            metrics::record_ws_dropped(self.shared.feed, 1);
            let due = state
                .last_drop_event
                .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(1));
            if due {
                state.last_drop_event = Some(now);
                drop_event = Some(state.dropped_total);
            }
        }
        let ordinal = state.ordinal();
        state.data.push_back((ordinal, now, event));
        self.shared.store_depth(&state);
        Ok(Attempt::Queued(drop_event))
    }

    fn emit_terminal(&self, queued: usize, oldest_ms: u64) {
        emit!(
            Level::ERROR,
            events::WS_OVERFLOW_TERMINAL,
            feed = self.shared.feed.as_str(),
            queued,
            oldest_ms,
            "delivery queue full: ending the feed"
        );
    }

    fn queued(&self, drop_event: Option<u64>) {
        if let Some(dropped) = drop_event {
            emit!(
                Level::WARN,
                events::WS_OVERFLOW_DROPPED,
                feed = self.shared.feed.as_str(),
                dropped,
                policy = "drop_oldest",
                "delivery queue full: dropped the oldest item"
            );
        }
        self.shared.consumer.wake();
    }

    /// Queues a data, decode-error or raw item. Under [`OverflowPolicy::Fail`] a full queue
    /// makes this wait up to `delivery_wait`.
    pub(crate) async fn push_data(
        &self,
        event: FeedEvent<T>,
        policy: OverflowPolicy,
        delivery_wait: Duration,
    ) -> Result<(), PushError> {
        let deadline = Instant::now() + delivery_wait;
        let mut event = event;
        loop {
            // Created before the check: a notify_one in between leaves a permit for it.
            let room = self.shared.producer.notified();
            match self.try_push(event, policy)? {
                Attempt::Queued(drop_event) => {
                    self.queued(drop_event);
                    return Ok(());
                }
                Attempt::Full(back) => event = back,
            }
            if tokio::time::timeout_at(deadline, room).await.is_err() {
                // Room freed exactly at the deadline still counts.
                return match self.try_push(event, policy)? {
                    Attempt::Queued(drop_event) => {
                        self.queued(drop_event);
                        Ok(())
                    }
                    Attempt::Full(_) => {
                        let (queued, oldest) = {
                            let state = self.shared.lock();
                            (state.data.len(), oldest_ms(&state.data, Instant::now()))
                        };
                        self.emit_terminal(queued, oldest);
                        Err(PushError::Overload)
                    }
                };
            }
        }
    }

    /// Queues a lifecycle item; a full lifecycle queue is terminal.
    pub(crate) fn push_lifecycle(&self, event: Lifecycle) -> Result<(), PushError> {
        let mut state = self.shared.lock();
        if !state.receiver_alive {
            return Err(PushError::ReceiverDropped);
        }
        let now = Instant::now();
        if state.lifecycle.len() >= self.shared.lifecycle_capacity {
            let (queued, oldest) = (state.lifecycle.len(), oldest_ms(&state.lifecycle, now));
            drop(state);
            self.emit_terminal(queued, oldest);
            return Err(PushError::Overload);
        }
        let ordinal = state.ordinal();
        state.lifecycle.push_back((ordinal, now, event));
        self.shared.store_depth(&state);
        drop(state);
        self.shared.consumer.wake();
        Ok(())
    }

    /// Ends the stream once the queued items are consumed: cleanly with `None`, or with the
    /// given error. The first call wins.
    pub(crate) fn finish(&self, error: Option<FeedError>) {
        let mut state = self.shared.lock();
        if state.finish.is_none() {
            state.finish = Some(match error {
                Some(e) => Finish::Failed(e),
                None => Finish::Clean,
            });
        }
        drop(state);
        self.shared.consumer.wake();
    }

    /// Items waiting in both queues.
    pub(crate) fn queue_len(&self) -> usize {
        self.shared.depth.load(Ordering::Relaxed)
    }

    /// Data items dropped so far under `DropOldest`.
    pub(crate) fn dropped_total(&self) -> u64 {
        self.shared.lock().dropped_total
    }

    /// Whether the stream has been dropped.
    pub(crate) fn is_closed(&self) -> bool {
        !self.shared.lock().receiver_alive
    }
}

/// One stream item.
type Item<T> = Result<FeedEvent<T>, FeedError>;

impl<T> FeedEvents<T> {
    /// Takes the next item if one is ready, or the end of the stream.
    fn take(&self) -> Option<Poll<Option<Item<T>>>> {
        let mut state = self.shared.lock();
        let data_head = state.data.front().map(|(n, _, _)| *n);
        let life_head = state.lifecycle.front().map(|(n, _, _)| *n);
        if (data_head.is_some() || life_head.is_some()) && state.dropped > 0 {
            let dropped = std::mem::take(&mut state.dropped);
            return Some(Poll::Ready(Some(Ok(FeedEvent::Lifecycle(
                Lifecycle::Lagged { dropped },
            )))));
        }
        let item = match (data_head, life_head) {
            (Some(d), Some(l)) if l < d => state
                .lifecycle
                .pop_front()
                .map(|(_, _, e)| FeedEvent::Lifecycle(e)),
            (Some(_), _) => state.data.pop_front().map(|(_, _, e)| e),
            (None, Some(_)) => state
                .lifecycle
                .pop_front()
                .map(|(_, _, e)| FeedEvent::Lifecycle(e)),
            (None, None) => None,
        };
        if let Some(item) = item {
            self.shared.store_depth(&state);
            drop(state);
            // Room was made for a waiting producer.
            self.shared.producer.notify_one();
            return Some(Poll::Ready(Some(Ok(item))));
        }
        match state.finish.take() {
            Some(Finish::Clean) => {
                state.finish = Some(Finish::Clean);
                Some(Poll::Ready(None))
            }
            Some(Finish::Failed(error)) => {
                state.finish = Some(Finish::Done);
                Some(Poll::Ready(Some(Err(error))))
            }
            Some(Finish::Done) => {
                state.finish = Some(Finish::Done);
                Some(Poll::Ready(None))
            }
            None => None,
        }
    }
}

impl<T> Stream for FeedEvents<T> {
    type Item = Result<FeedEvent<T>, FeedError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(ready) = self.take() {
            return ready;
        }
        // Register, then re-check, so a push between the two is not missed.
        self.shared.consumer.register(cx.waker());
        self.take().unwrap_or(Poll::Pending)
    }
}

impl<T> Drop for FeedEvents<T> {
    fn drop(&mut self) {
        self.shared.lock().receiver_alive = false;
        // A producer waiting for room must notice.
        self.shared.producer.notify_waiters();
        self.shared.producer.notify_one();
    }
}

#[cfg(test)]
#[path = "delivery_tests.rs"]
mod tests;
