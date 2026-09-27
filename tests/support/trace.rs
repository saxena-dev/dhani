//! `CaptureLayer` and a scoped `set_default` helper for observability assertions.
//!
//! [`install`] returns a [`Capture`] and a guard; until the guard drops, every span and event on
//! the current thread is recorded: spans with a unique id, name, level, target, parent, declared
//! field names and every recorded value (including later `record` calls), and events with their
//! target, level, parent span and fields.
//!
//! `tracing` caches each callsite's interest process-wide. While at most one dispatcher exists,
//! that interest comes from whichever thread registers the callsite first, so a test without a
//! subscriber can make a span "never enabled" for a concurrent test. [`install`] therefore keeps
//! a second, `NoSubscriber` dispatcher alive for the whole binary, which makes every
//! registration combine all live dispatchers.
//!
//! The capture is thread-local. Tests must run the code under observation on the installing
//! thread: `#[tokio::test]` (current-thread) does, and so does `#[tokio::test(flavor =
//! "current_thread")]` for feed tests whose actors are spawned onto the same runtime.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::{DefaultGuard, NoSubscriber};
use tracing::{Dispatch, Level};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

/// One captured span.
#[derive(Clone, Debug)]
pub struct SpanRecord {
    /// Unique for the capture (tracing ids can be reused after a span closes).
    pub id: u64,
    pub name: &'static str,
    pub level: Level,
    pub target: &'static str,
    /// The parent's `id`, if the parent was captured.
    pub parent: Option<u64>,
    /// Every declared field, recorded or not, in declaration order.
    pub field_names: Vec<&'static str>,
    /// Recorded values: strings verbatim, other values in `Debug` form.
    pub fields: BTreeMap<&'static str, String>,
}

impl SpanRecord {
    /// A recorded field value.
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }
}

/// One captured event.
#[derive(Clone, Debug)]
pub struct EventRecord {
    pub target: &'static str,
    pub level: Level,
    /// The `id` of the span the event was emitted in, if captured.
    pub parent: Option<u64>,
    pub fields: BTreeMap<&'static str, String>,
}

impl EventRecord {
    /// A field value.
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }

    /// The `event` field.
    pub fn name(&self) -> &str {
        self.field("event").unwrap_or("")
    }
}

#[derive(Default)]
struct Log {
    next_id: u64,
    /// Open spans: tracing id → capture id.
    open: HashMap<Id, u64>,
    spans: Vec<SpanRecord>,
    events: Vec<EventRecord>,
}

/// What a [`CaptureLayer`] has recorded.
#[derive(Clone, Default)]
pub struct Capture(Arc<Mutex<Log>>);

impl Capture {
    /// Every captured span, in creation order.
    pub fn spans(&self) -> Vec<SpanRecord> {
        self.0.lock().unwrap().spans.clone()
    }

    /// Captured spans named `name`.
    pub fn spans_named(&self, name: &str) -> Vec<SpanRecord> {
        self.spans()
            .into_iter()
            .filter(|s| s.name == name)
            .collect()
    }

    /// Every captured event, in emission order.
    pub fn events(&self) -> Vec<EventRecord> {
        self.0.lock().unwrap().events.clone()
    }

    /// Captured events whose `event` field is `name`.
    pub fn events_named(&self, name: &str) -> Vec<EventRecord> {
        self.events()
            .into_iter()
            .filter(|e| e.name() == name)
            .collect()
    }

    /// Captured events on `dhani::*` targets.
    pub fn dhani_events(&self) -> Vec<EventRecord> {
        self.events()
            .into_iter()
            .filter(|e| e.target.starts_with("dhani::"))
            .collect()
    }
}

/// Records spans and events into a [`Capture`].
pub struct CaptureLayer(Capture);

struct Values<'a>(&'a mut BTreeMap<&'static str, String>);

impl Visit for Values<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name(), value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.0.insert(field.name(), format!("{value:?}"));
    }
}

impl<S> tracing_subscriber::Layer<S> for CaptureLayer
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let meta = attrs.metadata();
        let mut fields = BTreeMap::new();
        attrs.record(&mut Values(&mut fields));
        let parent_id = ctx.span(id).and_then(|s| s.parent()).map(|p| p.id());
        let mut log = self.0.0.lock().unwrap();
        log.next_id += 1;
        let unique = log.next_id;
        let parent = parent_id.and_then(|p| log.open.get(&p).copied());
        log.open.insert(id.clone(), unique);
        log.spans.push(SpanRecord {
            id: unique,
            name: meta.name(),
            level: *meta.level(),
            target: meta.target(),
            parent,
            field_names: meta.fields().iter().map(|f| f.name()).collect(),
            fields,
        });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, _: Context<'_, S>) {
        let mut log = self.0.0.lock().unwrap();
        let Some(unique) = log.open.get(id).copied() else {
            return;
        };
        if let Some(span) = log.spans.iter_mut().rev().find(|s| s.id == unique) {
            values.record(&mut Values(&mut span.fields));
        }
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        let meta = event.metadata();
        let mut fields = BTreeMap::new();
        event.record(&mut Values(&mut fields));
        let span_id = ctx.event_span(event).map(|s| s.id());
        let mut log = self.0.0.lock().unwrap();
        let parent = span_id.and_then(|s| log.open.get(&s).copied());
        log.events.push(EventRecord {
            target: meta.target(),
            level: *meta.level(),
            parent,
            fields,
        });
    }

    fn on_close(&self, id: Id, _: Context<'_, S>) {
        self.0.0.lock().unwrap().open.remove(&id);
    }
}

/// Like [`install`], but records only spans and events at `max` or more severe, as a
/// subscriber configured at that level would (for example `Level::DEBUG` leaves TRACE spans
/// disabled).
pub fn install_with_max_level(max: Level) -> (Capture, DefaultGuard) {
    use tracing_subscriber::Layer;
    use tracing_subscriber::filter::LevelFilter;

    static KEEP: OnceLock<Dispatch> = OnceLock::new();
    KEEP.get_or_init(|| Dispatch::new(NoSubscriber::default()));
    let capture = Capture::default();
    let layer = CaptureLayer(capture.clone()).with_filter(LevelFilter::from_level(max));
    let subscriber = tracing_subscriber::registry().with(layer);
    (capture, tracing::subscriber::set_default(subscriber))
}

/// Starts capturing on this thread until the guard drops.
pub fn install() -> (Capture, DefaultGuard) {
    static KEEP: OnceLock<Dispatch> = OnceLock::new();
    KEEP.get_or_init(|| Dispatch::new(NoSubscriber::default()));
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(CaptureLayer(capture.clone()));
    (capture, tracing::subscriber::set_default(subscriber))
}
