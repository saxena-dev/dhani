//! The single owner task of a feed.
//!
//! The owner alone holds the socket, the connection epoch, the sequence counter, the desired
//! subscriptions and the termination decision. Handles reach it only through the command
//! mailbox, the status snapshot and the stop signal; there is no shared socket. Every `select!`
//! is biased: stop first, then commands, then the socket, then timers.
//!
//! One connection runs: connect (under the handshake timeout) → `Connected` (and `Gap` after an
//! earlier loss) → the protocol's opening messages → `Restoring`, writing the whole desired set →
//! `Active` → serve until the connection ends → reconnect or finish, per the disposition table.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use futures_util::{SinkExt, StreamExt};
use secrecy::{ExposeSecret, SecretString};
use tokio::net::TcpStream;
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest};
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};
use tracing::{Instrument, Level, Span};

use super::super::protocol::{FeedProtocol, Message};
use super::super::{
    DisconnectReason, FailureRecord, FeedError, FeedEvent, FeedLimits, FeedState, Lifecycle,
    OverflowPolicy, TerminalReason,
};
use super::delivery::{DeliverySender, PushError};
use super::lifecycle::{Cause, Disposition, ReconnectBudget, dispose};
use super::status::StatusCell;
use super::subscriptions::{CommandError, Desired, Revision, SubscriptionCommand};
use crate::obs::events::{self, emit};
use crate::obs::metrics::{self, ConnectResult, ReconnectReason};
use crate::obs::spans;

#[path = "owner_frames.rs"]
mod frames;

use frames::Sampler;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// One command from a handle, with its reply channel and the caller's `ws.command` span.
pub(crate) struct Envelope<S> {
    pub(crate) command: SubscriptionCommand<S>,
    pub(crate) reply: oneshot::Sender<Result<Revision, CommandError>>,
    pub(crate) span: Span,
}

/// The stop request shared by every handle: a flag plus a wake-up, needing no mailbox room.
#[derive(Default)]
pub(crate) struct Stop {
    flag: AtomicBool,
    notify: Notify,
}

impl Stop {
    /// Asks the owner to shut down.
    pub(crate) fn request(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
        self.notify.notify_one();
    }

    fn requested(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Resolves once a stop has been requested.
    async fn wait(&self) {
        loop {
            let woken = self.notify.notified();
            if self.requested() {
                return;
            }
            woken.await;
        }
    }
}

/// The label of a disconnect reason (span `cause` and reconnect metric).
fn reason_label(reason: &DisconnectReason) -> (&'static str, ReconnectReason) {
    match reason {
        DisconnectReason::Eof => ("eof", ReconnectReason::Eof),
        DisconnectReason::RemoteClose { .. } => ("remote_close", ReconnectReason::RemoteClose),
        DisconnectReason::LivenessTimeout => ("liveness_timeout", ReconnectReason::LivenessTimeout),
        DisconnectReason::Transport => ("transport", ReconnectReason::Transport),
        DisconnectReason::Protocol => ("protocol", ReconnectReason::Protocol),
        DisconnectReason::ServerDisconnect { .. } => {
            ("server_disconnect", ReconnectReason::ServerDisconnect)
        }
        DisconnectReason::Shutdown => ("shutdown", ReconnectReason::Shutdown),
    }
}

/// How one connection ended.
enum Ended {
    /// Reconnect or finish according to the disposition of this cause.
    Cause(Cause, Option<u16>),
    /// A clean shutdown was requested.
    Stop,
    /// A terminal reason decided locally (delivery, handles, interrupted write).
    Terminal(TerminalReason),
}

/// Decrements the connected gauge when a connection ends, however it ends.
struct ConnectedGauge(crate::labels::FeedKind);

impl Drop for ConnectedGauge {
    fn drop(&mut self) {
        metrics::record_ws_connections_active(self.0, false);
    }
}

/// Opens the socket; on failure, the cause and the handshake status if one was received.
async fn handshake(
    url: SecretString,
    max_frame_bytes: usize,
    connector: Option<Connector>,
) -> Result<Socket, (Cause, Option<u16>)> {
    let request = url
        .expose_secret()
        .into_client_request()
        .map_err(|_| (Cause::Connect, None))?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(max_frame_bytes))
        .max_frame_size(Some(max_frame_bytes));
    match tokio_tungstenite::connect_async_tls_with_config(request, Some(config), true, connector)
        .await
    {
        Ok((socket, _)) => Ok(socket),
        // Keep only the status: the error may carry the URL, which holds credentials.
        Err(tungstenite::Error::Http(response)) => {
            let status = response.status().as_u16();
            Err((Cause::HandshakeStatus(status), Some(status)))
        }
        Err(tungstenite::Error::Tls(_)) => Err((Cause::Tls, None)),
        Err(_) => Err((Cause::Connect, None)),
    }
}

