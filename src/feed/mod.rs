//! WebSocket feeds: builders, handles and public feed types, including `FeedLimits`,
//! `ReconnectPolicy` and `OverflowPolicy`.

pub(crate) mod actor;
mod builder;
mod depth;
mod global;
mod handle;
mod market;
mod order_update;
mod protocol;
mod tls;

#[allow(
    unused_imports,
    reason = "glob re-export scheme is fixed before the items exist; each glob imports nothing until its module gains public items"
)]
pub use self::{
    builder::*, depth::*, global::*, handle::*, market::*, order_update::*, protocol::*, tls::*,
};
pub use actor::{CommandError, FeedEvents, Revision, SubscriptionCommand, SubscriptionError};

use std::fmt;
use std::time::{Duration, SystemTime};

use crate::decoder::DecodeError;
use crate::error::{ConfigError, DataErrorCode};

/// Why a feed could not be spawned.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum FeedSpawnError {
    /// `spawn` was called outside a tokio runtime.
    #[error("no tokio runtime")]
    NoRuntime,
    /// The feed URL is not a valid `wss://` (or, for tests, `ws://`) URL.
    #[error("invalid url")]
    InvalidUrl,
    /// The environment has no documented endpoint for this feed (e.g. the sandbox) and no URL
    /// was given.
    #[error("unsupported environment")]
    UnsupportedEnvironment,
    /// An invalid configuration value.
    #[error(transparent)]
    Config(ConfigError),
    /// The TLS configuration could not be built.
    #[error(transparent)]
    Tls(Box<dyn std::error::Error + Send + Sync>),
}

/// How a feed task ended.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum TaskOutcome {
    /// Shut down on request.
    Clean,
    /// Ended for a terminal reason.
    Terminal(TerminalReason),
    /// The owner task panicked.
    Panicked,
}

/// The kind of a WebSocket data frame.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameKind {
    /// A binary frame.
    Binary,
    /// A text frame.
    Text,
}

/// Where a feed is in its lifecycle.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedState {
    /// Opening a connection.
    Connecting,
    /// Connected and writing the desired subscriptions.
    Restoring,
    /// The desired subscriptions have been written (data may or may not be flowing).
    Active,
    /// Waiting before the next connection attempt.
    Backoff,
    /// Shutting down.
    Stopping,
    /// Stopped cleanly (terminal).
    Stopped,
    /// Ended for a terminal reason (terminal).
    Failed,
}

/// Why a live connection ended.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisconnectReason {
    /// The server closed the TCP stream.
    Eof,
    /// The server sent a WebSocket close frame.
    RemoteClose {
        /// The close code, if any.
        code: Option<u16>,
    },
    /// No frame arrived within the liveness timeout.
    LivenessTimeout,
    /// A network or TLS error.
    Transport,
    /// A WebSocket protocol violation, including an oversized frame.
    Protocol,
    /// The server sent a disconnect packet.
    ServerDisconnect {
        /// The disconnect code, if it could be read unambiguously.
        code: Option<u16>,
    },
    /// Shut down on request.
    Shutdown,
}

/// One recent connection failure, kept in [`FeedStatus::last_failures`].
#[derive(Clone, Debug, PartialEq)]
pub struct FailureRecord {
    /// When the connection ended or the attempt failed.
    pub at: SystemTime,
    /// The connection epoch.
    pub epoch: u64,
    /// Why.
    pub reason: DisconnectReason,
    /// The handshake status, for a rejected handshake.
    pub http_status: Option<u16>,
}

fn check(ok: bool, field: &'static str, reason: &'static str) -> Result<(), ConfigError> {
    if ok {
        Ok(())
    } else {
        Err(ConfigError::new(field, reason))
    }
}

const MS: Duration = Duration::from_millis(1);
const SEC: Duration = Duration::from_secs(1);

