//! One constructor function per span and the `SPAN_CATALOGUE` constant.
//!
//! Every span is created here, with its fixed target and level and its full field set; fields
//! recorded later start as [`tracing::field::Empty`]. [`SPAN_CATALOGUE`] lists the same data, and
//! a test holds the two in step.
//!
//! Field values are enum labels, numbers, endpoint labels, order IDs or correlation IDs. No span
//! ever records a URL, header, credential, security ID list or payload.

use tracing::field::Empty;
use tracing::{Level, Span};

use super::{TARGET_DECODE, TARGET_HTTP, TARGET_RATELIMIT, TARGET_WS};
use crate::labels::{EndpointId, FeedKind, Method, RateClass, RetryClass};
use crate::types::{CorrelationId, OrderId};

/// The contract of one span: its name, level, target and field names in declaration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpanSpec {
    /// The span name.
    pub name: &'static str,
    /// The span level.
    pub level: Level,
    /// The `tracing` target.
    pub target: &'static str,
    /// Every field, including those recorded after creation.
    pub fields: &'static [&'static str],
}

const HTTP_REQUEST: SpanSpec = SpanSpec {
    name: "dhani.http.request",
    level: Level::INFO,
    target: TARGET_HTTP,
    fields: &[
        "endpoint",
        "method",
        "rate_class",
        "retry_class",
        "order_id",
        "correlation_id",
        "outcome",
        "http_status",
        "attempts",
        "error_kind",
        "api_error_code",
        "duration_ms",
    ],
};

const HTTP_ADMISSION: SpanSpec = SpanSpec {
    name: "dhani.http.admission",
    level: Level::DEBUG,
    target: TARGET_RATELIMIT,
    fields: &["rate_class", "keyed", "wait_ms", "result"],
};

const HTTP_ATTEMPT: SpanSpec = SpanSpec {
    name: "dhani.http.attempt",
    level: Level::DEBUG,
    target: TARGET_HTTP,
    fields: &[
        "attempt",
        "http_status",
        "stage",
        "error_kind",
        "duration_ms",
    ],
};

const WS_SESSION: SpanSpec = SpanSpec {
    name: "dhani.ws.session",
    level: Level::INFO,
    target: TARGET_WS,
    fields: &["feed", "session_id", "terminal_reason"],
};

const WS_CONNECTION: SpanSpec = SpanSpec {
    name: "dhani.ws.connection",
    level: Level::INFO,
    target: TARGET_WS,
    fields: &[
        "feed",
        "epoch",
        "attempt",
        "cause",
        "result",
        "http_status",
        "disconnect_reason",
        "server_code",
        "duration_ms",
    ],
};

const WS_RESTORE: SpanSpec = SpanSpec {
    name: "dhani.ws.restore",
    level: Level::DEBUG,
    target: TARGET_WS,
    fields: &["epoch", "revision", "instruments", "messages", "result"],
};

const WS_COMMAND: SpanSpec = SpanSpec {
    name: "dhani.ws.command",
    level: Level::DEBUG,
    target: TARGET_WS,
    fields: &["command", "revision", "decision", "rejection"],
};

const WS_FRAME: SpanSpec = SpanSpec {
    name: "dhani.ws.frame",
    level: Level::TRACE,
    target: TARGET_DECODE,
    fields: &["feed", "bytes", "packets", "result"],
};

/// Every span the crate creates.
pub const SPAN_CATALOGUE: &[SpanSpec] = &[
    HTTP_REQUEST,
    HTTP_ADMISSION,
    HTTP_ATTEMPT,
    WS_SESSION,
    WS_CONNECTION,
    WS_RESTORE,
    WS_COMMAND,
    WS_FRAME,
];

/// `dhani.http.request`: one logical REST operation, covering validation, admission, every
/// attempt and backoff. Later fields: `outcome`, `http_status`, `attempts`, `error_kind`,
/// `api_error_code`, `duration_ms`.
#[doc(hidden)]
pub fn http_request(
    endpoint: EndpointId,
    method: Method,
    rate_class: RateClass,
    retry_class: RetryClass,
    order_id: Option<&OrderId>,
    correlation_id: Option<&CorrelationId>,
) -> Span {
    let span = tracing::info_span!(
        target: TARGET_HTTP,
        "dhani.http.request",
        endpoint = endpoint.as_str(),
        method = method.as_str(),
        rate_class = rate_class.as_str(),
        retry_class = retry_class.as_str(),
        order_id = Empty,
        correlation_id = Empty,
        outcome = Empty,
        http_status = Empty,
        attempts = Empty,
        error_kind = Empty,
        api_error_code = Empty,
        duration_ms = Empty,
    );
    if let Some(id) = order_id {
        span.record("order_id", id.as_ref());
    }
    if let Some(id) = correlation_id {
        span.record("correlation_id", id.as_ref());
    }
    span
}

/// `dhani.http.admission`: a wait for, or refusal of, rate-limit admission under `parent`
/// (opened only when admission is not immediate). Later fields: `wait_ms`, `result`.
#[doc(hidden)]
pub fn http_admission(parent: &Span, rate_class: RateClass, keyed: bool) -> Span {
    tracing::debug_span!(
        target: TARGET_RATELIMIT,
        parent: parent,
        "dhani.http.admission",
        rate_class = rate_class.as_str(),
        keyed,
        wait_ms = Empty,
        result = Empty,
    )
}