/// The owner task's state.
pub(crate) struct Owner<P: FeedProtocol> {
    pub(crate) protocol: P,
    pub(crate) url: SecretString,
    pub(crate) connector: Option<Connector>,
    pub(crate) limits: FeedLimits,
    pub(crate) overflow: OverflowPolicy,
    pub(crate) capture_raw: bool,
    pub(crate) mailbox: mpsc::Receiver<Envelope<P::Sub>>,
    pub(crate) status: Arc<StatusCell>,
    pub(crate) delivery: DeliverySender<P::Data>,
    pub(crate) stop: Arc<Stop>,
    pub(crate) budget: ReconnectBudget,
    pub(crate) desired: Desired<P::Sub>,
    pub(crate) sent: BTreeSet<P::Sub>,
    pub(crate) epoch: u64,
    pub(crate) seq: u64,
    /// The last lost connection: epoch, last data seq, when, why.
    pub(crate) lost: Option<(u64, Option<u64>, SystemTime, DisconnectReason)>,
    pub(crate) last_data_seq: Option<u64>,
    /// The queue length and drop count last published in the status.
    pub(crate) status_queue: (usize, u64),
    /// The desired revision last written to a connection, the same value last published as
    /// `FeedStatus::sent_revision`. It is `None` until the first restore, so the first connection
    /// never reports `CommandsSent` on restore, even for commands queued while it was pending.
    pub(crate) status_sent_revision: Option<Revision>,
}

impl<P: FeedProtocol> Owner<P> {
    /// Runs the feed to its end and reports the terminal reason (`Shutdown` for a clean stop).
    pub(crate) async fn run(mut self) -> TerminalReason {
        let reason = self.connections().await;
        // The session span is current here (the task is instrumented with it).
        Span::current().record("terminal_reason", format!("{reason:?}").as_str());
        let clean = reason == TerminalReason::Shutdown;
        self.status.update(|s| {
            s.state = if clean {
                FeedState::Stopped
            } else {
                FeedState::Failed
            };
            s.terminal = Some(reason.clone());
        });
        if clean {
            let _ = self.lifecycle(Lifecycle::Stopped);
            self.delivery.finish(None);
        } else {
            self.delivery.finish(Some(FeedError(reason.clone())));
        }
        emit!(
            Level::INFO,
            events::WS_STOPPED,
            feed = P::FEED.as_str(),
            terminal_reason = format!("{reason:?}"),
            clean,
            "feed stopped"
        );
        reason
    }

    /// Takes the next sequence number of the current epoch.
    fn next_seq(&mut self) -> u64 {
        let seq = self.seq;
        self.seq += 1;
        seq
    }

    fn lifecycle(&mut self, event: Lifecycle) -> Result<(), TerminalReason> {
        let _seq = self.next_seq();
        self.delivery.push_lifecycle(event).map_err(|e| match e {
            PushError::Overload => TerminalReason::DeliveryOverload,
            PushError::ReceiverDropped => TerminalReason::ReceiverDropped,
        })
    }

    async fn data(&mut self, event: FeedEvent<P::Data>) -> Result<(), TerminalReason> {
        self.delivery
            .push_data(event, self.overflow, self.limits.delivery_wait)
            .await
            .map_err(|e| match e {
                PushError::Overload => TerminalReason::DeliveryOverload,
                PushError::ReceiverDropped => TerminalReason::ReceiverDropped,
            })?;
        // Publish only real changes, so status watchers are not woken per item.
        let (queue_len, dropped_total) = (self.delivery.queue_len(), self.delivery.dropped_total());
        let current = self.status_queue;
        if current != (queue_len, dropped_total) {
            self.status_queue = (queue_len, dropped_total);
            self.status.update(|s| {
                s.queue_len = queue_len;
                s.dropped_total = dropped_total;
            });
        }
        Ok(())
    }

