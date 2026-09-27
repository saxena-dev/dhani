//! Observability contract tests (REST): catalogue conformance of the REST spans and events, and
//! the eight REST scenarios of the architecture's observability test plan, driven through the
//! client's test hook against the raw-TCP fault harness on a paused clock.
//!
//! Event coverage (every `http.*` and `ratelimit.*` event is asserted by at least one test):
//! `http.request.completed` (success), `http.request.failed` (twice_429), `http.request.rejected`
//! (mutation_429), `http.retry.scheduled` (retry_503), `ratelimit.waited` and
//! `ratelimit.refused` (back_to_back_quotes), `ratelimit.remote` (the four 429 tests).

mod support;

use std::future::Future;
use std::time::Duration;

use dhani::config::{Environment, Urls};
use dhani::labels::{EndpointId, Method, RateClass, RetryClass};
use dhani::obs::{EVENT_CATALOGUE, SPAN_CATALOGUE, events, spans};
use dhani::rest::{AdmissionLimits, BodyLimits, QuotaProfile, RateLimiter, Timeouts};
use dhani::types::RawJson;
use dhani::{DhanClient, ErrorKind};
use support::fault_http::{FaultHttp, Reply};
use support::trace::{Capture, SpanRecord, install};
use tokio::task::JoinHandle;

const REQUEST: &str = "dhani.http.request";
const ATTEMPT: &str = "dhani.http.attempt";
const ADMISSION: &str = "dhani.http.admission";
const EMPTY_LIST: &str = "[]";
const QUOTE_OK: &str = r#"{"data":{},"status":"success"}"#;

type Outcome = dhani::Result<Option<RawJson>>;

fn builder(
    server: &FaultHttp,
    limiter: RateLimiter,
    timeouts: Timeouts,
) -> dhani::DhanClientBuilder {
    let base = server.base_url();
    let mut urls = Urls::for_env(Environment::Live);
    urls.rest = url::Url::parse(&format!("{base}/v2")).unwrap();
    urls.auth = url::Url::parse(&base).unwrap();
    DhanClient::builder()
        .urls(urls)
        .timeouts(timeouts)
        .limits(BodyLimits::default())
        .rate_limiter(limiter)
}

fn client(server: &FaultHttp, limiter: RateLimiter) -> DhanClient {
    builder(server, limiter, Timeouts::default())
        .credentials(support::mock::credentials())
        .build()
        .unwrap()
}

fn call(
    client: &DhanClient,
    id: EndpointId,
    body: Option<serde_json::Value>,
) -> impl Future<Output = Outcome> + Send + 'static {
    let client = client.clone();
    async move { client.__execute_for_tests(id, &[], &[], body, None).await }
}

fn read(client: &DhanClient) -> impl Future<Output = Outcome> + Send + 'static {
    call(client, EndpointId::OrdersList, None)
}

fn place(client: &DhanClient) -> impl Future<Output = Outcome> + Send + 'static {
    call(
        client,
        EndpointId::OrdersPlace,
        Some(serde_json::json!({"transactionType": "BUY"})),
    )
}

fn quote(client: &DhanClient) -> impl Future<Output = Outcome> + Send + 'static {
    call(
        client,
        EndpointId::MarketQuoteLtp,
        Some(serde_json::json!({"NSE_EQ": [1333]})),
    )
}

/// Yields up to `rounds` times without advancing the clock; whether `task` finished.
async fn settle<T>(task: &JoinHandle<T>, rounds: usize) -> bool {
    for _ in 0..rounds {
        if task.is_finished() {
            return true;
        }
        tokio::task::yield_now().await;
    }
    task.is_finished()
}

/// Runs `fut` on the paused clock. Spinning on `yield_now` keeps the runtime from parking, so
/// the clock never auto-advances while loopback IO is pending; the clock is advanced in 100 ms
/// steps only once the call has stopped making progress without it (parked on a timer).
async fn run<T: Send + 'static>(fut: impl Future<Output = T> + Send + 'static) -> T {
    let task = tokio::spawn(fut);
    for _ in 0..20_000 {
        if settle(&task, 2_000).await {
            return task.await.unwrap();
        }
        tokio::time::advance(Duration::from_millis(100)).await;
    }
    panic!("the call did not finish");
}

