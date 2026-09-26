//! Observability: span constructors, event names, redaction and metrics.

mod events;
mod metrics;
mod redact;
mod spans;

pub use redact::{Redactor, sanitize};
