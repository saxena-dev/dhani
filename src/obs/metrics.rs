//! Metric name constants and record functions; the bodies are no-ops unless the `metrics`
//! feature is enabled.
//!
//! Metrics go through the `metrics` facade: with the feature on and no recorder installed they
//! cost nothing, and with the feature off every record function is an empty inline function.
//! Label values are `&'static str` from closed enums, so the number of series is bounded; see
//! [`SERIES_BOUND`]. No label ever carries a client ID, token, order ID, security ID, URL or
//! broker text. Durations are recorded in seconds.

#![cfg_attr(
    not(all(test, feature = "metrics")),
    allow(
        dead_code,
        reason = "record functions are called by the transport, limiter and feed instrumentation; \
                  label strings are only read when the metrics feature is on"
    )
)]

use std::time::Duration;

use crate::error::ErrorKind;
use crate::labels::{EndpointId, FeedKind, RateClass};

/// Requests finished, by endpoint and outcome.
pub const HTTP_REQUESTS_TOTAL: &str = "dhani_http_requests_total";
/// Request duration in seconds (admission, attempts and backoff), by endpoint and outcome.
pub const HTTP_REQUEST_DURATION_SECONDS: &str = "dhani_http_request_duration_seconds";
/// Attempts finished, by endpoint and result.
pub const HTTP_ATTEMPTS_TOTAL: &str = "dhani_http_attempts_total";
/// Attempt duration in seconds (dispatch to classification), by endpoint.
pub const HTTP_ATTEMPT_DURATION_SECONDS: &str = "dhani_http_attempt_duration_seconds";
/// Retries scheduled, by endpoint and cause.
pub const HTTP_RETRIES_TOTAL: &str = "dhani_http_retries_total";
/// Admission wait in seconds for grants that waited, by rate class.
pub const RATELIMIT_WAIT_SECONDS: &str = "dhani_ratelimit_wait_seconds";
/// Local refusals and remote rate-limit errors, by rate class and source.
pub const RATELIMIT_REJECTIONS_TOTAL: &str = "dhani_ratelimit_rejections_total";
/// Connected feeds, by feed.
pub const WS_CONNECTIONS_ACTIVE: &str = "dhani_ws_connections_active";
/// Connect attempts, by feed and result.
pub const WS_CONNECT_ATTEMPTS_TOTAL: &str = "dhani_ws_connect_attempts_total";
/// Reconnect attempts, by feed and the disconnect reason that caused them.
pub const WS_RECONNECTS_TOTAL: &str = "dhani_ws_reconnects_total";
/// Server disconnect packets, by feed and code.
pub const WS_SERVER_DISCONNECTS_TOTAL: &str = "dhani_ws_server_disconnects_total";
/// Data frames received, by feed and frame kind.
pub const WS_FRAMES_TOTAL: &str = "dhani_ws_frames_total";
/// Bytes of data frames received, by feed.
pub const WS_BYTES_TOTAL: &str = "dhani_ws_bytes_total";
/// Packets decoded, by feed and packet kind.
pub const WS_PACKETS_TOTAL: &str = "dhani_ws_packets_total";
/// Decode failures (not sampled), by feed and failure kind.
pub const WS_DECODE_FAILURES_TOTAL: &str = "dhani_ws_decode_failures_total";
/// Items waiting in the delivery queue, by feed.
pub const WS_QUEUE_DEPTH: &str = "dhani_ws_queue_depth";
/// Items dropped by the drop-oldest overflow policy, by feed.
pub const WS_DROPPED_TOTAL: &str = "dhani_ws_dropped_total";

/// Declares a closed label enum with `ALL` and `as_str`.
macro_rules! label_enum {
    ($(#[doc = $doc:literal])* $name:ident { $($variant:ident => $label:literal,)+ }) => {
        $(#[doc = $doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub(crate) enum $name {
            $($variant,)+
        }

        impl $name {
            pub(crate) const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub(crate) fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $label,)+
                }
            }
        }
    };
}