    /// Applies a command and replies; the caller's span is entered while it is applied.
    fn apply(&mut self, envelope: Envelope<P::Sub>) -> bool {
        let capacity = self.protocol.capacity();
        let result = envelope
            .span
            .in_scope(|| self.desired.apply(&envelope.command, capacity, P::key));
        let changed = matches!(result, Ok(Some(_)));
        let reply = match result {
            Ok(Some(revision)) => {
                envelope.span.record("revision", revision.0);
                envelope.span.record("decision", "accepted");
                Ok(revision)
            }
            Ok(None) => {
                envelope.span.record("revision", self.desired.revision().0);
                envelope.span.record("decision", "unchanged");
                Ok(self.desired.revision())
            }
            Err(e) => {
                envelope.span.record("decision", "rejected");
                envelope.span.record("rejection", e.to_string().as_str());
                emit!(
                    Level::DEBUG,
                    events::WS_SUBSCRIPTION_REJECTED,
                    feed = P::FEED.as_str(),
                    reason = e.to_string(),
                    "subscription command refused"
                );
                Err(CommandError::Invalid(e))
            }
        };
        let _ = envelope.reply.send(reply);
        let (revision, subscriptions) = (self.desired.revision(), self.desired.set().len());
        self.status.update(|s| {
            s.desired_revision = revision;
            s.subscriptions = subscriptions;
        });
        changed
    }

    /// Connection attempts until the feed ends.
    async fn connections(&mut self) -> TerminalReason {
        let mut cause_label = "start";
        loop {
            if self.stop.requested() {
                return TerminalReason::Shutdown;
            }
            let attempt = self.budget.attempts() + 1;
            self.epoch += 1;
            self.seq = 0;
            self.last_data_seq = None;
            let span = spans::ws_connection(P::FEED, self.epoch, attempt, cause_label);
            let ended = self.connection(attempt).instrument(span.clone()).await;
            let (cause, http_status) = match ended {
                Ended::Stop => return TerminalReason::Shutdown,
                Ended::Terminal(reason) => return reason,
                Ended::Cause(cause, status) => (cause, status),
            };
            let reason = match &cause {
                Cause::Disconnected(r) => r.clone(),
                Cause::ServerDisconnect(code) => DisconnectReason::ServerDisconnect { code: *code },
                Cause::GlobalError(code) => {
                    DisconnectReason::ServerDisconnect { code: Some(*code) }
                }
                _ => DisconnectReason::Transport,
            };
            self.status.record_failure(FailureRecord {
                at: SystemTime::now(),
                epoch: self.epoch,
                reason: reason.clone(),
                http_status,
            });
            if let Disposition::Terminal(t) = dispose(&cause) {
                return t;
            }
            self.budget.mark_lost();
            let (label, metric) = reason_label(&reason);
            cause_label = label;
            let Some(delay) = self.budget.next_delay(Instant::now()) else {
                let attempts = self.budget.attempts();
                let outage_ms = self
                    .budget
                    .outage_elapsed(Instant::now())
                    .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
                emit!(
                    Level::ERROR,
                    events::WS_RECONNECT_EXHAUSTED,
                    feed = P::FEED.as_str(),
                    attempts,
                    outage_ms,
                    "reconnection budget exhausted"
                );
                return TerminalReason::ReconnectExhausted {
                    attempts,
                    last: reason,
                };
            };
            metrics::record_ws_reconnect(P::FEED, metric);
            let next_attempt = self.budget.attempts() + 1;
            emit!(
                Level::INFO,
                events::WS_RECONNECT_SCHEDULED,
                feed = P::FEED.as_str(),
                attempt = next_attempt,
                delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
                "reconnecting after a backoff"
            );
            self.status.update(|s| s.state = FeedState::Backoff);
            if let Err(t) = self.lifecycle(Lifecycle::Backoff {
                delay,
                next_attempt,
            }) {
                return t;
            }
            if let Some(end) = self.back_off(delay).await {
                return end;
            }
        }
    }

    /// Waits out a backoff, still accepting commands; `Some` ends the feed.
    async fn back_off(&mut self, delay: Duration) -> Option<TerminalReason> {
        let until = tokio::time::sleep(delay);
        tokio::pin!(until);
        let stop = Arc::clone(&self.stop);
        loop {
            tokio::select! {
                biased;
                () = stop.wait() => return Some(TerminalReason::Shutdown),
                envelope = self.mailbox.recv() => match envelope {
                    Some(envelope) => {
                        self.apply(envelope);
                    }
                    None => return Some(TerminalReason::HandlesDropped),
                },
                () = &mut until => return None,
            }
        }
    }

