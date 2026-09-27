//! `FeedBuilder<P>`: URL, limits, overflow policy, raw capture and spawn.

use std::collections::BTreeSet;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures_util::FutureExt;
use tokio::sync::mpsc;
use tracing::Instrument;

use super::actor::{self, Desired, Owner, ReconnectBudget, StatusCell, Stop};
use super::handle::{FeedHandle, FeedTask};
use super::protocol::FeedProtocol;
use super::{
    FeedError, FeedEvents, FeedLimits, FeedSpawnError, FeedState, FeedStatus, OverflowPolicy,
    TaskOutcome, TerminalReason, tls,
};
use crate::obs::spans;

/// Process-unique session ids for the `dhani.ws.session` span.
static SESSIONS: AtomicU64 = AtomicU64::new(1);

/// Configures and spawns one feed.
#[allow(
    private_bounds,
    reason = "the protocol trait is sealed and crate-private; users name the feed aliases"
)]
pub struct FeedBuilder<P: FeedProtocol> {
    protocol: P,
    /// Whether the protocol has a documented endpoint for the chosen environment.
    endpoint_available: bool,
    url: Option<url::Url>,
    limits: FeedLimits,
    overflow: OverflowPolicy,
    capture_raw: bool,
}

#[allow(
    private_bounds,
    private_interfaces,
    reason = "the protocol trait is sealed and crate-private; users name the feed aliases"
)]
impl<P: FeedProtocol> FeedBuilder<P> {
    /// A builder for `protocol`; `endpoint_available` is false where the environment has no
    /// documented endpoint for the feed (a URL must then be given).
    #[allow(dead_code, reason = "used by the feed builders")]
    pub(crate) fn new(protocol: P, endpoint_available: bool) -> Self {
        FeedBuilder {
            protocol,
            endpoint_available,
            url: None,
            limits: FeedLimits::default(),
            overflow: OverflowPolicy::default(),
            capture_raw: false,
        }
    }

    /// Connects to `url` instead of the documented endpoint (`ws://` is allowed, for tests). Feeds
    /// that authenticate in the query string add their credentials to it.
    pub fn url(mut self, url: url::Url) -> Self {
        self.url = Some(url);
        self
    }

    /// Queue, timeout and reconnection limits.
    pub fn limits(mut self, limits: FeedLimits) -> Self {
        self.limits = limits;
        self
    }

    /// What to do when the consumer falls behind.
    pub fn overflow(mut self, policy: OverflowPolicy) -> Self {
        self.overflow = policy;
        self
    }

    /// Also delivers every received frame as `FeedEvent::Raw`, before its data items.
    pub fn capture_raw(mut self, on: bool) -> Self {
        self.capture_raw = on;
        self
    }

    /// The protocol (test support).
    #[cfg(test)]
    pub(crate) fn into_protocol_for_tests(self) -> P {
        self.protocol
    }

    /// The limits (test support).
    #[cfg(test)]
    pub(crate) fn limits_for_tests(&self) -> FeedLimits {
        self.limits
    }

    /// Starts the feed on the current tokio runtime.
    #[allow(
        clippy::type_complexity,
        reason = "the handle, stream and task triple of the public API (§8.2)"
    )]
    pub fn spawn(
        self,
    ) -> Result<(FeedHandle<P::Sub>, FeedEvents<P::Data>, FeedTask), FeedSpawnError> {
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| FeedSpawnError::NoRuntime)?;
        self.limits.validate().map_err(FeedSpawnError::Config)?;
        let url = match &self.url {
            Some(url) if matches!(url.scheme(), "ws" | "wss") && url.host().is_some() => {
                self.protocol.url_for(url)
            }
            Some(_) => return Err(FeedSpawnError::InvalidUrl),
            None if self.endpoint_available => self.protocol.url().clone(),
            None => return Err(FeedSpawnError::UnsupportedEnvironment),
        };
        let secure = {
            use secrecy::ExposeSecret;
            url.expose_secret().starts_with("wss://")
        };
        let connector = if secure {
            Some(tls::connector()?)
        } else {
            None
        };
        let (tx, mailbox) = mpsc::channel(self.limits.mailbox);
        let (status, reader) = StatusCell::new(FeedStatus::initial());
        let status = Arc::new(status);
        let (delivery, events) = actor::channel(
            P::FEED,
            self.limits.queue_capacity,
            self.limits.lifecycle_capacity,
        );
        let stop = Arc::new(Stop::default());
        let owner = Owner {
            protocol: self.protocol,
            url,
            connector,
            limits: self.limits,
            overflow: self.overflow,
            capture_raw: self.capture_raw,
            mailbox,
            status: Arc::clone(&status),
            delivery: delivery.clone(),
            stop: Arc::clone(&stop),
            budget: ReconnectBudget::new(self.limits.reconnect),
            desired: Desired::new(),
            sent: BTreeSet::new(),
            epoch: 0,
            seq: 0,
            lost: None,
            last_data_seq: None,
            status_queue: (0, 0),
        };
        let session = spans::ws_session(P::FEED, SESSIONS.fetch_add(1, Ordering::Relaxed));
        let task = runtime.spawn(
            async move {
                match AssertUnwindSafe(owner.run()).catch_unwind().await {
                    Ok(TerminalReason::Shutdown) => TaskOutcome::Clean,
                    Ok(reason) => TaskOutcome::Terminal(reason),
                    Err(_) => {
                        status.update(|s| {
                            s.state = FeedState::Failed;
                            s.terminal = Some(TerminalReason::Panicked);
                        });
                        delivery.finish(Some(FeedError(TerminalReason::Panicked)));
                        TaskOutcome::Panicked
                    }
                }
            }
            .instrument(session),
        );
        Ok((
            FeedHandle::new(tx, reader, stop),
            events,
            FeedTask::new(task),
        ))
    }
}

#[cfg(test)]
#[path = "builder_tests.rs"]
mod tests;
