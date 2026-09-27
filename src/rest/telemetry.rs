//! Telemetry of the REST pipeline: request and attempt span fields, the `http.*` and
//! `ratelimit.*` events, and the `dhani_http_*` / `dhani_ratelimit_*` metrics.
//!
//! Field and label values come only from closed sets (enum labels, endpoint labels, numbers and
//! parsed broker error codes); nothing here ever sees a URL, header, credential or payload.

use std::time::Duration;

use tracing::{Level, Span};

use crate::error::{Error, ErrorKind, RateLimitSource, Stage, ValidationReason};
use crate::labels::RateClass;
use crate::obs::events::{self, emit};
use crate::obs::metrics::{self, AttemptResult, RejectionSource, RetryCause};
use crate::rest::endpoint::Endpoint;
use crate::rest::retry::Cause;

/// Whole milliseconds, saturating.
pub(crate) fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

fn stage_label(stage: Stage) -> &'static str {
    match stage {
        Stage::NotSent => "not_sent",
        Stage::Sent => "sent",
        Stage::ResponseReceived => "response_received",
    }
}

fn reason_label(reason: &ValidationReason) -> &'static str {
    match reason {
        ValidationReason::Missing => "missing",
        ValidationReason::Empty => "empty",
        ValidationReason::TooLong { .. } => "too_long",
        ValidationReason::TooMany { .. } => "too_many",
        ValidationReason::OutOfRange => "out_of_range",
        ValidationReason::NotFinite => "not_finite",
        ValidationReason::NotPositive => "not_positive",
        ValidationReason::InvalidCharacters => "invalid_characters",
        ValidationReason::UnknownEnumValue => "unknown_enum_value",
        ValidationReason::Inconsistent(_) => "inconsistent",
        ValidationReason::BodyTooLarge { .. } => "body_too_large",
    }
}

fn api_code(error: &Error) -> Option<String> {
    error
        .api()
        .and_then(|a| a.error_code.as_ref())
        .map(|c| c.label())
}

fn is_local_refusal(error: &Error) -> bool {
    matches!(
        error.rate_limit().map(|r| r.source),
        Some(RateLimitSource::LocalWaitExceeded | RateLimitSource::LocalCeiling)
    )
}

/// Finishes a request: records the span's final fields and the request metrics, and emits its
/// one terminal event inside the span. `outcome` is `(http_status, attempts)` on success.
pub(crate) fn request_finished(
    span: &Span,
    ep: &Endpoint,
    outcome: Result<(u16, u32), &Error>,
    elapsed: Duration,
) {
    let duration_ms = millis(elapsed);
    let endpoint = ep.id.as_str();
    span.record("duration_ms", duration_ms);
    span.in_scope(|| match outcome {
        Ok((http_status, attempts)) => {
            span.record("outcome", "ok");
            span.record("http_status", http_status);
            span.record("attempts", attempts);
            metrics::record_http_request(ep.id, None, elapsed);
            emit!(
                Level::DEBUG,
                events::HTTP_REQUEST_COMPLETED,
                endpoint,
                http_status,
                attempts,
                duration_ms,
                "request completed"
            );
        }
        Err(error) => {
            let kind = error.kind();
            let code = api_code(error);
            span.record("outcome", kind.as_str());
            span.record("error_kind", kind.as_str());
            span.record("attempts", error.attempts());
            if let Some(status) = error.http_status() {
                span.record("http_status", status);
            }
            if let Some(code) = &code {
                span.record("api_error_code", code.as_str());
            }
            metrics::record_http_request(ep.id, Some(kind), elapsed);
            match kind {
                ErrorKind::Validation | ErrorKind::Config | ErrorKind::Credential => {
                    rejected(endpoint, error)
                }
                // Reported by the limiter as ratelimit.refused.
                ErrorKind::RateLimited if is_local_refusal(error) => {}
                _ => failed(endpoint, error, code.as_deref(), duration_ms),
            }
        }
    });
}

fn rejected(endpoint: &'static str, error: &Error) {
    let (field, reason) = match (error.validation(), error.config()) {
        (Some(v), _) => (Some(v.field), reason_label(&v.reason)),
        (None, Some(c)) => (Some(c.field), c.reason),
        (None, None) => (None, "invalid credential text"),
    };
    emit!(
        Level::DEBUG,
        events::HTTP_REQUEST_REJECTED,
        endpoint,
        error_kind = error.kind().as_str(),
        field,
        reason,
        "request rejected before sending"
    );
}

fn failed(endpoint: &'static str, error: &Error, api_error_code: Option<&str>, duration_ms: u64) {
    macro_rules! failed_at {
        ($level:expr) => {
            emit!(
                $level,
                events::HTTP_REQUEST_FAILED,
                endpoint,
                error_kind = error.kind().as_str(),
                stage = stage_label(error.stage()),
                http_status = error.http_status(),
                api_error_code,
                attempts = error.attempts(),
                duration_ms,
                "request failed"
            )
        };
    }
    match error.kind() {
        ErrorKind::Api => failed_at!(Level::INFO),
        ErrorKind::Auth | ErrorKind::Decode => failed_at!(Level::ERROR),
        _ => failed_at!(Level::WARN),
    }
}