/// Every captured dhani span and event matches its catalogue entry.
fn assert_catalogued(capture: &Capture) {
    for span in capture
        .spans()
        .iter()
        .filter(|s| s.name.starts_with("dhani."))
    {
        let spec = SPAN_CATALOGUE
            .iter()
            .find(|c| c.name == span.name)
            .unwrap_or_else(|| panic!("uncatalogued span {}", span.name));
        assert_eq!(
            (span.level, span.target, span.field_names.as_slice()),
            (spec.level, spec.target, spec.fields),
            "{}",
            span.name
        );
    }
    for event in capture.dhani_events() {
        let spec = EVENT_CATALOGUE
            .iter()
            .find(|c| c.name == event.name())
            .unwrap_or_else(|| panic!("uncatalogued event {:?}", event.fields));
        assert_eq!(event.target, spec.target, "{}", spec.name);
    }
}

fn only(spans: Vec<SpanRecord>) -> SpanRecord {
    assert_eq!(spans.len(), 1, "{spans:?}");
    spans.into_iter().next().unwrap()
}

fn gap(server: &FaultHttp) -> Duration {
    let r = server.requests();
    r[1].at - r[0].at
}

#[tokio::test]
async fn rest_span_constructors_match_the_catalogue() {
    let (capture, _guard) = install();
    let request = spans::http_request(
        EndpointId::OrdersList,
        Method::Get,
        RateClass::NonTrading,
        RetryClass::Read,
        None,
        None,
    );
    let _admission = spans::http_admission(&request, RateClass::NonTrading, false);
    let _attempt = spans::http_attempt(&request, 1);
    let captured = capture.spans();
    let names: Vec<_> = captured.iter().map(|s| s.name).collect();
    assert_eq!(names, [REQUEST, ADMISSION, ATTEMPT]);
    assert_catalogued(&capture);
    assert_eq!(
        (captured[1].parent, captured[2].parent),
        (Some(captured[0].id), Some(captured[0].id))
    );
}

#[tokio::test(start_paused = true)]
async fn success_produces_one_request_and_one_attempt_span() {
    let server = FaultHttp::start(vec![Reply::json(200, EMPTY_LIST)]).await;
    let client = client(&server, RateLimiter::disabled());
    let (capture, _guard) = install();
    run(read(&client)).await.unwrap();
    assert_catalogued(&capture);
    let request = only(capture.spans_named(REQUEST));
    let attempt = only(capture.spans_named(ATTEMPT));
    assert_eq!(attempt.parent, Some(request.id));
    assert_eq!(
        (
            request.field("endpoint"),
            request.field("outcome"),
            request.field("http_status"),
            request.field("attempts")
        ),
        (Some("orders.list"), Some("ok"), Some("200"), Some("1"))
    );
    assert_eq!(
        (
            attempt.field("attempt"),
            attempt.field("http_status"),
            attempt.field("stage")
        ),
        (Some("1"), Some("200"), Some("response_received"))
    );
    let completed = capture.events_named(events::HTTP_REQUEST_COMPLETED.name);
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].parent, Some(request.id));
    assert!(capture.spans_named(ADMISSION).is_empty());
}

