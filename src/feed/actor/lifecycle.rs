//! Reconnect dispositions and the reconnect budget.
//!
//! Every way a connection attempt or a live connection can end is a [`Cause`], and [`dispose`]
//! maps each one to retry or a terminal reason, following the architecture's disposition table:
//!
//! | Cause | Disposition |
//! |---|---|
//! | handshake 401 or 403 | terminal `AuthRejected` |
//! | handshake 400 | terminal `HandshakeRejected{400}`: the query parameters were refused (DOC:1864) |
//! | handshake 429 | retry: a rate-limited connect should back off |
//! | other handshake 4xx | terminal `HandshakeRejected` |
//! | handshake 5xx or another unexpected status, connect error, TLS error, handshake timeout | retry |
//! | end of stream, remote close, protocol error, transport error, liveness timeout | retry |
//! | server disconnect 800 | retry |
//! | server disconnect 804-814 | terminal `ServerDisconnect{code}`; 805 means a newer connection took this one's place (DOC:6034), so reconnecting would evict it in turn |
//! | server disconnect with an unknown or unreadable code | retry, within the budget |
//! | global-feed error packet | as the server-disconnect rows; repeated invalid attempts can block the IP (DOC:1863) |
//! | delivery overload, receiver dropped, handles dropped, panic | terminal |
//!
//! The Python SDK reconnects every second forever, even after an 805; this SDK does not.

use std::time::Duration;

use tokio::time::Instant;

use super::super::{DisconnectReason, ReconnectPolicy, TerminalReason};
use crate::backoff::{Backoff, SplitMix64};

/// Why a connection attempt failed or a live connection ended.
#[allow(dead_code, reason = "produced by the feed owner task")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Cause {
    /// The handshake got an HTTP status other than 101.
    HandshakeStatus(u16),
    /// The TCP connection could not be opened.
    Connect,
    /// The TLS handshake failed.
    Tls,
    /// The handshake did not finish within `handshake_timeout`.
    HandshakeTimeout,
    /// A live connection ended.
    Disconnected(DisconnectReason),
    /// The server sent a disconnect packet with this code (`None` if unreadable or ambiguous).
    ServerDisconnect(Option<u16>),
    /// The global feed sent an error packet with this code.
    GlobalError(u16),
    /// The consumer stopped reading and the queues filled.
    DeliveryOverload,
    /// The event stream was dropped.
    ReceiverDropped,
    /// Every handle was dropped.
    HandlesDropped,
    /// The owner task panicked.
    Panicked,
}

/// What to do after a [`Cause`].
#[allow(dead_code, reason = "consumed by the feed owner task")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Disposition {
    /// Back off and reconnect, within the budget.
    Retry,
    /// End the feed.
    Terminal(TerminalReason),
}

/// The disposition of a server disconnect code.
fn server_code(code: Option<u16>) -> Disposition {
    match code {
        Some(800) | None => Disposition::Retry,
        Some(code @ 804..=814) => Disposition::Terminal(TerminalReason::ServerDisconnect { code }),
        Some(_) => Disposition::Retry,
    }
}

/// Maps a cause to its disposition.
#[allow(dead_code, reason = "used by the feed owner task")]
pub(crate) fn dispose(cause: &Cause) -> Disposition {
    match cause {
        Cause::HandshakeStatus(http_status @ (401 | 403)) => {
            Disposition::Terminal(TerminalReason::AuthRejected {
                http_status: *http_status,
            })
        }
        Cause::HandshakeStatus(429) => Disposition::Retry,
        Cause::HandshakeStatus(http_status @ 400..=499) => {
            Disposition::Terminal(TerminalReason::HandshakeRejected {
                http_status: *http_status,
            })
        }
        Cause::HandshakeStatus(_) | Cause::Connect | Cause::Tls | Cause::HandshakeTimeout => {
            Disposition::Retry
        }
        Cause::Disconnected(reason) => match reason {
            DisconnectReason::Eof
            | DisconnectReason::RemoteClose { .. }
            | DisconnectReason::LivenessTimeout
            | DisconnectReason::Transport
            | DisconnectReason::Protocol => Disposition::Retry,
            DisconnectReason::ServerDisconnect { code } => server_code(*code),
            DisconnectReason::Shutdown => Disposition::Terminal(TerminalReason::Shutdown),
        },
        Cause::ServerDisconnect(code) => server_code(*code),
        Cause::GlobalError(code) => server_code(Some(*code)),
        Cause::DeliveryOverload => Disposition::Terminal(TerminalReason::DeliveryOverload),
        Cause::ReceiverDropped => Disposition::Terminal(TerminalReason::ReceiverDropped),
        Cause::HandlesDropped => Disposition::Terminal(TerminalReason::HandlesDropped),
        Cause::Panicked => Disposition::Terminal(TerminalReason::Panicked),
    }
}