    /// One connection: handshake, restore, serve.
    async fn connection(&mut self, attempt: u32) -> Ended {
        // Commands queued since the last read apply before connecting, so the restore writes
        // them.
        while let Ok(envelope) = self.mailbox.try_recv() {
            self.apply(envelope);
        }
        self.status.update(|s| {
            s.state = FeedState::Connecting;
            s.epoch = self.epoch;
        });
        emit!(
            Level::DEBUG,
            events::WS_CONNECTING,
            feed = P::FEED.as_str(),
            epoch = self.epoch,
            attempt,
            "connecting"
        );
        if let Err(t) = self.lifecycle(Lifecycle::Connecting {
            epoch: self.epoch,
            attempt,
        }) {
            return Ended::Terminal(t);
        }
        let started = Instant::now();
        let stop = Arc::clone(&self.stop);
        let handshake = tokio::select! {
            biased;
            () = stop.wait() => return Ended::Stop,
            result = tokio::time::timeout(
                self.limits.handshake_timeout,
                handshake(self.url.clone(), self.limits.max_frame_bytes, self.connector.clone()),
            ) => result,
        };
        let mut socket = match handshake {
            Err(_) => {
                metrics::record_ws_connect_attempt(P::FEED, ConnectResult::Failed);
                Span::current().record("result", "failed");
                return Ended::Cause(Cause::HandshakeTimeout, None);
            }
            Ok(Err((cause, status))) => {
                let rejected = status.is_some();
                metrics::record_ws_connect_attempt(
                    P::FEED,
                    if rejected {
                        ConnectResult::Rejected
                    } else {
                        ConnectResult::Failed
                    },
                );
                let span = Span::current();
                span.record("result", if rejected { "rejected" } else { "failed" });
                if let Some(http_status) = status {
                    span.record("http_status", http_status);
                    let disposition = if matches!(dispose(&cause), Disposition::Retry) {
                        "retry"
                    } else {
                        "terminal"
                    };
                    if disposition == "retry" {
                        emit!(
                            Level::WARN,
                            events::WS_HANDSHAKE_REJECTED,
                            feed = P::FEED.as_str(),
                            http_status,
                            disposition,
                            "handshake rejected"
                        );
                    } else {
                        emit!(
                            Level::ERROR,
                            events::WS_HANDSHAKE_REJECTED,
                            feed = P::FEED.as_str(),
                            http_status,
                            disposition,
                            "handshake rejected"
                        );
                    }
                }
                return Ended::Cause(cause, status);
            }
            Ok(Ok(socket)) => socket,
        };
        metrics::record_ws_connect_attempt(P::FEED, ConnectResult::Connected);
        metrics::record_ws_connections_active(P::FEED, true);
        let _gauge = ConnectedGauge(P::FEED);
        let handshake_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Span::current().record("result", "connected");
        emit!(
            Level::INFO,
            events::WS_CONNECTED,
            feed = P::FEED.as_str(),
            epoch = self.epoch,
            handshake_ms,
            "connected"
        );
        if let Err(t) = self.lifecycle(Lifecycle::Connected { epoch: self.epoch }) {
            return Ended::Terminal(t);
        }
        if let Some((previous_epoch, last_seq, disconnected_at, reason)) = self.lost.take()
            && let Err(t) = self.lifecycle(Lifecycle::Gap {
                previous_epoch,
                last_seq,
                disconnected_at,
                reconnected_at: SystemTime::now(),
                reason,
            })
        {
            return Ended::Terminal(t);
        }
        let ended = self.connected(&mut socket, started).await;
        let span = Span::current();
        span.record(
            "duration_ms",
            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        );
        if let Ended::Cause(cause, _) = &ended {
            let reason = match cause {
                Cause::Disconnected(r) => r.clone(),
                Cause::ServerDisconnect(code) => DisconnectReason::ServerDisconnect { code: *code },
                Cause::GlobalError(code) => {
                    DisconnectReason::ServerDisconnect { code: Some(*code) }
                }
                _ => DisconnectReason::Transport,
            };
            span.record("disconnect_reason", reason_label(&reason).0);
            let server_code = match &reason {
                DisconnectReason::ServerDisconnect { code } => code.map(u64::from),
                _ => None,
            };
            emit!(
                Level::WARN,
                events::WS_DISCONNECTED,
                feed = P::FEED.as_str(),
                epoch = self.epoch,
                reason = reason_label(&reason).0,
                server_code,
                "connection lost"
            );
            self.lost = Some((
                self.epoch,
                self.last_data_seq,
                SystemTime::now(),
                reason.clone(),
            ));
            if let Err(t) = self.lifecycle(Lifecycle::Disconnected {
                epoch: self.epoch,
                reason,
            }) {
                return Ended::Terminal(t);
            }
        }
        ended
    }