label_enum! {
    /// How one HTTP attempt ended.
    AttemptResult {
        Ok => "ok",
        Http4xx => "http_4xx",
        Http5xx => "http_5xx",
        Timeout => "timeout",
        Transport => "transport",
        Decode => "decode",
    }
}

label_enum! {
    /// Why a retry was scheduled.
    RetryCause {
        Status502 => "status_502",
        Status503 => "status_503",
        Status504 => "status_504",
        Timeout => "timeout",
        Transport => "transport",
        RateLimited => "rate_limited",
    }
}

label_enum! {
    /// Where a rate-limit rejection came from.
    RejectionSource {
        WaitExceeded => "wait_exceeded",
        Ceiling => "ceiling",
        WaitersFull => "waiters_full",
        Remote => "remote",
    }
}

label_enum! {
    /// How a feed connect attempt ended.
    ConnectResult {
        Connected => "connected",
        Rejected => "rejected",
        Failed => "failed",
    }
}

label_enum! {
    /// The disconnect reason behind a reconnect (the feed's `DisconnectReason`, snake_case).
    ReconnectReason {
        Eof => "eof",
        RemoteClose => "remote_close",
        LivenessTimeout => "liveness_timeout",
        Transport => "transport",
        Protocol => "protocol",
        ServerDisconnect => "server_disconnect",
        Shutdown => "shutdown",
    }
}

label_enum! {
    /// A received data frame's kind.
    FrameKind {
        Binary => "binary",
        Text => "text",
    }
}

label_enum! {
    /// A decoded packet's kind.
    PacketKind {
        Ticker => "ticker",
        PrevClose => "prev_close",
        Quote => "quote",
        OpenInterest => "open_interest",
        Full => "full",
        Disconnect => "disconnect",
        Other => "other",
        DepthBid => "depth_bid",
        DepthAsk => "depth_ask",
        GlobalTrade => "global_trade",
        GlobalPrevClose => "global_prev_close",
        GlobalCircuitLimit => "global_circuit_limit",
        GlobalWeek52 => "global_week52",
        OrderUpdate => "order_update",
    }
}

label_enum! {
    /// A decode failure's kind (the decoder's `DecodeErrorKind`, snake_case).
    DecodeFailure {
        Truncated => "truncated",
        BadLength => "bad_length",
        UnknownCode => "unknown_code",
        TrailingBytes => "trailing_bytes",
        LengthMismatch => "length_mismatch",
        RowCountMismatch => "row_count_mismatch",
        AmbiguousDisconnect => "ambiguous_disconnect",
        UnexpectedBinary => "unexpected_binary",
        UnexpectedText => "unexpected_text",
        Json => "json",
    }
}

/// Every `ErrorKind`, for the size of the outcome label domain.
const ERROR_KINDS: [ErrorKind; 10] = [
    ErrorKind::Config,
    ErrorKind::Credential,
    ErrorKind::Validation,
    ErrorKind::RateLimited,
    ErrorKind::Timeout,
    ErrorKind::Transport,
    ErrorKind::Auth,
    ErrorKind::Api,
    ErrorKind::HttpStatus,
    ErrorKind::Decode,
];

/// Label values of the server-disconnect `code` label: `800` to `814`, then `other`.
const SERVER_CODES: [&str; 16] = [
    "800", "801", "802", "803", "804", "805", "806", "807", "808", "809", "810", "811", "812",
    "813", "814", "other",
];

const ENDPOINTS: usize = EndpointId::ALL.len();
const OUTCOMES: usize = ERROR_KINDS.len() + 1;
const FEEDS: usize = 5;
const RATE_CLASSES: usize = 6;