/// The reconnection budget of one outage: an attempt count, a deadline and the backoff.
///
/// The count and deadline reset only after a connection reached `Active` and was later lost; a
/// connection that fails before becoming active keeps spending the same budget.
#[allow(dead_code, reason = "owned by the feed owner task")]
#[derive(Debug)]
pub(crate) struct ReconnectBudget {
    policy: ReconnectPolicy,
    backoff: Backoff,
    attempts: u32,
    outage_started: Option<Instant>,
    active: bool,
}

#[allow(dead_code, reason = "used by the feed owner task")]
impl ReconnectBudget {
    /// A fresh budget for `policy` (seeded from `jitter_seed`, or from entropy).
    pub(crate) fn new(policy: ReconnectPolicy) -> Self {
        let rng = policy
            .jitter_seed
            .map_or_else(SplitMix64::from_entropy, SplitMix64::new);
        ReconnectBudget {
            backoff: Backoff::new(policy.initial_backoff, policy.max_backoff, rng),
            policy,
            attempts: 0,
            outage_started: None,
            active: false,
        }
    }

    /// Records a failure at `now` and returns the delay before the next attempt, or `None` when
    /// the attempts or the outage deadline are used up.
    pub(crate) fn next_delay(&mut self, now: Instant) -> Option<Duration> {
        let started = *self.outage_started.get_or_insert(now);
        if self.attempts >= self.policy.max_attempts
            || now.saturating_duration_since(started) >= self.policy.outage_deadline
        {
            return None;
        }
        self.attempts += 1;
        // Never wait past the outage deadline.
        let left = self.policy.outage_deadline - now.saturating_duration_since(started);
        Some(self.backoff.delay(self.attempts).min(left))
    }

    /// How long the current outage has lasted at `now`, if one is in progress.
    pub(crate) fn outage_elapsed(&self, now: Instant) -> Option<Duration> {
        self.outage_started
            .map(|at| now.saturating_duration_since(at))
    }

    /// Reconnect attempts spent in the current outage.
    pub(crate) fn attempts(&self) -> u32 {
        self.attempts
    }

    /// The connection reached `Active`.
    pub(crate) fn mark_active(&mut self) {
        self.active = true;
    }