    /// Restores the subscriptions and serves a live connection until it ends.
    async fn connected(&mut self, socket: &mut Socket, started: Instant) -> Ended {
        for message in self.protocol.on_open() {
            if let Some(end) = self.write(socket, message).await {
                return end;
            }
        }
        self.status.update(|s| s.state = FeedState::Restoring);
        self.sent.clear();
        let restore = spans::ws_restore(
            self.epoch,
            self.desired.revision().0,
            self.desired.set().len(),
            0,
        );
        let restored = self.reconcile(socket).instrument(restore.clone()).await;
        match restored {
            Ok(messages) => {
                restore.record("messages", messages);
                restore.record("result", "ok");
            }
            Err(end) => {
                restore.record("result", "error");
                return end;
            }
        }
        let revision = self.desired.revision();
        // Commands accepted while disconnected (after an earlier connection's writes) are now
        // written: report them before Active.
        let previously_sent = self.status_sent_revision;
        if previously_sent.is_some_and(|previous| previous != revision)
            && let Err(t) = self.lifecycle(Lifecycle::CommandsSent { revision })
        {
            return Ended::Terminal(t);
        }
        self.status_sent_revision = Some(revision);
        self.budget.mark_active();
        self.status.update(|s| {
            s.state = FeedState::Active;
            s.sent_revision = Some(revision);
        });
        emit!(
            Level::INFO,
            events::WS_RESTORED,
            feed = P::FEED.as_str(),
            epoch = self.epoch,
            revision = revision.0,
            instruments = self.sent.len(),
            "subscriptions restored"
        );
        if let Err(t) = self.lifecycle(Lifecycle::Active {
            epoch: self.epoch,
            revision,
        }) {
            return Ended::Terminal(t);
        }
        let _ = started;
        self.serve(socket).await
    }

    /// Writes the frames that bring the server from `sent` to `desired`; returns how many.
    async fn reconcile(&mut self, socket: &mut Socket) -> Result<usize, Ended> {
        let frames = self.protocol.reconcile(&self.sent, self.desired.set());
        let count = frames.len();
        for frame in frames {
            if let Some(end) = self.write(socket, Message::Text(frame.into())).await {
                return Err(end);
            }
        }
        self.sent = self.desired.set().clone();
        if count > 0 {
            emit!(
                Level::DEBUG,
                events::WS_SUBSCRIPTION_SENT,
                feed = P::FEED.as_str(),
                revision = self.desired.revision().0,
                messages = count,
                instruments = self.sent.len(),
                "subscriptions written"
            );
        }
        Ok(count)
    }

    /// Sends one message; a stop during the write interrupts it (the socket is then dropped).
    async fn write(&mut self, socket: &mut Socket, message: Message) -> Option<Ended> {
        let stop = Arc::clone(&self.stop);
        // The write comes first, so a write that completes is never reported as interrupted;
        // a stop cuts a write still in progress, and a stalled write ends as a liveness loss.
        let liveness = self.limits.liveness_timeout;
        let sent = tokio::select! {
            biased;
            sent = async {
                socket.feed(message).await?;
                socket.flush().await
            } => sent,
            () = stop.wait() => return Some(Ended::Terminal(TerminalReason::SendInterrupted)),
            () = tokio::time::sleep(liveness) => {
                return Some(Ended::Cause(Cause::Disconnected(DisconnectReason::LivenessTimeout), None));
            }
        };
        sent.err()
            .map(|_| Ended::Cause(Cause::Disconnected(DisconnectReason::Transport), None))
    }