/// Upper bound on the number of series of each metric: the product of its label domain sizes.
pub const SERIES_BOUND: &[(&str, usize)] = &[
    (HTTP_REQUESTS_TOTAL, ENDPOINTS * OUTCOMES),
    (HTTP_REQUEST_DURATION_SECONDS, ENDPOINTS * OUTCOMES),
    (HTTP_ATTEMPTS_TOTAL, ENDPOINTS * AttemptResult::ALL.len()),
    (HTTP_ATTEMPT_DURATION_SECONDS, ENDPOINTS),
    (HTTP_RETRIES_TOTAL, ENDPOINTS * RetryCause::ALL.len()),
    (RATELIMIT_WAIT_SECONDS, RATE_CLASSES),
    (
        RATELIMIT_REJECTIONS_TOTAL,
        RATE_CLASSES * RejectionSource::ALL.len(),
    ),
    (WS_CONNECTIONS_ACTIVE, FEEDS),
    (WS_CONNECT_ATTEMPTS_TOTAL, FEEDS * ConnectResult::ALL.len()),
    (WS_RECONNECTS_TOTAL, FEEDS * ReconnectReason::ALL.len()),
    (WS_SERVER_DISCONNECTS_TOTAL, FEEDS * SERVER_CODES.len()),
    (WS_FRAMES_TOTAL, FEEDS * FrameKind::ALL.len()),
    (WS_BYTES_TOTAL, FEEDS),
    (WS_PACKETS_TOTAL, FEEDS * PacketKind::ALL.len()),
    (WS_DECODE_FAILURES_TOTAL, FEEDS * DecodeFailure::ALL.len()),
    (WS_QUEUE_DEPTH, FEEDS),
    (WS_DROPPED_TOTAL, FEEDS),
];

fn outcome(kind: Option<ErrorKind>) -> &'static str {
    kind.map_or("ok", ErrorKind::as_str)
}

fn server_code(code: u16) -> &'static str {
    match code {
        800..=814 => SERVER_CODES[usize::from(code - 800)],
        _ => SERVER_CODES[15],
    }
}

/// Expands to the body of a record function: the facade call when the feature is on, and a use
/// of every argument when it is off.
macro_rules! record {
    ([$($arg:ident),*] $body:block) => {
        #[cfg(feature = "metrics")]
        $body
        #[cfg(not(feature = "metrics"))]
        let _ = ($($arg,)*);
    };
}

/// A request finished with `kind` (`None` for success) after `elapsed`.
#[inline(always)]
pub(crate) fn record_http_request(
    endpoint: EndpointId,
    kind: Option<ErrorKind>,
    elapsed: Duration,
) {
    record!([endpoint, kind, elapsed] {
        let labels = [("endpoint", endpoint.as_str()), ("outcome", outcome(kind))];
        metrics::counter!(HTTP_REQUESTS_TOTAL, &labels).increment(1);
        metrics::histogram!(HTTP_REQUEST_DURATION_SECONDS, &labels).record(elapsed.as_secs_f64());
    });
}

/// An attempt finished with `result` after `elapsed`.
#[inline(always)]
pub(crate) fn record_http_attempt(endpoint: EndpointId, result: AttemptResult, elapsed: Duration) {
    record!([endpoint, result, elapsed] {
        metrics::counter!(HTTP_ATTEMPTS_TOTAL, "endpoint" => endpoint.as_str(), "result" => result.as_str())
            .increment(1);
        metrics::histogram!(HTTP_ATTEMPT_DURATION_SECONDS, "endpoint" => endpoint.as_str())
            .record(elapsed.as_secs_f64());
    });
}

/// A retry was scheduled for `cause`.
#[inline(always)]
pub(crate) fn record_http_retry(endpoint: EndpointId, cause: RetryCause) {
    record!([endpoint, cause] {
        metrics::counter!(HTTP_RETRIES_TOTAL, "endpoint" => endpoint.as_str(), "cause" => cause.as_str())
            .increment(1);
    });
}

/// A grant was issued after waiting `waited`.
#[inline(always)]
pub(crate) fn record_ratelimit_wait(class: RateClass, waited: Duration) {
    record!([class, waited] {
        metrics::histogram!(RATELIMIT_WAIT_SECONDS, "rate_class" => class.as_str())
            .record(waited.as_secs_f64());
    });
}