#[tokio::test(start_paused = true)]
async fn retry_503_gives_two_attempts_and_one_retry_event() {
    let server = FaultHttp::start(vec![
        Reply::text(503, "Service Unavailable"),
        Reply::json(200, EMPTY_LIST),
    ])
    .await;
    let client = client(&server, RateLimiter::disabled());
    let (capture, _guard) = install();
    run(read(&client)).await.unwrap();
    assert_catalogued(&capture);
    let request = only(capture.spans_named(REQUEST));
    let attempts = capture.spans_named(ATTEMPT);
    assert_eq!(attempts.len(), 2);
    assert!(attempts.iter().all(|a| a.parent == Some(request.id)));
    assert_eq!(
        (
            attempts[0].field("http_status"),
            attempts[0].field("error_kind")
        ),
        (Some("503"), Some("http_status"))
    );
    assert_eq!(attempts[1].field("attempt"), Some("2"));
    let retries = capture.events_named(events::HTTP_RETRY_SCHEDULED.name);
    assert_eq!(retries.len(), 1);
    assert_eq!(
        (retries[0].field("attempt"), retries[0].field("cause")),
        (Some("1"), Some("status_503"))
    );
    assert_eq!(request.field("attempts"), Some("2"));
}

#[tokio::test(start_paused = true)]
async fn retry_429_then_200_waits_a_second_and_reports_the_remote_limit() {
    let server = FaultHttp::start(vec![
        Reply::text(429, "Too Many Requests"),
        Reply::json(200, EMPTY_LIST),
    ])
    .await;
    let client = client(&server, RateLimiter::disabled());
    let (capture, _guard) = install();
    run(read(&client)).await.unwrap();
    assert_catalogued(&capture);
    assert_eq!(capture.spans_named(ATTEMPT).len(), 2);
    let remote = capture.events_named(events::RATELIMIT_REMOTE.name);
    assert_eq!(remote.len(), 1);
    assert_eq!(
        (
            remote[0].field("will_retry"),
            remote[0].field("http_status"),
            remote[0].field("endpoint")
        ),
        (Some("true"), Some("429"), Some("orders.list"))
    );
    assert!(gap(&server) >= Duration::from_secs(1), "{:?}", gap(&server));
}

#[tokio::test(start_paused = true)]
async fn twice_429_fails_as_rate_limited_after_two_attempts() {
    let limited = || Reply::text(429, "Too Many Requests");
    let server = FaultHttp::start(vec![limited(), limited(), Reply::json(200, EMPTY_LIST)]).await;
    let client = client(&server, RateLimiter::disabled());
    let (capture, _guard) = install();
    let err = run(read(&client)).await.unwrap_err();
    assert_eq!((err.kind(), err.attempts()), (ErrorKind::RateLimited, 2));
    assert_catalogued(&capture);
    let request = only(capture.spans_named(REQUEST));
    assert_eq!(
        (
            request.field("error_kind"),
            request.field("outcome"),
            request.field("attempts")
        ),
        (Some("rate_limited"), Some("rate_limited"), Some("2"))
    );
    assert_eq!(capture.spans_named(ATTEMPT).len(), 2);
    let remote: Vec<_> = capture
        .events_named(events::RATELIMIT_REMOTE.name)
        .iter()
        .map(|e| e.field("will_retry").unwrap().to_owned())
        .collect();
    assert_eq!(remote, ["true", "false"]);
    let failed = capture.events_named(events::HTTP_REQUEST_FAILED.name);
    assert_eq!(failed.len(), 1);
    assert_eq!(
        (failed[0].level, failed[0].field("error_kind")),
        (tracing::Level::WARN, Some("rate_limited"))
    );
}