    /// The serve loop of a live connection.
    async fn serve(&mut self, socket: &mut Socket) -> Ended {
        let stop = Arc::clone(&self.stop);
        let ping_every = self.protocol.client_ping_interval();
        let mut next_ping = ping_every.map(|p| Instant::now() + p);
        let mut last_frame = Instant::now();
        let mut sampler = Sampler::default();
        loop {
            let liveness = last_frame + self.limits.liveness_timeout;
            let ping_at = next_ping.unwrap_or(liveness);
            tokio::select! {
                biased;
                () = stop.wait() => return self.close(socket).await,
                envelope = self.mailbox.recv() => {
                    let Some(envelope) = envelope else {
                        return Ended::Terminal(TerminalReason::HandlesDropped);
                    };
                    let mut changed = self.apply(envelope);
                    // Drain what is queued, then reconcile once.
                    while let Ok(more) = self.mailbox.try_recv() {
                        changed |= self.apply(more);
                    }
                    if changed {
                        if let Err(end) = self.reconcile(socket).await {
                            return end;
                        }
                        let revision = self.desired.revision();
                        self.status_sent_revision = Some(revision);
                        self.status.update(|s| s.sent_revision = Some(revision));
                        if let Err(t) = self.lifecycle(Lifecycle::CommandsSent { revision }) {
                            return Ended::Terminal(t);
                        }
                    }
                }
                frame = socket.next() => {
                    let Some(frame) = frame else {
                        return Ended::Cause(Cause::Disconnected(DisconnectReason::Eof), None);
                    };
                    let frame = match frame {
                        Ok(frame) => frame,
                        // The peer dropped the TCP connection without a close frame.
                        Err(tungstenite::Error::Protocol(
                            tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
                        )) => {
                            return Ended::Cause(Cause::Disconnected(DisconnectReason::Eof), None);
                        }
                        Err(tungstenite::Error::Capacity(_) | tungstenite::Error::Protocol(_) | tungstenite::Error::Utf8(_)) => {
                            return Ended::Cause(Cause::Disconnected(DisconnectReason::Protocol), None);
                        }
                        Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                            return Ended::Cause(Cause::Disconnected(DisconnectReason::Eof), None);
                        }
                        Err(_) => return Ended::Cause(Cause::Disconnected(DisconnectReason::Transport), None),
                    };
                    last_frame = Instant::now();
                    self.status.frame_received(last_frame);
                    if let Some(end) = self.frame(frame, &mut sampler).await {
                        return end;
                    }
                }
                () = tokio::time::sleep_until(liveness) => {
                    let silent_ms = u64::try_from(last_frame.elapsed().as_millis()).unwrap_or(u64::MAX);
                    emit!(Level::WARN, events::WS_LIVENESS_TIMEOUT, feed = P::FEED.as_str(), epoch = self.epoch, silent_ms, "no frame within the liveness timeout");
                    return Ended::Cause(Cause::Disconnected(DisconnectReason::LivenessTimeout), None);
                }
                () = tokio::time::sleep_until(ping_at), if next_ping.is_some() => {
                    next_ping = ping_every.map(|p| Instant::now() + p);
                    if let Some(end) = self.write(socket, Message::Ping(Vec::new().into())).await {
                        return end;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_disconnect_reason_has_a_label() {
        let reasons = [
            (DisconnectReason::Eof, "eof"),
            (DisconnectReason::RemoteClose { code: None }, "remote_close"),
            (DisconnectReason::LivenessTimeout, "liveness_timeout"),
            (DisconnectReason::Transport, "transport"),
            (DisconnectReason::Protocol, "protocol"),
            (
                DisconnectReason::ServerDisconnect { code: Some(805) },
                "server_disconnect",
            ),
            (DisconnectReason::Shutdown, "shutdown"),
        ];
        for (reason, label) in reasons {
            assert_eq!(reason_label(&reason).0, label);
            assert_eq!(reason_label(&reason).1.as_str(), label);
        }
    }

    #[tokio::test]
    async fn stop_resolves_waiters_before_and_after_the_request() {
        let stop = Arc::new(Stop::default());
        let early = tokio::spawn({
            let stop = Arc::clone(&stop);
            async move { stop.wait().await }
        });
        tokio::task::yield_now().await;
        stop.request();
        early.await.unwrap();
        stop.wait().await;
        assert!(stop.requested());
    }
}
