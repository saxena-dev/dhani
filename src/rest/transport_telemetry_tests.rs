//! Telemetry of the transport: spans and the terminal-event rule, captured with a
//! process-wide subscriber that forwards to a per-thread sink.

use super::*;

/// Captures the `event` field of every dhani event and the name of every new span on the thread
/// that installed it.
///
/// `tracing` caches each callsite's interest process-wide, so a scoped subscriber can race tests
/// on other threads that hit the same callsites without one. A single global subscriber that is
/// always interested avoids that: it forwards to whichever sink the current thread installed.
/// Tests must run on the thread that installs the sink (`#[tokio::test]` is current-thread).
mod capture {
    use std::cell::RefCell;
    use std::sync::{Arc, Mutex, Once};

    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id};
    use tracing_subscriber::layer::{Context, SubscriberExt};

    #[derive(Clone, Default)]
    pub(super) struct Captured {
        events: Arc<Mutex<Vec<String>>>,
        spans: Arc<Mutex<Vec<&'static str>>>,
    }

    thread_local! {
        static SINK: RefCell<Option<Captured>> = const { RefCell::new(None) };
    }

    struct Forward;

    struct EventName(String);

    impl Visit for EventName {
        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "event" {
                self.0 = value.to_owned();
            }
        }

        fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
    }

    impl<S> tracing_subscriber::Layer<S> for Forward
    where
        S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    {
        fn on_new_span(&self, attrs: &Attributes<'_>, _: &Id, _: Context<'_, S>) {
            SINK.with(|sink| {
                if let Some(c) = sink.borrow().as_ref() {
                    c.spans.lock().unwrap().push(attrs.metadata().name());
                }
            });
        }

        fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
            // The HTTP stack's own events are not part of the contract.
            if !event.metadata().target().starts_with("dhani::") {
                return;
            }
            SINK.with(|sink| {
                if let Some(c) = sink.borrow().as_ref() {
                    let mut name = EventName(String::new());
                    event.record(&mut name);
                    c.events.lock().unwrap().push(name.0);
                }
            });
        }
    }

    /// Clears the thread's sink when dropped.
    pub(super) struct Installed;

    impl Drop for Installed {
        fn drop(&mut self) {
            SINK.with(|sink| sink.borrow_mut().take());
        }
    }

    impl Captured {
        /// Directs this thread's spans and events to `self` until the guard drops.
        pub(super) fn install(&self) -> Installed {
            static GLOBAL: Once = Once::new();
            GLOBAL.call_once(|| {
                let subscriber = tracing_subscriber::registry().with(Forward);
                tracing::subscriber::set_global_default(subscriber)
                    .expect("no other global subscriber in the unit tests");
            });
            SINK.with(|sink| *sink.borrow_mut() = Some(self.clone()));
            Installed
        }

        pub(super) fn events(&self) -> Vec<String> {
            self.events.lock().unwrap().clone()
        }

        pub(super) fn spans(&self) -> Vec<&'static str> {
            self.spans.lock().unwrap().clone()
        }
    }
}

#[tokio::test]
async fn a_validation_failure_emits_one_rejected_event_and_no_failed_event() {
    let captured = capture::Captured::default();
    let _guard = captured.install();
    let t = transport(Environment::Live);
    let err = t
        .execute::<serde_json::Value>(Some(&credentials()), by_id(EndpointId::OrdersList), || {
            Err(ValidationError::new(
                "quantity",
                ValidationReason::NotPositive,
            ))
        })
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert_eq!(captured.events(), ["http.request.rejected"]);
    assert_eq!(captured.spans(), ["dhani.http.request"]);
}

#[tokio::test]
async fn a_success_emits_one_completed_event_inside_request_and_attempt_spans() {
    let (base, _) = scripted_server(vec![OK_JSON]).await;
    let t = loopback(base);
    let captured = capture::Captured::default();
    let _guard = captured.install();
    let rows: Vec<serde_json::Value> = t
        .execute(Some(&credentials()), by_id(EndpointId::OrdersList), || {
            Ok(Call::empty())
        })
        .await
        .unwrap();
    assert!(rows.is_empty());
    let events = captured.events();
    assert_eq!(events, ["http.request.completed"]);
    assert_eq!(
        captured
            .spans()
            .into_iter()
            .filter(|s| s.starts_with("dhani."))
            .collect::<Vec<_>>(),
        ["dhani.http.request", "dhani.http.attempt"]
    );
}

#[tokio::test]
async fn a_retried_read_emits_a_retry_event_and_one_terminal_event() {
    let (base, _) = scripted_server(vec![UNAVAILABLE, OK_JSON]).await;
    let t = loopback(base);
    let captured = capture::Captured::default();
    let _guard = captured.install();
    t.execute::<serde_json::Value>(Some(&credentials()), by_id(EndpointId::OrdersList), || {
        Ok(Call::empty())
    })
    .await
    .unwrap();
    assert_eq!(
        captured.events(),
        ["http.retry.scheduled", "http.request.completed"]
    );
    let attempts = captured
        .spans()
        .into_iter()
        .filter(|s| *s == "dhani.http.attempt")
        .count();
    assert_eq!(attempts, 2);
}

#[tokio::test]
async fn a_server_error_emits_one_failed_event() {
    let (base, _) = scripted_server(vec![UNAVAILABLE, UNAVAILABLE, UNAVAILABLE]).await;
    let t = loopback(base);
    let captured = capture::Captured::default();
    let _guard = captured.install();
    t.execute::<serde_json::Value>(Some(&credentials()), by_id(EndpointId::OrdersList), || {
        Ok(Call::empty())
    })
    .await
    .unwrap_err();
    let events = captured.events();
    assert_eq!(
        events
            .iter()
            .filter(|e| *e == "http.request.failed")
            .count(),
        1
    );
    assert_eq!(
        events.last().map(String::as_str),
        Some("http.request.failed")
    );
}

// real-time: the call is abandoned after 200 ms against a listener that never answers.
#[tokio::test]
async fn a_dropped_call_emits_no_terminal_event() {
    // Connections queue in the backlog and are never answered.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = url::Url::parse(&format!("http://{}/v2", listener.local_addr().unwrap())).unwrap();
    let t = loopback(base);
    let creds = credentials();
    let captured = capture::Captured::default();
    let _guard = captured.install();
    let call = t.execute::<serde_json::Value>(Some(&creds), by_id(EndpointId::OrdersList), || {
        Ok(Call::empty())
    });
    let abandoned = tokio::time::timeout(std::time::Duration::from_millis(200), call).await;
    assert!(abandoned.is_err());
    assert!(captured.events().is_empty());
    assert!(captured.spans().contains(&"dhani.http.request"));
    drop(listener);
}