#[tokio::test(start_paused = true)]
async fn mutation_429_fails_after_one_attempt() {
    let server = FaultHttp::start(vec![
        Reply::text(429, "Too Many Requests"),
        Reply::json(200, "{}"),
    ])
    .await;
    let client = client(&server, RateLimiter::disabled());
    let (capture, _guard) = install();
    let err = run(place(&client)).await.unwrap_err();
    assert_eq!((err.kind(), err.attempts()), (ErrorKind::RateLimited, 1));
    // A mutation without its body is rejected before anything is sent.
    let err = run(call(&client, EndpointId::OrdersPlace, None))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert_catalogued(&capture);
    assert_eq!(capture.spans_named(ATTEMPT).len(), 1);
    assert!(
        capture
            .events_named(events::HTTP_RETRY_SCHEDULED.name)
            .is_empty()
    );
    let remote = capture.events_named(events::RATELIMIT_REMOTE.name);
    assert_eq!(remote.len(), 1);
    assert_eq!(remote[0].field("will_retry"), Some("false"));
    let rejected = capture.events_named(events::HTTP_REQUEST_REJECTED.name);
    assert_eq!(rejected.len(), 1);
    assert_eq!(
        (
            rejected[0].field("error_kind"),
            rejected[0].field("field"),
            rejected[0].field("reason")
        ),
        (Some("validation"), Some("body"), Some("missing"))
    );
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn retry_after_three_seconds_is_honoured() {
    let limited = Reply::Respond {
        status: 429,
        headers: vec![("retry-after", "3".to_owned())],
        body: Vec::new(),
    };
    let server = FaultHttp::start(vec![limited, Reply::json(200, EMPTY_LIST)]).await;
    let client = client(&server, RateLimiter::disabled());
    let (capture, _guard) = install();
    run(read(&client)).await.unwrap();
    assert_catalogued(&capture);
    let remote = capture.events_named(events::RATELIMIT_REMOTE.name);
    assert_eq!(remote.len(), 1);
    assert_eq!(
        (
            remote[0].field("will_retry"),
            remote[0].field("retry_after_ms")
        ),
        (Some("true"), Some("3000"))
    );
    let retry = capture.events_named(events::HTTP_RETRY_SCHEDULED.name);
    assert_eq!(retry.len(), 1);
    assert_eq!(
        (retry[0].field("cause"), retry[0].field("delay_ms")),
        (Some("rate_limited"), Some("3000"))
    );
    assert!(gap(&server) >= Duration::from_secs(3), "{:?}", gap(&server));
}

#[tokio::test(start_paused = true)]
async fn back_to_back_quotes_wait_in_admission() {
    let server =
        FaultHttp::start(vec![Reply::json(200, QUOTE_OK), Reply::json(200, QUOTE_OK)]).await;
    let client = client(&server, RateLimiter::default());
    let (capture, _guard) = install();
    run(quote(&client)).await.unwrap();
    run(quote(&client)).await.unwrap();
    assert_catalogued(&capture);
    let requests = capture.spans_named(REQUEST);
    let admission = only(capture.spans_named(ADMISSION));
    assert_eq!(admission.parent, Some(requests[1].id));
    let wait_ms: u64 = admission.field("wait_ms").unwrap().parse().unwrap();
    assert!(wait_ms >= 900, "{wait_ms}");
    assert_eq!(
        (
            admission.field("rate_class"),
            admission.field("keyed"),
            admission.field("result")
        ),
        (Some("quote"), Some("false"), Some("granted"))
    );
    let waited = capture.events_named(events::RATELIMIT_WAITED.name);
    assert_eq!(waited.len(), 1);
    assert_eq!(waited[0].field("rate_class"), Some("quote"));

    // Without an admission wait, the next quote in the same second is refused locally.
    let strict = RateLimiter::new(
        QuotaProfile::dhan_v2(),
        AdmissionLimits::new(Duration::ZERO, 256).unwrap(),
    );
    let other = FaultHttp::start(vec![Reply::json(200, QUOTE_OK)]).await;
    let strict_client = self::client(&other, strict);
    run(quote(&strict_client)).await.unwrap();
    let err = run(quote(&strict_client)).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::RateLimited);
    let refused = capture.events_named(events::RATELIMIT_REFUSED.name);
    assert_eq!(refused.len(), 1);
    assert_eq!(
        (refused[0].field("rate_class"), refused[0].field("source")),
        (Some("quote"), Some("wait_exceeded"))
    );
    assert!(
        capture
            .events_named(events::HTTP_REQUEST_FAILED.name)
            .is_empty()
    );
    assert_catalogued(&capture);
}