/// Reconnection budget and backoff (SDK policy).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReconnectPolicy {
    /// Connection attempts per outage: 1..=100 (default 10).
    pub max_attempts: u32,
    /// Longest outage before giving up: 10 s..=1 h (default 5 min).
    pub outage_deadline: Duration,
    /// Base of the full-jitter backoff: 100 ms..=10 s (default 500 ms).
    pub initial_backoff: Duration,
    /// Cap of the full-jitter backoff: `initial_backoff`..=5 min (default 30 s).
    pub max_backoff: Duration,
    /// Seed for the jitter generator; random when `None`.
    pub jitter_seed: Option<u64>,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        ReconnectPolicy {
            max_attempts: 10,
            outage_deadline: 300 * SEC,
            initial_backoff: 500 * MS,
            max_backoff: 30 * SEC,
            jitter_seed: None,
        }
    }
}

impl ReconnectPolicy {
    /// Checks every value against its range.
    pub fn validate(&self) -> Result<(), ConfigError> {
        check(
            (1..=100).contains(&self.max_attempts),
            "reconnect.max_attempts",
            "must be between 1 and 100",
        )?;
        check(
            (10 * SEC..=3600 * SEC).contains(&self.outage_deadline),
            "reconnect.outage_deadline",
            "must be between 10 s and 1 h",
        )?;
        check(
            (100 * MS..=10 * SEC).contains(&self.initial_backoff),
            "reconnect.initial_backoff",
            "must be between 100 ms and 10 s",
        )?;
        check(
            (self.initial_backoff..=300 * SEC).contains(&self.max_backoff),
            "reconnect.max_backoff",
            "must be between the initial backoff and 5 min",
        )
    }
}

/// Bounds on one feed's queues, waits and frames (SDK policy).
#[non_exhaustive]
#[derive(Clone, Copy, Debug)]
pub struct FeedLimits {
    /// Data-queue capacity: 64..=1 000 000 (default 4096).
    pub queue_capacity: usize,
    /// Lifecycle-queue capacity: 16..=4096 (default 256).
    pub lifecycle_capacity: usize,
    /// Command mailbox capacity: 1..=1024 (default 64).
    pub mailbox: usize,
    /// How long a full data queue may hold up reading under `OverflowPolicy::Fail`: 10 ms..=10 s
    /// (default 1 s).
    pub delivery_wait: Duration,
    /// Handshake timeout: 1 s..=60 s (default 10 s).
    pub handshake_timeout: Duration,
    /// Silence after which the connection is considered dead: 5 s..=120 s (default 30 s). The
    /// server pings every 10 s and closes after 40 s without a pong (DOC:5896-5902).
    pub liveness_timeout: Duration,
    /// Bound on a clean shutdown: 1 s..=60 s (default 5 s).
    pub shutdown_timeout: Duration,
    /// Largest accepted message or frame: 4 KiB..=16 MiB (default 1 MiB).
    pub max_frame_bytes: usize,
    /// Reconnection budget.
    pub reconnect: ReconnectPolicy,
}

impl Default for FeedLimits {
    fn default() -> Self {
        FeedLimits {
            queue_capacity: 4096,
            lifecycle_capacity: 256,
            mailbox: 64,
            delivery_wait: SEC,
            handshake_timeout: 10 * SEC,
            liveness_timeout: 30 * SEC,
            shutdown_timeout: 5 * SEC,
            max_frame_bytes: 1024 * 1024,
            reconnect: ReconnectPolicy::default(),
        }
    }
}