/// A request was refused locally, or rate-limited by the broker.
#[inline(always)]
pub(crate) fn record_ratelimit_rejection(class: RateClass, source: RejectionSource) {
    record!([class, source] {
        metrics::counter!(RATELIMIT_REJECTIONS_TOTAL, "rate_class" => class.as_str(), "source" => source.as_str())
            .increment(1);
    });
}

/// A feed connected (`connected = true`) or its connection ended (`false`).
#[inline(always)]
pub(crate) fn record_ws_connections_active(feed: FeedKind, connected: bool) {
    record!([feed, connected] {
        let delta = if connected { 1.0 } else { -1.0 };
        metrics::gauge!(WS_CONNECTIONS_ACTIVE, "feed" => feed.as_str()).increment(delta);
    });
}

/// A connect attempt ended with `result`.
#[inline(always)]
pub(crate) fn record_ws_connect_attempt(feed: FeedKind, result: ConnectResult) {
    record!([feed, result] {
        metrics::counter!(WS_CONNECT_ATTEMPTS_TOTAL, "feed" => feed.as_str(), "result" => result.as_str())
            .increment(1);
    });
}

/// A reconnect attempt started after a disconnect for `reason`.
#[inline(always)]
pub(crate) fn record_ws_reconnect(feed: FeedKind, reason: ReconnectReason) {
    record!([feed, reason] {
        metrics::counter!(WS_RECONNECTS_TOTAL, "feed" => feed.as_str(), "reason" => reason.as_str())
            .increment(1);
    });
}

/// A server disconnect packet with `code` arrived.
#[inline(always)]
pub(crate) fn record_ws_server_disconnect(feed: FeedKind, code: u16) {
    record!([feed, code] {
        metrics::counter!(WS_SERVER_DISCONNECTS_TOTAL, "feed" => feed.as_str(), "code" => server_code(code))
            .increment(1);
    });
}

/// A data frame of `bytes` bytes arrived.
#[inline(always)]
pub(crate) fn record_ws_frame(feed: FeedKind, kind: FrameKind, bytes: usize) {
    record!([feed, kind, bytes] {
        metrics::counter!(WS_FRAMES_TOTAL, "feed" => feed.as_str(), "kind" => kind.as_str())
            .increment(1);
        metrics::counter!(WS_BYTES_TOTAL, "feed" => feed.as_str())
            .increment(u64::try_from(bytes).unwrap_or(u64::MAX));
    });
}

/// A packet of `packet` kind was decoded.
#[inline(always)]
pub(crate) fn record_ws_packet(feed: FeedKind, packet: PacketKind) {
    record!([feed, packet] {
        metrics::counter!(WS_PACKETS_TOTAL, "feed" => feed.as_str(), "packet" => packet.as_str())
            .increment(1);
    });
}

/// A frame or packet failed to decode.
#[inline(always)]
pub(crate) fn record_ws_decode_failure(feed: FeedKind, kind: DecodeFailure) {
    record!([feed, kind] {
        metrics::counter!(WS_DECODE_FAILURES_TOTAL, "feed" => feed.as_str(), "kind" => kind.as_str())
            .increment(1);
    });
}

/// The delivery queue now holds `depth` items.
#[inline(always)]
pub(crate) fn record_ws_queue_depth(feed: FeedKind, depth: usize) {
    record!([feed, depth] {
        metrics::gauge!(WS_QUEUE_DEPTH, "feed" => feed.as_str()).set(depth as f64);
    });
}