#[tokio::test(start_paused = true)]
async fn concurrent_requests_keep_separate_parents() {
    let server = FaultHttp::start(vec![
        Reply::json(200, EMPTY_LIST),
        Reply::json(200, EMPTY_LIST),
    ])
    .await;
    let client = client(&server, RateLimiter::disabled());
    let (capture, _guard) = install();
    let (a, b) = (tokio::spawn(read(&client)), tokio::spawn(read(&client)));
    let (a, b) = run(async move { (a.await.unwrap(), b.await.unwrap()) }).await;
    a.unwrap();
    b.unwrap();
    assert_catalogued(&capture);
    let requests = capture.spans_named(REQUEST);
    assert_eq!(requests.len(), 2);
    let ids = [requests[0].id, requests[1].id];
    let mut attempt_parents: Vec<_> = capture
        .spans_named(ATTEMPT)
        .iter()
        .map(|a| a.parent.unwrap())
        .collect();
    attempt_parents.sort_unstable();
    assert_eq!(attempt_parents, ids);
    let mut event_parents: Vec<_> = capture
        .events_named(events::HTTP_REQUEST_COMPLETED.name)
        .iter()
        .map(|e| e.parent.unwrap())
        .collect();
    event_parents.sort_unstable();
    assert_eq!(event_parents, ids);
}

// ---- Terminal-event rule (per ErrorKind) and the REST sentinel sweep ----

/// The terminal `http.request.*` events of a capture, as `(event, level)`.
fn terminal_events(capture: &Capture) -> Vec<(String, tracing::Level)> {
    capture
        .dhani_events()
        .into_iter()
        .filter(|e| e.name().starts_with("http.request."))
        .map(|e| (e.name().to_owned(), e.level))
        .collect()
}

/// How a case drives the client.
enum Drive {
    Read,
    Place,
    PlaceWithBody(serde_json::Value),
}