impl FeedLimits {
    /// Checks every value against its range.
    pub fn validate(&self) -> Result<(), ConfigError> {
        check(
            (64..=1_000_000).contains(&self.queue_capacity),
            "queue_capacity",
            "must be between 64 and 1000000",
        )?;
        check(
            (16..=4096).contains(&self.lifecycle_capacity),
            "lifecycle_capacity",
            "must be between 16 and 4096",
        )?;
        check(
            (1..=1024).contains(&self.mailbox),
            "mailbox",
            "must be between 1 and 1024",
        )?;
        check(
            (10 * MS..=10 * SEC).contains(&self.delivery_wait),
            "delivery_wait",
            "must be between 10 ms and 10 s",
        )?;
        check(
            (SEC..=60 * SEC).contains(&self.handshake_timeout),
            "handshake_timeout",
            "must be between 1 s and 60 s",
        )?;
        check(
            (5 * SEC..=120 * SEC).contains(&self.liveness_timeout),
            "liveness_timeout",
            "must be between 5 s and 120 s",
        )?;
        check(
            (SEC..=60 * SEC).contains(&self.shutdown_timeout),
            "shutdown_timeout",
            "must be between 1 s and 60 s",
        )?;
        check(
            (4 * 1024..=16 * 1024 * 1024).contains(&self.max_frame_bytes),
            "max_frame_bytes",
            "must be between 4 KiB and 16 MiB",
        )?;
        self.reconnect.validate()
    }
}

/// What to do when the data queue is full.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverflowPolicy {
    /// Stop reading and wait up to `delivery_wait` for room, then end the feed. Nothing is
    /// dropped (the default).
    #[default]
    Fail,
    /// Drop the oldest data item and report the loss with `Lifecycle::Lagged`.
    DropOldest,
}

/// One item of a feed's event stream.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum FeedEvent<T> {
    /// A decoded data item.
    Data(Delivery<T>),
    /// A lifecycle transition.
    Lifecycle(Lifecycle),
    /// A frame or packet that could not be decoded (non-terminal).
    DecodeError(DecodeError),
    /// A raw frame, when raw capture is on; it precedes the frame's data items.
    Raw(RawFrame),
}

/// A decoded data item with its position in the stream.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct Delivery<T> {
    /// The connection epoch (starts at 1).
    pub epoch: u64,
    /// The item's sequence number within the epoch, shared with lifecycle and error items.
    pub seq: u64,
    /// When the frame was received.
    pub received_at: SystemTime,
    /// The item.
    pub value: T,
}

/// A received frame, kept for fixture capture.
#[non_exhaustive]
#[derive(Clone, PartialEq)]
pub struct RawFrame {
    /// The connection epoch.
    pub epoch: u64,
    /// The frame's sequence number within the epoch.
    pub seq: u64,
    /// When the frame was received.
    pub received_at: SystemTime,
    /// Binary or text.
    pub kind: FrameKind,
    /// The frame's bytes.
    pub bytes: Vec<u8>,
}

impl fmt::Debug for RawFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RawFrame")
            .field("epoch", &self.epoch)
            .field("seq", &self.seq)
            .field("received_at", &self.received_at)
            .field("kind", &self.kind)
            .field("len", &self.bytes.len())
            .finish()
    }
}

/// A lifecycle transition, delivered in order with the data.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum Lifecycle {
    /// A connection attempt started.
    Connecting {
        /// The new epoch.
        epoch: u64,
        /// The attempt number within the outage.
        attempt: u32,
    },
    /// The handshake succeeded.
    Connected {
        /// The epoch.
        epoch: u64,
    },
    /// Subscription commands were written.
    CommandsSent {
        /// The desired revision they implement.
        revision: Revision,
    },
    /// The desired subscriptions were written after connecting.
    Active {
        /// The epoch.
        epoch: u64,
        /// The desired revision written.
        revision: Revision,
    },
    /// A live connection ended.
    Disconnected {
        /// The epoch.
        epoch: u64,
        /// Why.
        reason: DisconnectReason,
    },
    /// The server sent a disconnect packet.
    ServerDisconnect {
        /// The epoch.
        epoch: u64,
        /// The code as sent.
        code: u16,
        /// The code, if it is a documented data API error code.
        known: Option<DataErrorCode>,
    },
    /// A connection was re-established after one was lost. Facts only: no count of missed
    /// items is invented.
    Gap {
        /// The epoch that was lost.
        previous_epoch: u64,
        /// The last sequence number delivered in that epoch, if any.
        last_seq: Option<u64>,
        /// When the previous connection ended.
        disconnected_at: SystemTime,
        /// When the new connection was established.
        reconnected_at: SystemTime,
        /// Why the previous connection ended.
        reason: DisconnectReason,
    },
    /// Waiting before the next attempt.
    Backoff {
        /// The wait.
        delay: Duration,
        /// The next attempt number.
        next_attempt: u32,
    },
    /// Data items were dropped under `OverflowPolicy::DropOldest`.
    Lagged {
        /// How many.
        dropped: u64,
    },
    /// The feed stopped.
    Stopped,
}

