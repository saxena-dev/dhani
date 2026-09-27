//! Observability contract tests (architecture §6.7): catalogue conformance of spans and events,
//! the REST and WS scenarios, the redaction sentinel sweep and, with the `metrics` feature, the
//! metric coverage. The REST half needs the `rest` feature and the WS half the `feed` feature;
//! each is compiled only when its feature is on.

mod support;

#[cfg(feature = "rest")]
#[path = "observability/rest.rs"]
mod rest;

#[cfg(feature = "feed")]
#[path = "observability/feed.rs"]
mod feed;