struct Case {
    name: &'static str,
    replies: Vec<Reply>,
    drive: Drive,
    kind: ErrorKind,
    /// The one terminal event and its level.
    terminal: (&'static str, tracing::Level),
}

fn fast_timeouts() -> Timeouts {
    Timeouts::new(
        Duration::from_secs(1),
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .unwrap()
}

#[tokio::test(start_paused = true)]
async fn every_error_kind_emits_exactly_one_terminal_event_at_its_level() {
    use tracing::Level;
    let failed = events::HTTP_REQUEST_FAILED.name;
    let rejected = events::HTTP_REQUEST_REJECTED.name;
    let oversized = serde_json::json!({"remarks": "x".repeat(2 * 1024 * 1024)});
    let cases = vec![
        Case {
            name: "validation (oversized body)",
            replies: vec![],
            drive: Drive::PlaceWithBody(oversized),
            kind: ErrorKind::Validation,
            terminal: (rejected, Level::DEBUG),
        },
        Case {
            name: "rate limited (remote)",
            replies: vec![Reply::text(429, "Too Many Requests")],
            drive: Drive::Place,
            kind: ErrorKind::RateLimited,
            terminal: (failed, Level::WARN),
        },
        Case {
            name: "timeout",
            replies: vec![Reply::Stall],
            drive: Drive::Place,
            kind: ErrorKind::Timeout,
            terminal: (failed, Level::WARN),
        },
        Case {
            name: "transport",
            replies: vec![Reply::DropAfterRequest],
            drive: Drive::Place,
            kind: ErrorKind::Transport,
            terminal: (failed, Level::WARN),
        },
        Case {
            name: "auth",
            replies: vec![Reply::json(
                401,
                r#"{"errorType":"Invalid_Authentication","errorCode":"DH-901","errorMessage":"Client ID or user generated access token is invalid or expired"}"#,
            )],
            drive: Drive::Read,
            kind: ErrorKind::Auth,
            terminal: (failed, Level::ERROR),
        },
        Case {
            name: "api",
            replies: vec![Reply::json(
                400,
                r#"{"errorType":"Input_Exception","errorCode":"DH-905"}"#,
            )],
            drive: Drive::Read,
            kind: ErrorKind::Api,
            terminal: (failed, Level::INFO),
        },
        Case {
            name: "http status",
            replies: vec![Reply::text(500, "Internal Server Error")],
            drive: Drive::Read,
            kind: ErrorKind::HttpStatus,
            terminal: (failed, Level::WARN),
        },
        Case {
            name: "decode",
            replies: vec![Reply::json(200, r#"{"orderId": 1"#)],
            drive: Drive::Read,
            kind: ErrorKind::Decode,
            terminal: (failed, Level::ERROR),
        },
    ];
    for case in cases {
        let server = FaultHttp::start(case.replies).await;
        let client = builder(&server, RateLimiter::disabled(), fast_timeouts())
            .credentials(support::mock::credentials())
            .build()
            .unwrap();
        let (capture, guard) = install();
        let call = match case.drive {
            Drive::Read => run(read(&client)).await,
            Drive::Place => run(place(&client)).await,
            Drive::PlaceWithBody(body) => {
                run(call(&client, EndpointId::OrdersPlace, Some(body))).await
            }
        };
        drop(guard);
        let err = call.expect_err(case.name);
        assert_eq!(err.kind(), case.kind, "{}", case.name);
        assert_eq!(
            terminal_events(&capture),
            [(case.terminal.0.to_owned(), case.terminal.1)],
            "{}",
            case.name
        );
        assert_catalogued(&capture);
    }
}

#[tokio::test(start_paused = true)]
async fn config_errors_are_rejected_and_credential_errors_never_reach_the_client() {
    use tracing::Level;
    // Config: a client without credentials.
    let server = FaultHttp::start(vec![]).await;
    let client = builder(&server, RateLimiter::disabled(), Timeouts::default())
        .build()
        .unwrap();
    let (capture, guard) = install();
    let err = run(read(&client)).await.unwrap_err();
    // Credential: invalid credential text is refused at the boundary, before any call exists,
    // so there is no request span and no terminal event.
    let credential: dhani::Error = dhani::ClientId::new("not a client id!").unwrap_err().into();
    drop(guard);
    assert_eq!(err.kind(), ErrorKind::Config);
    assert_eq!(credential.kind(), ErrorKind::Credential);
    assert!(!credential.may_have_reached_server());
    assert_eq!(
        terminal_events(&capture),
        [(events::HTTP_REQUEST_REJECTED.name.to_owned(), Level::DEBUG)]
    );
    let rejected = &capture.events_named(events::HTTP_REQUEST_REJECTED.name)[0];
    assert_eq!(
        (rejected.field("error_kind"), rejected.field("field")),
        (Some("config"), Some("credentials"))
    );
    assert_eq!(capture.spans_named(REQUEST).len(), 1);
    assert!(server.requests().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_local_refusal_emits_ratelimit_refused_and_no_failed_event() {
    let server = FaultHttp::start(vec![Reply::json(200, QUOTE_OK)]).await;
    let strict = RateLimiter::new(
        QuotaProfile::dhan_v2(),
        AdmissionLimits::new(Duration::ZERO, 256).unwrap(),
    );
    let client = client(&server, strict);
    run(quote(&client)).await.unwrap();
    let (capture, guard) = install();
    let err = run(quote(&client)).await.unwrap_err();
    drop(guard);
    assert_eq!((err.kind(), err.attempts()), (ErrorKind::RateLimited, 0));
    assert!(terminal_events(&capture).is_empty());
    let refused = capture.events_named(events::RATELIMIT_REFUSED.name);
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].level, tracing::Level::WARN);
    let request = only(capture.spans_named(REQUEST));
    assert_eq!(request.field("outcome"), Some("rate_limited"));
}

const SENTINEL_CLIENT_ID: &str = "9999888877";
const SENTINEL_PIN: &str = "90817263";
const SENTINEL_TOTP: &str = "835724";
const SENTINEL_APP_SECRET: &str = "ZZAPPSECRETQ7X2ZZ";

#[tokio::test(start_paused = true)]
async fn no_sentinel_reaches_spans_events_errors_or_credential_debug() {
    let token = support::mock::ACCESS_TOKEN;
    let token_signature = token.rsplit('.').next().unwrap();
    let echo = format!(
        r#"{{"errorType":"Input_Exception","errorCode":"DH-905","errorMessage":"bad request for {SENTINEL_CLIENT_ID} with {token}"}}"#
    );
    let auth_echo = format!(
        r#"{{"errorType":"Input_Exception","errorCode":"DH-905","errorMessage":"rejected pin={SENTINEL_PIN} totp={SENTINEL_TOTP}"}}"#
    );
    let server = FaultHttp::start(vec![
        Reply::json(200, EMPTY_LIST),
        Reply::json(400, echo),
        Reply::json(400, auth_echo),
        Reply::DropAfterRequest,
    ])
    .await;
    let client = builder(&server, RateLimiter::disabled(), Timeouts::default())
        .credentials(support::mock::credentials())
        .retry(dhani::rest::RetryPolicy::none())
        .build()
        .unwrap();
    let auth_query = || {
        vec![
            ("dhanClientId", SENTINEL_CLIENT_ID.to_owned()),
            ("pin", SENTINEL_PIN.to_owned()),
            ("totp", SENTINEL_TOTP.to_owned()),
        ]
    };
    let (capture, guard) = install();
    run(read(&client)).await.unwrap();
    let echoed = run(read(&client)).await.unwrap_err();
    let mut errors = vec![echoed];
    for _ in 0..2 {
        let client = client.clone();
        let query = auth_query();
        let result = run(async move {
            client
                .__execute_for_tests(EndpointId::AuthGenerateAccessToken, &[], &query, None, None)
                .await
        })
        .await;
        errors.push(result.unwrap_err());
    }
    drop(guard);

    let mut corpus: Vec<String> = Vec::new();
    for span in capture.spans() {
        corpus.extend(span.fields.values().cloned());
    }
    for event in capture.events() {
        corpus.extend(event.fields.values().cloned());
    }
    let mut sources = 0;
    for error in &errors {
        corpus.push(error.to_string());
        corpus.push(format!("{error:?}"));
        if let Some(detail) = error.detail() {
            corpus.push(detail.to_owned());
        }
        // The source chain (the reqwest error of the dropped auth call carries no URL).
        let mut source = std::error::Error::source(error);
        while let Some(link) = source {
            sources += 1;
            corpus.push(link.to_string());
            corpus.push(format!("{link:?}"));
            source = link.source();
        }
    }
    assert!(sources > 0, "the transport error has a source to sweep");
    let credentials = support::mock::credentials();
    corpus.push(format!("{credentials:?}"));
    corpus.push(format!("{:?}", credentials.client_id()));
    corpus.push(format!("{:?}", credentials.access_token()));
    corpus.push(format!(
        "{:?}",
        dhani::credentials::Pin::new(SENTINEL_PIN).unwrap()
    ));
    corpus.push(format!(
        "{:?}",
        dhani::credentials::Totp::new(SENTINEL_TOTP).unwrap()
    ));
    corpus.push(format!(
        "{:?}",
        dhani::credentials::AppSecret::new(SENTINEL_APP_SECRET).unwrap()
    ));

    for sentinel in [
        SENTINEL_CLIENT_ID,
        token,
        token_signature,
        SENTINEL_PIN,
        SENTINEL_TOTP,
        SENTINEL_APP_SECRET,
    ] {
        for text in &corpus {
            assert!(
                !text.contains(sentinel),
                "sentinel {sentinel:?} leaked in {text:?}"
            );
        }
    }
    // The sanitiser ran on the echoed broker message (not only the credential Debug output).
    let echoed_message = errors[0]
        .api()
        .and_then(|a| a.error_message.as_ref())
        .map(|m| m.as_str().to_owned())
        .expect("the echoed broker message is kept");
    assert!(echoed_message.contains("<redacted>"), "{echoed_message}");
    assert_eq!(server.requests().len(), 4);
}