/// Why a feed ended.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalReason {
    /// Shut down on request.
    Shutdown,
    /// The handshake was refused with 401 or 403.
    AuthRejected {
        /// The HTTP status.
        http_status: u16,
    },
    /// The handshake was refused with another non-retryable status.
    HandshakeRejected {
        /// The HTTP status.
        http_status: u16,
    },
    /// The server sent a terminal disconnect code.
    ServerDisconnect {
        /// The code.
        code: u16,
    },
    /// The reconnection budget ran out.
    ReconnectExhausted {
        /// Attempts made in the outage.
        attempts: u32,
        /// Why the last connection ended.
        last: DisconnectReason,
    },
    /// The consumer stopped reading and the queues filled.
    DeliveryOverload,
    /// The event stream was dropped.
    ReceiverDropped,
    /// Every handle was dropped.
    HandlesDropped,
    /// A write to the socket was interrupted.
    SendInterrupted,
    /// A clean shutdown did not finish in time.
    ShutdownTimeout,
    /// The owner task panicked.
    Panicked,
}

impl fmt::Display for TerminalReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shutdown => f.write_str("shut down"),
            Self::AuthRejected { http_status } => {
                write!(f, "authentication rejected (HTTP {http_status})")
            }
            Self::HandshakeRejected { http_status } => {
                write!(f, "handshake rejected (HTTP {http_status})")
            }
            Self::ServerDisconnect { code } => write!(f, "server disconnected with code {code}"),
            Self::ReconnectExhausted { attempts, last } => {
                write!(
                    f,
                    "reconnection gave up after {attempts} attempts (last: {last:?})"
                )
            }
            Self::DeliveryOverload => f.write_str("the consumer fell behind"),
            Self::ReceiverDropped => f.write_str("the event stream was dropped"),
            Self::HandlesDropped => f.write_str("every handle was dropped"),
            Self::SendInterrupted => f.write_str("a socket write was interrupted"),
            Self::ShutdownTimeout => f.write_str("shutdown timed out"),
            Self::Panicked => f.write_str("the feed task panicked"),
        }
    }
}

/// The terminal error a feed's stream yields once, before ending.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedError(pub TerminalReason);

impl fmt::Display for FeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "feed ended: {}", self.0)
    }
}

impl std::error::Error for FeedError {}

/// A snapshot of a feed's state; reading it never waits on the data queue.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct FeedStatus {
    /// Lifecycle state.
    pub state: FeedState,
    /// Current connection epoch.
    pub epoch: u64,
    /// The latest accepted revision of the desired subscriptions.
    pub desired_revision: Revision,
    /// The revision last written to the server, if any.
    pub sent_revision: Option<Revision>,
    /// Number of desired subscriptions.
    pub subscriptions: usize,
    /// Items waiting in the data queue.
    pub queue_len: usize,
    /// Data items dropped so far under `DropOldest`.
    pub dropped_total: u64,
    /// Time since the last inbound frame.
    pub last_frame_age: Option<Duration>,
    /// Up to 16 recent failures, oldest first.
    pub last_failures: Vec<FailureRecord>,
    /// The terminal reason, once the feed has ended.
    pub terminal: Option<TerminalReason>,
}

#[cfg(test)]
mod tests;
