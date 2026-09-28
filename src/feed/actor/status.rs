//! The feed status cell: the owner updates it, handles read it without waiting on the data queue.
//!
//! Status changes are published through a `watch` channel with a version number, so a reader can
//! wait for "anything newer than the version I saw" and several quick changes coalesce into one
//! wake-up. The last-frame time changes with every frame, so it is kept in an atomic instead and
//! turned into `last_frame_age` when a snapshot is taken.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::watch;
use tokio::time::Instant;

use super::super::{FailureRecord, FeedState, FeedStatus};
use super::Revision;

/// Most failures kept in [`FeedStatus::last_failures`].
const MAX_FAILURES: usize = 16;

#[allow(dead_code, reason = "used by the feed owner task")]
impl FeedStatus {
    /// The status of a feed that has not connected yet.
    pub(crate) fn initial() -> Self {
        FeedStatus {
            state: FeedState::Connecting,
            epoch: 0,
            desired_revision: Revision(0),
            sent_revision: None,
            subscriptions: 0,
            queue_len: 0,
            dropped_total: 0,
            last_frame_age: None,
            last_failures: Vec::new(),
            terminal: None,
        }
    }
}

#[derive(Clone)]
struct Versioned {
    version: u64,
    status: FeedStatus,
}

/// The time of the last inbound frame, as nanoseconds after `base` plus one (0 = none yet).
struct LastFrame {
    base: Instant,
    nanos: AtomicU64,
}

impl LastFrame {
    fn age(&self, now: Instant) -> Option<std::time::Duration> {
        match self.nanos.load(Ordering::Relaxed) {
            0 => None,
            n => {
                let at = self.base + std::time::Duration::from_nanos(n - 1);
                Some(now.saturating_duration_since(at))
            }
        }
    }
}

/// The owner's side of the status.
#[allow(dead_code, reason = "owned by the feed owner task")]
pub(crate) struct StatusCell {
    tx: watch::Sender<Versioned>,
    last_frame: Arc<LastFrame>,
}

/// A handle's side of the status. Cheap to clone.
#[allow(dead_code, reason = "held by feed handles")]
#[derive(Clone)]
pub(crate) struct StatusReader {
    rx: watch::Receiver<Versioned>,
    last_frame: Arc<LastFrame>,
    /// The delivery queues' live depth, read at snapshot time.
    queue_depth: Option<Arc<std::sync::atomic::AtomicUsize>>,
}

#[allow(dead_code, reason = "used by the feed owner task")]
impl StatusCell {
    /// A cell holding `initial`, and a reader for it.
    pub(crate) fn new(initial: FeedStatus) -> (StatusCell, StatusReader) {
        let (tx, rx) = watch::channel(Versioned {
            version: 0,
            status: initial,
        });
        let last_frame = Arc::new(LastFrame {
            base: Instant::now(),
            nanos: AtomicU64::new(0),
        });
        let reader = StatusReader {
            rx,
            last_frame: Arc::clone(&last_frame),
            queue_depth: None,
        };
        (StatusCell { tx, last_frame }, reader)
    }

    /// Changes the status and publishes it under the next version.
    pub(crate) fn update(&self, change: impl FnOnce(&mut FeedStatus)) {
        self.tx.send_modify(|v| {
            change(&mut v.status);
            v.version += 1;
        });
    }

    /// Records a failure, keeping the most recent 16.
    pub(crate) fn record_failure(&self, failure: FailureRecord) {
        self.update(|s| {
            s.last_failures.push(failure);
            let excess = s.last_failures.len().saturating_sub(MAX_FAILURES);
            s.last_failures.drain(..excess);
        });
    }

    /// Notes that a frame arrived at `now` (no wake-up is published).
    pub(crate) fn frame_received(&self, now: Instant) {
        let nanos = now
            .saturating_duration_since(self.last_frame.base)
            .as_nanos();
        let stored = u64::try_from(nanos)
            .unwrap_or(u64::MAX - 1)
            .saturating_add(1);
        self.last_frame.nanos.store(stored, Ordering::Relaxed);
    }
}

#[allow(dead_code, reason = "used by feed handles")]
impl StatusReader {
    /// The current status, with `last_frame_age` measured now.
    pub(crate) fn snapshot(&self) -> FeedStatus {
        self.read(&self.rx.borrow())
    }