    /// The connection was lost; if it had been active, a new outage (and budget) starts.
    pub(crate) fn mark_lost(&mut self) {
        if self.active {
            self.active = false;
            self.attempts = 0;
            self.outage_started = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminal(reason: TerminalReason) -> Disposition {
        Disposition::Terminal(reason)
    }

    /// One representative of every cause, with its expected disposition.
    fn table() -> Vec<(Cause, Disposition)> {
        use Disposition::Retry;
        let mut rows = vec![
            (
                Cause::HandshakeStatus(401),
                terminal(TerminalReason::AuthRejected { http_status: 401 }),
            ),
            (
                Cause::HandshakeStatus(403),
                terminal(TerminalReason::AuthRejected { http_status: 403 }),
            ),
            (
                Cause::HandshakeStatus(400),
                terminal(TerminalReason::HandshakeRejected { http_status: 400 }),
            ),
            (Cause::HandshakeStatus(429), Retry),
            (
                Cause::HandshakeStatus(404),
                terminal(TerminalReason::HandshakeRejected { http_status: 404 }),
            ),
            (Cause::HandshakeStatus(500), Retry),
            (Cause::HandshakeStatus(503), Retry),
            (Cause::HandshakeStatus(200), Retry),
            (Cause::HandshakeStatus(302), Retry),
            (Cause::Connect, Retry),
            (Cause::Tls, Retry),
            (Cause::HandshakeTimeout, Retry),
            (Cause::Disconnected(DisconnectReason::Eof), Retry),
            (
                Cause::Disconnected(DisconnectReason::RemoteClose { code: Some(1000) }),
                Retry,
            ),
            (
                Cause::Disconnected(DisconnectReason::RemoteClose { code: None }),
                Retry,
            ),
            (
                Cause::Disconnected(DisconnectReason::LivenessTimeout),
                Retry,
            ),
            (Cause::Disconnected(DisconnectReason::Transport), Retry),
            (Cause::Disconnected(DisconnectReason::Protocol), Retry),
            (
                Cause::Disconnected(DisconnectReason::ServerDisconnect { code: Some(805) }),
                terminal(TerminalReason::ServerDisconnect { code: 805 }),
            ),
            (
                Cause::Disconnected(DisconnectReason::ServerDisconnect { code: None }),
                Retry,
            ),
            (
                Cause::Disconnected(DisconnectReason::Shutdown),
                terminal(TerminalReason::Shutdown),
            ),
            (Cause::ServerDisconnect(None), Retry),
            (Cause::ServerDisconnect(Some(999)), Retry),
            (
                Cause::DeliveryOverload,
                terminal(TerminalReason::DeliveryOverload),
            ),
            (
                Cause::ReceiverDropped,
                terminal(TerminalReason::ReceiverDropped),
            ),
            (
                Cause::HandlesDropped,
                terminal(TerminalReason::HandlesDropped),
            ),
            (Cause::Panicked, terminal(TerminalReason::Panicked)),
        ];
        // Every code 800..=814, for both the market/depth packet and the global error packet.
        for code in 800..=814u16 {
            let expected = match code {
                800..=803 => Retry,
                _ => terminal(TerminalReason::ServerDisconnect { code }),
            };
            rows.push((Cause::ServerDisconnect(Some(code)), expected.clone()));
            rows.push((Cause::GlobalError(code), expected));
        }
        rows
    }

    #[test]
    fn the_disposition_table_is_total() {
        let rows = table();
        // Every variant appears (the match fails to compile when a variant is added).
        let mut seen = [false; 11];
        let mut reasons_seen = [false; 7];
        for (cause, _) in &rows {
            let i = match cause {
                Cause::HandshakeStatus(_) => 0,
                Cause::Connect => 1,
                Cause::Tls => 2,
                Cause::HandshakeTimeout => 3,
                Cause::Disconnected(reason) => {
                    // Every disconnect reason appears too.
                    reasons_seen[match reason {
                        DisconnectReason::Eof => 0,
                        DisconnectReason::RemoteClose { .. } => 1,
                        DisconnectReason::LivenessTimeout => 2,
                        DisconnectReason::Transport => 3,
                        DisconnectReason::Protocol => 4,
                        DisconnectReason::ServerDisconnect { .. } => 5,
                        DisconnectReason::Shutdown => 6,
                    }] = true;
                    4
                }
                Cause::ServerDisconnect(_) => 5,
                Cause::GlobalError(_) => 6,
                Cause::DeliveryOverload => 7,
                Cause::ReceiverDropped => 8,
                Cause::HandlesDropped => 9,
                Cause::Panicked => 10,
            };
            seen[i] = true;
        }
        assert!(seen.iter().all(|s| *s), "{seen:?}");
        assert!(reasons_seen.iter().all(|s| *s), "{reasons_seen:?}");
        for (cause, expected) in rows {
            assert_eq!(dispose(&cause), expected, "{cause:?}");
        }
    }

    fn policy(max_attempts: u32, outage_secs: u64) -> ReconnectPolicy {
        ReconnectPolicy {
            max_attempts,
            outage_deadline: Duration::from_secs(outage_secs),
            initial_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(30),
            jitter_seed: Some(7),
        }
    }

    #[test]
    fn attempts_run_out() {
        let now = Instant::now();
        let mut budget = ReconnectBudget::new(policy(2, 300));
        let first = budget.next_delay(now).unwrap();
        let second = budget.next_delay(now).unwrap();
        assert!(first <= Duration::from_millis(500) && second <= Duration::from_secs(1));
        assert_eq!(budget.next_delay(now), None);
        assert_eq!(budget.attempts(), 2);
    }

    #[test]
    fn the_outage_deadline_ends_the_budget_early() {
        let start = Instant::now();
        let mut budget = ReconnectBudget::new(policy(10, 10));
        assert!(budget.next_delay(start).is_some());
        // One second is left, so the delay is clamped to it.
        let late = budget.next_delay(start + Duration::from_secs(9)).unwrap();
        assert!(late <= Duration::from_secs(1), "{late:?}");
        assert_eq!(budget.next_delay(start + Duration::from_secs(10)), None);
        assert_eq!(budget.attempts(), 2);
    }

    #[test]
    fn only_losing_an_active_connection_resets_the_budget() {
        let now = Instant::now();
        let mut budget = ReconnectBudget::new(policy(2, 300));
        budget.next_delay(now);
        budget.next_delay(now);
        // Lost before becoming active: the same outage continues.
        budget.mark_lost();
        assert_eq!(budget.next_delay(now), None);
        budget.mark_active();
        budget.mark_lost();
        assert_eq!(budget.attempts(), 0);
        assert!(budget.next_delay(now + Duration::from_secs(1000)).is_some());
    }
}