/// The drop-oldest policy dropped `dropped` items.
#[inline(always)]
pub(crate) fn record_ws_dropped(feed: FeedKind, dropped: u64) {
    record!([feed, dropped] {
        metrics::counter!(WS_DROPPED_TOTAL, "feed" => feed.as_str()).increment(dropped);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bound(name: &str) -> usize {
        SERIES_BOUND
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, b)| *b)
            .unwrap()
    }

    #[test]
    fn series_bounds_are_the_products_of_the_label_domains() {
        assert_eq!(SERIES_BOUND.len(), 17);
        assert_eq!(bound(HTTP_REQUESTS_TOTAL), 75 * 11);
        assert_eq!(bound(HTTP_REQUEST_DURATION_SECONDS), 75 * 11);
        assert_eq!(bound(HTTP_ATTEMPTS_TOTAL), 75 * 6);
        assert_eq!(bound(HTTP_ATTEMPT_DURATION_SECONDS), 75);
        assert_eq!(bound(HTTP_RETRIES_TOTAL), 75 * 6);
        assert_eq!(bound(RATELIMIT_WAIT_SECONDS), 6);
        assert_eq!(bound(RATELIMIT_REJECTIONS_TOTAL), 6 * 4);
        assert_eq!(bound(WS_CONNECTIONS_ACTIVE), 5);
        assert_eq!(bound(WS_CONNECT_ATTEMPTS_TOTAL), 5 * 3);
        assert_eq!(bound(WS_RECONNECTS_TOTAL), 5 * 7);
        assert_eq!(bound(WS_SERVER_DISCONNECTS_TOTAL), 5 * 16);
        assert_eq!(bound(WS_FRAMES_TOTAL), 5 * 2);
        assert_eq!(bound(WS_BYTES_TOTAL), 5);
        assert_eq!(bound(WS_PACKETS_TOTAL), 5 * 14);
        assert_eq!(bound(WS_DECODE_FAILURES_TOTAL), 5 * 10);
        assert_eq!(bound(WS_QUEUE_DEPTH), 5);
        assert_eq!(bound(WS_DROPPED_TOTAL), 5);
    }

    #[test]
    fn label_domains_are_complete_and_distinct() {
        // Exhaustive matches: adding a variant to either enum fails to compile until the domain
        // size above is revisited.
        for kind in ERROR_KINDS {
            match kind {
                ErrorKind::Config
                | ErrorKind::Credential
                | ErrorKind::Validation
                | ErrorKind::RateLimited
                | ErrorKind::Timeout
                | ErrorKind::Transport
                | ErrorKind::Auth
                | ErrorKind::Api
                | ErrorKind::HttpStatus
                | ErrorKind::Decode => {}
            }
        }
        for class in [
            RateClass::Order,
            RateClass::Data,
            RateClass::Quote,
            RateClass::NonTrading,
            RateClass::TokenGeneration,
            RateClass::Unmetered,
        ] {
            match class {
                RateClass::Order
                | RateClass::Data
                | RateClass::Quote
                | RateClass::NonTrading
                | RateClass::TokenGeneration
                | RateClass::Unmetered => {}
            }
        }
        let feeds = [
            FeedKind::Market,
            FeedKind::Depth20,
            FeedKind::Depth200,
            FeedKind::OrderUpdate,
            FeedKind::Global,
        ];
        for feed in feeds {
            match feed {
                FeedKind::Market
                | FeedKind::Depth20
                | FeedKind::Depth200
                | FeedKind::OrderUpdate
                | FeedKind::Global => {}
            }
        }
        assert_eq!(feeds.len(), FEEDS);
        let mut outcomes: Vec<_> = ERROR_KINDS.iter().map(|k| outcome(Some(*k))).collect();
        outcomes.push(outcome(None));
        outcomes.sort_unstable();
        outcomes.dedup();
        assert_eq!(outcomes.len(), OUTCOMES);
        assert_eq!(
            (
                server_code(800),
                server_code(805),
                server_code(814),
                server_code(815),
                server_code(0)
            ),
            ("800", "805", "814", "other", "other")
        );
    }

    #[test]
    fn record_functions_accept_every_label() {
        // With the feature off these are no-ops; with it on and no recorder, facade no-ops.
        for endpoint in EndpointId::ALL {
            record_http_request(*endpoint, None, Duration::from_millis(5));
        }
        for result in AttemptResult::ALL {
            record_http_attempt(EndpointId::OrdersList, *result, Duration::from_millis(5));
        }
        for cause in RetryCause::ALL {
            record_http_retry(EndpointId::OrdersList, *cause);
        }
        record_ratelimit_wait(RateClass::Quote, Duration::from_millis(900));
        for source in RejectionSource::ALL {
            record_ratelimit_rejection(RateClass::Order, *source);
        }
        record_ws_connections_active(FeedKind::Market, true);
        record_ws_connections_active(FeedKind::Market, false);
        for result in ConnectResult::ALL {
            record_ws_connect_attempt(FeedKind::Market, *result);
        }
        for reason in ReconnectReason::ALL {
            record_ws_reconnect(FeedKind::Market, *reason);
        }
        record_ws_server_disconnect(FeedKind::Market, 805);
        for kind in FrameKind::ALL {
            record_ws_frame(FeedKind::Market, *kind, 162);
        }
        for packet in PacketKind::ALL {
            record_ws_packet(FeedKind::Market, *packet);
        }
        for kind in DecodeFailure::ALL {
            record_ws_decode_failure(FeedKind::Market, *kind);
        }
        record_ws_queue_depth(FeedKind::Market, 3);
        record_ws_dropped(FeedKind::Market, 2);
    }

    #[cfg(feature = "metrics")]
    #[test]
    fn recorded_series_carry_the_catalogued_names_and_labels() {
        use metrics_util::debugging::{DebugValue, DebuggingRecorder};

        /// `(name, labels, value)` of one recorded series.
        type Series = (String, Vec<(String, String)>, String);

        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            record_http_request(
                EndpointId::OrdersList,
                Some(ErrorKind::RateLimited),
                Duration::from_millis(250),
            );
            record_http_retry(EndpointId::OrdersList, RetryCause::Status503);
            record_ws_server_disconnect(FeedKind::Market, 805);
            record_ws_frame(FeedKind::Market, FrameKind::Binary, 162);
        });
        let mut seen: Vec<Series> = snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .map(|(key, _, _, value)| {
                let key = key.key();
                let labels = key
                    .labels()
                    .map(|l| (l.key().to_owned(), l.value().to_owned()))
                    .collect();
                let value = match value {
                    DebugValue::Counter(n) => format!("counter {n}"),
                    DebugValue::Gauge(g) => format!("gauge {}", g.into_inner()),
                    DebugValue::Histogram(h) => {
                        format!(
                            "histogram {:?}",
                            h.iter().map(|v| v.into_inner()).collect::<Vec<_>>()
                        )
                    }
                };
                (key.name().to_owned(), labels, value)
            })
            .collect();
        seen.sort();
        let l = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            seen,
            vec![
                (
                    HTTP_REQUEST_DURATION_SECONDS.to_owned(),
                    l(&[("endpoint", "orders.list"), ("outcome", "rate_limited")]),
                    "histogram [0.25]".to_owned()
                ),
                (
                    HTTP_REQUESTS_TOTAL.to_owned(),
                    l(&[("endpoint", "orders.list"), ("outcome", "rate_limited")]),
                    "counter 1".to_owned()
                ),
                (
                    HTTP_RETRIES_TOTAL.to_owned(),
                    l(&[("endpoint", "orders.list"), ("cause", "status_503")]),
                    "counter 1".to_owned()
                ),
                (
                    WS_BYTES_TOTAL.to_owned(),
                    l(&[("feed", "market")]),
                    "counter 162".to_owned()
                ),
                (
                    WS_FRAMES_TOTAL.to_owned(),
                    l(&[("feed", "market"), ("kind", "binary")]),
                    "counter 1".to_owned()
                ),
                (
                    WS_SERVER_DISCONNECTS_TOTAL.to_owned(),
                    l(&[("feed", "market"), ("code", "805")]),
                    "counter 1".to_owned()
                ),
            ]
        );
    }
}