/// Marks a request span `outcome = "cancelled"` (or `"panicked"` while unwinding), with its
/// duration, if the call is dropped before it finishes.
pub(crate) struct Pending<'a> {
    span: &'a Span,
    started: tokio::time::Instant,
    finished: bool,
}

impl<'a> Pending<'a> {
    pub(crate) fn new(span: &'a Span, started: tokio::time::Instant) -> Self {
        Pending {
            span,
            started,
            finished: false,
        }
    }

    /// The call completed; its outcome is recorded by [`request_finished`].
    pub(crate) fn finished(&mut self) {
        self.finished = true;
    }
}

impl Drop for Pending<'_> {
    fn drop(&mut self) {
        if !self.finished {
            let outcome = if std::thread::panicking() {
                "panicked"
            } else {
                "cancelled"
            };
            self.span.record("outcome", outcome);
            self.span
                .record("duration_ms", millis(self.started.elapsed()));
        }
    }
}

/// How an attempt ended, for the attempts metric.
fn attempt_result(error: &Error) -> AttemptResult {
    match (error.kind(), error.http_status()) {
        (ErrorKind::Timeout, _) => AttemptResult::Timeout,
        (ErrorKind::Transport, _) => AttemptResult::Transport,
        (ErrorKind::Decode, _) => AttemptResult::Decode,
        (_, Some(status)) if status >= 500 => AttemptResult::Http5xx,
        // Any other non-2xx status (4xx, and the rare unfollowed redirect).
        _ => AttemptResult::Http4xx,
    }
}

/// Finishes an attempt span and records the attempt metrics. `result` is the status on success.
pub(crate) fn attempt_finished(
    span: &Span,
    ep: &Endpoint,
    result: Result<u16, &Error>,
    elapsed: Duration,
) {
    span.record("duration_ms", millis(elapsed));
    let label = match result {
        Ok(status) => {
            span.record("http_status", status);
            span.record("stage", stage_label(Stage::ResponseReceived));
            AttemptResult::Ok
        }
        Err(error) => {
            if let Some(status) = error.http_status() {
                span.record("http_status", status);
            }
            span.record("stage", stage_label(error.stage()));
            span.record("error_kind", error.kind().as_str());
            attempt_result(error)
        }
    };
    metrics::record_http_attempt(ep.id, label, elapsed);
}

/// A retry of attempt `attempt` is about to sleep for `delay`.
pub(crate) fn retry_scheduled(ep: &Endpoint, attempt: u32, cause: Cause, delay: Duration) {
    let label = match cause {
        Cause::Status502 => RetryCause::Status502,
        Cause::Status503 => RetryCause::Status503,
        Cause::Status504 => RetryCause::Status504,
        Cause::Timeout => RetryCause::Timeout,
        Cause::Transport => RetryCause::Transport,
        Cause::RemoteRateLimit { .. } => RetryCause::RateLimited,
    };
    metrics::record_http_retry(ep.id, label);
    emit!(
        Level::WARN,
        events::HTTP_RETRY_SCHEDULED,
        endpoint = ep.id.as_str(),
        attempt,
        cause = cause.label(),
        delay_ms = millis(delay),
        "retry scheduled"
    );
}

/// The broker rate-limited an attempt. The rejection metric counts only a remote rate limit
/// that ends the call (one that is retried is not a rejection of the request).
pub(crate) fn remote_rate_limited(
    ep: &Endpoint,
    error: &Error,
    will_retry: bool,
    retry_after: Option<Duration>,
) {
    if !will_retry {
        metrics::record_ratelimit_rejection(ep.rate, RejectionSource::Remote);
    }
    let code = api_code(error);
    emit!(
        Level::WARN,
        events::RATELIMIT_REMOTE,
        endpoint = ep.id.as_str(),
        http_status = error.http_status(),
        api_error_code = code.as_deref(),
        will_retry,
        retry_after_ms = retry_after.map(millis),
        "rate limited by the broker"
    );
}

/// Admission was granted after waiting `waited`.
pub(crate) fn admission_waited(class: RateClass, waited: Duration) {
    metrics::record_ratelimit_wait(class, waited);
    emit!(
        Level::DEBUG,
        events::RATELIMIT_WAITED,
        rate_class = class.as_str(),
        wait_ms = millis(waited),
        "admitted after waiting"
    );
}

/// Why the local limiter refused a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    WaitExceeded,
    Ceiling,
    WaitersFull,
}

impl Refusal {
    /// The admission span's `result` and the event's `source`.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::WaitExceeded => "wait_exceeded",
            Self::Ceiling => "ceiling",
            Self::WaitersFull => "waiters_full",
        }
    }
}

/// The local limiter refused a request.
pub(crate) fn admission_refused(class: RateClass, refusal: Refusal) {
    let source = match refusal {
        Refusal::WaitExceeded => RejectionSource::WaitExceeded,
        Refusal::Ceiling => RejectionSource::Ceiling,
        Refusal::WaitersFull => RejectionSource::WaitersFull,
    };
    metrics::record_ratelimit_rejection(class, source);
    emit!(
        Level::WARN,
        events::RATELIMIT_REFUSED,
        rate_class = class.as_str(),
        source = refusal.as_str(),
        "refused by the local rate limiter"
    );
}
