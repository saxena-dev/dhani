//! Event-name constants and the `emit!` macro wrapper.
//!
//! Every event carries a constant `event` field naming it and is emitted under a fixed target;
//! the human-readable message may change between releases, so match on the `event` field.
//! Levels are chosen at each call site, and some events are emitted at two levels, as listed in
//! [`EVENT_CATALOGUE`].

use super::{TARGET_AUTH, TARGET_DECODE, TARGET_HTTP, TARGET_RATELIMIT, TARGET_WS};

/// One catalogued event: the value of its `event` field and its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    /// The value of the `event` field.
    pub name: &'static str,
    /// The `tracing` target.
    pub target: &'static str,
}

/// Declares one `Event` constant per `(NAME = "name", TARGET)` row and `EVENT_CATALOGUE`.
macro_rules! events {
    ($($(#[doc = $doc:literal])* $konst:ident = $name:literal, $target:ident;)+) => {
        $(
            $(#[doc = $doc])*
            pub const $konst: Event = Event { name: $name, target: $target };
        )+

        /// Every event the crate emits.
        pub const EVENT_CATALOGUE: &[Event] = &[$($konst),+];
    };
}

events! {
    /// A request succeeded and its body decoded (DEBUG).
    HTTP_REQUEST_COMPLETED = "http.request.completed", TARGET_HTTP;
    /// A request failed after reaching the pipeline (level by error kind).
    HTTP_REQUEST_FAILED = "http.request.failed", TARGET_HTTP;
    /// A request was rejected locally before sending: validation, config or credential (DEBUG).
    HTTP_REQUEST_REJECTED = "http.request.rejected", TARGET_HTTP;
    /// A retry is about to back off (WARN).
    HTTP_RETRY_SCHEDULED = "http.retry.scheduled", TARGET_HTTP;
    /// Admission was granted after a wait (DEBUG).
    RATELIMIT_WAITED = "ratelimit.waited", TARGET_RATELIMIT;
    /// The local limiter refused a request (WARN).
    RATELIMIT_REFUSED = "ratelimit.refused", TARGET_RATELIMIT;
    /// The broker rate-limited a request (WARN).
    RATELIMIT_REMOTE = "ratelimit.remote", TARGET_RATELIMIT;
    /// An access token was issued and decoded (INFO).
    AUTH_TOKEN_ISSUED = "auth.token.issued", TARGET_AUTH;
    /// A feed is about to perform its handshake (DEBUG).
    WS_CONNECTING = "ws.connecting", TARGET_WS;
    /// A feed handshake succeeded (INFO).
    WS_CONNECTED = "ws.connected", TARGET_WS;
    /// A feed's subscriptions were restored and it is active (INFO).
    WS_RESTORED = "ws.restored", TARGET_WS;
    /// A live connection ended other than on shutdown (WARN).
    WS_DISCONNECTED = "ws.disconnected", TARGET_WS;
    /// The server sent a disconnect packet (WARN if retryable, ERROR if terminal).
    WS_SERVER_DISCONNECT = "ws.server_disconnect", TARGET_WS;
    /// The handshake was rejected (WARN if retryable, ERROR if terminal).
    WS_HANDSHAKE_REJECTED = "ws.handshake.rejected", TARGET_WS;
    /// A reconnect backoff began (INFO).
    WS_RECONNECT_SCHEDULED = "ws.reconnect.scheduled", TARGET_WS;
    /// The reconnect budget is exhausted (ERROR).
    WS_RECONNECT_EXHAUSTED = "ws.reconnect.exhausted", TARGET_WS;
    /// No inbound frame arrived within the liveness window (WARN).
    WS_LIVENESS_TIMEOUT = "ws.liveness.timeout", TARGET_WS;
    /// Subscription messages were written (DEBUG).
    WS_SUBSCRIPTION_SENT = "ws.subscription.sent", TARGET_WS;
    /// A subscription command was refused locally (DEBUG).
    WS_SUBSCRIPTION_REJECTED = "ws.subscription.rejected", TARGET_WS;
    /// The delivery queue dropped its oldest items (WARN, at most once per second per feed).
    WS_OVERFLOW_DROPPED = "ws.overflow.dropped", TARGET_WS;
    /// The delivery queue overflowed under the failing policy (ERROR).
    WS_OVERFLOW_TERMINAL = "ws.overflow.terminal", TARGET_WS;
    /// A frame or packet failed to decode (WARN, sampled).
    WS_DECODE_FAILED = "ws.decode.failed", TARGET_DECODE;
    /// A packet carried an unknown response code (WARN, sampled).
    WS_UNKNOWN_PACKET = "ws.unknown_packet", TARGET_DECODE;
    /// A feed actor ended (INFO).
    WS_STOPPED = "ws.stopped", TARGET_WS;
}

/// Emits a catalogued event: `emit!(Level::WARN, events::WS_DISCONNECTED, feed = .., "message")`.
///
/// The target and the `event` field come from the [`Event`] constant, so neither can drift from
/// the catalogue. The level must be a constant (a `tracing` requirement).
#[cfg_attr(
    not(test),
    allow(
        unused_macros,
        reason = "used by the transport, limiter and feed instrumentation"
    )
)]
macro_rules! emit {
    ($level:expr, $event:expr $(,)?) => {
        ::tracing::event!(target: $event.target, $level, event = $event.name)
    };
    ($level:expr, $event:expr, $($rest:tt)*) => {
        ::tracing::event!(target: $event.target, $level, event = $event.name, $($rest)*)
    };
}
#[allow(
    unused_imports,
    reason = "used by the transport, limiter and feed instrumentation; calls in this module \
              resolve textually"
)]
pub(crate) use emit;

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context, SubscriberExt};

    use super::*;

    #[test]
    fn the_catalogue_has_twenty_four_distinct_events_on_known_targets() {
        assert_eq!(EVENT_CATALOGUE.len(), 24);
        let mut names: Vec<_> = EVENT_CATALOGUE.iter().map(|e| e.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 24);
        for event in EVENT_CATALOGUE {
            assert!(
                super::super::TARGETS.contains(&event.target),
                "{}",
                event.name
            );
        }
        assert_eq!(
            (
                WS_DECODE_FAILED.target,
                WS_UNKNOWN_PACKET.target,
                AUTH_TOKEN_ISSUED.target
            ),
            ("dhani::decode", "dhani::decode", "dhani::auth")
        );
    }

    /// `(target, level, event field, other field names)` of one event.
    type Seen = (String, tracing::Level, String, Vec<String>);

    /// Records every event.
    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Vec<Seen>>>);

    struct Fields(String, Vec<String>);

    impl Visit for Fields {
        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "event" {
                self.0 = value.to_owned();
            } else {
                self.1.push(field.name().to_owned());
            }
        }

        fn record_debug(&mut self, field: &Field, _: &dyn std::fmt::Debug) {
            self.1.push(field.name().to_owned());
        }
    }

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Recorder {
        fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
            let mut fields = Fields(String::new(), Vec::new());
            event.record(&mut fields);
            let meta = event.metadata();
            self.0.lock().unwrap().push((
                meta.target().to_owned(),
                *meta.level(),
                fields.0,
                fields.1,
            ));
        }
    }

    #[test]
    fn emit_sets_the_target_and_event_field() {
        let recorder = Recorder::default();
        let subscriber = tracing_subscriber::registry().with(recorder.clone());
        tracing::subscriber::with_default(subscriber, || {
            emit!(
                tracing::Level::WARN,
                WS_DISCONNECTED,
                feed = "market",
                epoch = 2u64,
                "disconnected"
            );
            emit!(tracing::Level::INFO, WS_STOPPED);
        });
        let seen = recorder.0.lock().unwrap();
        assert_eq!(seen.len(), 2);
        let (target, level, event, fields) = &seen[0];
        assert_eq!(
            (target.as_str(), *level, event.as_str()),
            ("dhani::ws", tracing::Level::WARN, "ws.disconnected")
        );
        assert_eq!(fields, &["message", "feed", "epoch"]);
        let (target, level, event, fields) = &seen[1];
        assert_eq!(
            (target.as_str(), *level, event.as_str()),
            ("dhani::ws", tracing::Level::INFO, "ws.stopped")
        );
        assert!(fields.is_empty());
    }
}