    /// The version of the current status.
    pub(crate) fn version(&self) -> u64 {
        self.rx.borrow().version
    }

    /// Waits for a status newer than version `since` (several changes coalesce into one), and
    /// returns it with its version. If the feed has ended, returns the final status.
    pub(crate) async fn changed(&self, since: u64) -> (u64, FeedStatus) {
        let mut rx = self.rx.clone();
        if let Ok(v) = rx.wait_for(|v| v.version > since).await {
            return (v.version, self.read(&v));
        }
        // The owner is gone: the last published status is final.
        let v = rx.borrow();
        (v.version, self.read(&v))
    }

    fn read(&self, v: &Versioned) -> FeedStatus {
        let mut status = v.status.clone();
        status.last_frame_age = self.last_frame.age(Instant::now());
        if let Some(depth) = &self.queue_depth {
            status.queue_len = depth.load(std::sync::atomic::Ordering::Relaxed);
        }
        status
    }

    /// Reads `queue_len` from the delivery queues' live counter instead of the last published
    /// value, which is only refreshed when data is pushed.
    pub(crate) fn with_queue_depth(mut self, depth: Arc<std::sync::atomic::AtomicUsize>) -> Self {
        self.queue_depth = Some(depth);
        self
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::super::super::DisconnectReason;
    use super::*;

    fn failure(epoch: u64) -> FailureRecord {
        FailureRecord {
            at: SystemTime::UNIX_EPOCH,
            epoch,
            reason: DisconnectReason::Eof,
            http_status: None,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn updates_are_versioned_and_visible_to_readers() {
        let (cell, reader) = StatusCell::new(FeedStatus::initial());
        assert_eq!(
            (reader.version(), reader.snapshot().state),
            (0, FeedState::Connecting)
        );
        cell.update(|s| {
            s.state = FeedState::Active;
            s.epoch = 1;
        });
        let s = reader.snapshot();
        assert_eq!(
            (reader.version(), s.state, s.epoch),
            (1, FeedState::Active, 1)
        );
        assert_eq!(s.last_frame_age, None);
    }

    #[tokio::test(start_paused = true)]
    async fn only_the_last_sixteen_failures_are_kept() {
        let (cell, reader) = StatusCell::new(FeedStatus::initial());
        for epoch in 1..=20 {
            cell.record_failure(failure(epoch));
        }
        let epochs: Vec<u64> = reader
            .snapshot()
            .last_failures
            .iter()
            .map(|f| f.epoch)
            .collect();
        assert_eq!(epochs, (5..=20).collect::<Vec<_>>());
    }

    #[tokio::test(start_paused = true)]
    async fn the_last_frame_age_is_measured_when_read() {
        let (cell, reader) = StatusCell::new(FeedStatus::initial());
        let version = reader.version();
        cell.frame_received(Instant::now());
        tokio::time::advance(Duration::from_millis(1500)).await;
        assert_eq!(
            reader.snapshot().last_frame_age,
            Some(Duration::from_millis(1500))
        );
        // Frames publish no wake-up.
        assert_eq!(reader.version(), version);
    }

    #[tokio::test(start_paused = true)]
    async fn changed_coalesces_and_returns_the_newest_status() {
        let (cell, reader) = StatusCell::new(FeedStatus::initial());
        cell.update(|s| s.epoch = 1);
        cell.update(|s| s.epoch = 2);
        cell.update(|s| s.epoch = 3);
        let (version, status) = reader.changed(0).await;
        assert_eq!((version, status.epoch), (3, 3));
        // Waits for the next change.
        let waiter = tokio::spawn({
            let reader = reader.clone();
            async move { reader.changed(3).await }
        });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        cell.update(|s| s.state = FeedState::Stopped);
        let (version, status) = waiter.await.unwrap();
        assert_eq!((version, status.state), (4, FeedState::Stopped));
    }

    #[tokio::test(start_paused = true)]
    async fn changed_returns_the_final_status_once_the_owner_is_gone() {
        let (cell, reader) = StatusCell::new(FeedStatus::initial());
        cell.update(|s| s.state = FeedState::Failed);
        drop(cell);
        let (version, status) = reader.changed(7).await;
        assert_eq!((version, status.state), (1, FeedState::Failed));
    }
}
