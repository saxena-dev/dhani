//! Observability: span constructors, event names, redaction and metrics.
//!
//! Spans and events go through `tracing` with the fixed targets below, so a consumer can filter
//! with, for example, `RUST_LOG=dhani::ws=debug`. Metrics go through the `metrics` facade when
//! the `metrics` feature is enabled.

pub mod events;
pub mod metrics;
mod redact;
pub mod spans;

pub use events::{EVENT_CATALOGUE, Event};
pub use metrics::SERIES_BOUND;
pub use redact::{Redactor, sanitize};
pub use spans::{SPAN_CATALOGUE, SpanSpec};

/// Target of REST requests, admission outcomes and retries.
pub const TARGET_HTTP: &str = "dhani::http";
/// Target of auth-host and token calls.
pub const TARGET_AUTH: &str = "dhani::auth";
/// Target of the feed connection lifecycle and subscriptions.
pub const TARGET_WS: &str = "dhani::ws";
/// Target of decoding failures and per-frame spans.
pub const TARGET_DECODE: &str = "dhani::decode";
/// Target of local rate-limiter decisions.
pub const TARGET_RATELIMIT: &str = "dhani::ratelimit";

/// Every target the crate emits under.
pub const TARGETS: [&str; 5] = [
    TARGET_HTTP,
    TARGET_AUTH,
    TARGET_WS,
    TARGET_DECODE,
    TARGET_RATELIMIT,
];