/// `dhani.http.attempt`: one attempt (1-based) under `parent`, from dispatch to
/// classification. Later fields: `http_status`, `stage`, `error_kind`, `duration_ms`.
#[doc(hidden)]
pub fn http_attempt(parent: &Span, attempt: u32) -> Span {
    tracing::debug_span!(
        target: TARGET_HTTP,
        parent: parent,
        "dhani.http.attempt",
        attempt,
        http_status = Empty,
        stage = Empty,
        error_kind = Empty,
        duration_ms = Empty,
    )
}

/// `dhani.ws.session`: the lifetime of one feed actor. Later field: `terminal_reason`.
#[doc(hidden)]
pub fn ws_session(feed: FeedKind, session_id: u64) -> Span {
    tracing::info_span!(
        target: TARGET_WS,
        "dhani.ws.session",
        feed = feed.as_str(),
        session_id,
        terminal_reason = Empty,
    )
}

/// `dhani.ws.connection`: one connect attempt and, if it succeeds, the connected phase.
/// `cause` is `start` or the label of the disconnect that led to it. Later fields: `result`,
/// `http_status`, `disconnect_reason`, `server_code`, `duration_ms`.
#[doc(hidden)]
pub fn ws_connection(feed: FeedKind, epoch: u64, attempt: u32, cause: &'static str) -> Span {
    tracing::info_span!(
        target: TARGET_WS,
        "dhani.ws.connection",
        feed = feed.as_str(),
        epoch,
        attempt,
        cause,
        result = Empty,
        http_status = Empty,
        disconnect_reason = Empty,
        server_code = Empty,
        duration_ms = Empty,
    )
}

/// `dhani.ws.restore`: rewriting the desired subscriptions after a handshake. Later field:
/// `result`.
#[doc(hidden)]
pub fn ws_restore(epoch: u64, revision: u64, instruments: usize, messages: usize) -> Span {
    tracing::debug_span!(
        target: TARGET_WS,
        "dhani.ws.restore",
        epoch,
        revision,
        instruments,
        messages,
        result = Empty,
    )
}

/// `dhani.ws.command`: a subscription command (`subscribe`, `unsubscribe`, `set_mode` or
/// `replace`), opened in the caller's context. Later fields: `revision`, `decision`,
/// `rejection`.
#[doc(hidden)]
pub fn ws_command(command: &'static str) -> Span {
    tracing::debug_span!(
        target: TARGET_WS,
        "dhani.ws.command",
        command,
        revision = Empty,
        decision = Empty,
        rejection = Empty,
    )
}

/// `dhani.ws.frame`: decoding one WebSocket frame of `bytes` bytes; disabled unless `TRACE` is
/// enabled for the decode target. Later fields: `packets`, `result`.
#[doc(hidden)]
pub fn ws_frame(feed: FeedKind, bytes: usize) -> Span {
    tracing::trace_span!(
        target: TARGET_DECODE,
        "dhani.ws.frame",
        feed = feed.as_str(),
        bytes,
        packets = Empty,
        result = Empty,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spec of the span `build` makes. Another test thread can cache a callsite's interest
    /// as "never" while this thread's scoped dispatcher is being registered; a disabled span is
    /// then rebuilt after recomputing the interest cache.
    fn spec_of(build: impl Fn() -> Span) -> SpanSpec {
        for _ in 0..16 {
            let span = build();
            if span.metadata().is_some() {
                return spec_from(&span);
            }
            tracing::callsite::rebuild_interest_cache();
        }
        panic!("the span stays disabled");
    }

    fn spec_from(span: &Span) -> SpanSpec {
        let meta = span.metadata().expect("the span is enabled");
        let fields: Vec<&'static str> = meta.fields().iter().map(|f| f.name()).collect();
        SpanSpec {
            name: meta.name(),
            level: *meta.level(),
            target: meta.target(),
            fields: fields.leak(),
        }
    }

    #[test]
    fn every_constructor_matches_its_catalogue_entry() {
        // A scoped dispatcher rebuilds callsite interest when it is registered; `spec_of`
        // retries a span that another test thread's cached interest left disabled.
        let subscriber = tracing_subscriber::registry();
        tracing::subscriber::with_default(subscriber, || {
            let order = OrderId::new("112111182198").unwrap();
            let correlation = CorrelationId::new("corr-1").unwrap();
            let request = || {
                http_request(
                    EndpointId::OrdersModify,
                    Method::Put,
                    RateClass::Order,
                    RetryClass::Mutation,
                    Some(&order),
                    Some(&correlation),
                )
            };
            let parent = request();
            let built = [
                spec_of(request),
                spec_of(|| http_admission(&parent, RateClass::Order, false)),
                spec_of(|| http_attempt(&parent, 1)),
                spec_of(|| ws_session(FeedKind::Market, 1)),
                spec_of(|| ws_connection(FeedKind::Market, 1, 1, "start")),
                spec_of(|| ws_restore(1, 1, 10, 1)),
                spec_of(|| ws_command("subscribe")),
                spec_of(|| ws_frame(FeedKind::Market, 162)),
            ];
            assert_eq!(built.len(), SPAN_CATALOGUE.len());
            for (spec, entry) in built.iter().zip(SPAN_CATALOGUE) {
                assert_eq!(spec, entry);
            }
        });
    }

    #[test]
    fn the_catalogue_has_eight_distinct_spans_on_known_targets() {
        assert_eq!(SPAN_CATALOGUE.len(), 8);
        let mut names: Vec<_> = SPAN_CATALOGUE.iter().map(|s| s.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 8);
        for spec in SPAN_CATALOGUE {
            assert!(
                super::super::TARGETS.contains(&spec.target),
                "{}",
                spec.name
            );
        }
    }
}
