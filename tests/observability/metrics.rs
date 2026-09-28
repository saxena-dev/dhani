//! Metrics (architecture §6.7 item 6): a `DebuggingRecorder` installed for the whole test body
//! with `set_default_local_recorder` on a current-thread runtime. Without the `metrics` feature
//! these tests are not compiled.

use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};

use super::*;
use dhani::obs::metrics as names;
use dhani::types::OrderId;

/// `(name, sorted labels, value)` of every series.
type Series = (String, Vec<(String, String)>, DebugValue);

fn series(snapshotter: &Snapshotter) -> Vec<Series> {
    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .map(|(key, _, _, value)| {
            let key = key.key();
            let mut labels: Vec<_> = key
                .labels()
                .map(|l| (l.key().to_owned(), l.value().to_owned()))
                .collect();
            labels.sort();
            (key.name().to_owned(), labels, value)
        })
        .collect()
}

fn counter(all: &[Series], name: &str, labels: &[(&str, &str)]) -> u64 {
    let mut want: Vec<(String, String)> = labels
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    want.sort();
    all.iter()
        .find(|(n, l, _)| n == name && *l == want)
        .map(|(_, _, v)| match v {
            DebugValue::Counter(c) => *c,
            other => panic!("{name} is not a counter: {other:?}"),
        })
        .unwrap_or(0)
}

fn histogram_samples(all: &[Series], name: &str) -> usize {
    all.iter()
        .filter(|(n, _, _)| n == name)
        .map(|(_, _, v)| match v {
            DebugValue::Histogram(h) => h.len(),
            other => panic!("{name} is not a histogram: {other:?}"),
        })
        .sum()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_success_counts_one_request_and_one_attempt() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = metrics::set_default_local_recorder(&recorder);
    let server = FaultHttp::start(vec![Reply::json(200, EMPTY_LIST)]).await;
    run(read(&client(&server, RateLimiter::disabled())))
        .await
        .unwrap();
    let all = series(&snapshotter);
    let ok = [("endpoint", "orders.list"), ("outcome", "ok")];
    assert_eq!(counter(&all, names::HTTP_REQUESTS_TOTAL, &ok), 1);
    assert_eq!(
        counter(
            &all,
            names::HTTP_ATTEMPTS_TOTAL,
            &[("endpoint", "orders.list"), ("result", "ok")]
        ),
        1
    );
    assert_eq!(
        histogram_samples(&all, names::HTTP_REQUEST_DURATION_SECONDS),
        1
    );
    assert_eq!(
        histogram_samples(&all, names::HTTP_ATTEMPT_DURATION_SECONDS),
        1
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_503_then_200_counts_one_retry() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = metrics::set_default_local_recorder(&recorder);
    let server = FaultHttp::start(vec![
        Reply::text(503, "Service Unavailable"),
        Reply::json(200, EMPTY_LIST),
    ])
    .await;
    run(read(&client(&server, RateLimiter::disabled())))
        .await
        .unwrap();
    let all = series(&snapshotter);
    let endpoint = ("endpoint", "orders.list");
    assert_eq!(
        counter(
            &all,
            names::HTTP_RETRIES_TOTAL,
            &[endpoint, ("cause", "status_503")]
        ),
        1
    );
    assert_eq!(
        counter(
            &all,
            names::HTTP_ATTEMPTS_TOTAL,
            &[endpoint, ("result", "http_5xx")]
        ),
        1
    );
    assert_eq!(
        counter(
            &all,
            names::HTTP_ATTEMPTS_TOTAL,
            &[endpoint, ("result", "ok")]
        ),
        1
    );
    assert_eq!(
        counter(
            &all,
            names::HTTP_REQUESTS_TOTAL,
            &[endpoint, ("outcome", "ok")]
        ),
        1
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn two_429s_count_one_remote_rejection() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = metrics::set_default_local_recorder(&recorder);
    let limited = || Reply::text(429, "Too Many Requests");
    let server = FaultHttp::start(vec![limited(), limited()]).await;
    run(read(&client(&server, RateLimiter::disabled())))
        .await
        .unwrap_err();
    let all = series(&snapshotter);
    assert_eq!(
        counter(
            &all,
            names::RATELIMIT_REJECTIONS_TOTAL,
            &[("rate_class", "non_trading"), ("source", "remote")]
        ),
        1
    );
    assert_eq!(
        counter(
            &all,
            names::HTTP_RETRIES_TOTAL,
            &[("endpoint", "orders.list"), ("cause", "rate_limited")]
        ),
        1
    );
    assert_eq!(
        counter(
            &all,
            names::HTTP_REQUESTS_TOTAL,
            &[("endpoint", "orders.list"), ("outcome", "rate_limited")]
        ),
        1
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn every_rest_and_ratelimit_metric_is_recorded() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = metrics::set_default_local_recorder(&recorder);
    // A retried read, then back-to-back quotes (a wait), then a zero-wait refusal.
    let server = FaultHttp::start(vec![
        Reply::text(503, "Service Unavailable"),
        Reply::json(200, EMPTY_LIST),
        Reply::json(200, QUOTE_OK),
        Reply::json(200, QUOTE_OK),
        Reply::json(200, QUOTE_OK),
    ])
    .await;
    let waiting = client(&server, RateLimiter::default());
    run(read(&waiting)).await.unwrap();
    run(quote(&waiting)).await.unwrap();
    run(quote(&waiting)).await.unwrap();
    let strict = RateLimiter::new(
        QuotaProfile::dhan_v2(),
        AdmissionLimits::new(Duration::ZERO, 256).unwrap(),
    );
    let refusing = client(&server, strict);
    run(quote(&refusing)).await.unwrap();
    run(quote(&refusing)).await.unwrap_err();
    let all = series(&snapshotter);
    let seen: Vec<&str> = all.iter().map(|(n, _, _)| n.as_str()).collect();
    for name in [
        names::HTTP_REQUESTS_TOTAL,
        names::HTTP_REQUEST_DURATION_SECONDS,
        names::HTTP_ATTEMPTS_TOTAL,
        names::HTTP_ATTEMPT_DURATION_SECONDS,
        names::HTTP_RETRIES_TOTAL,
        names::RATELIMIT_WAIT_SECONDS,
        names::RATELIMIT_REJECTIONS_TOTAL,
    ] {
        assert!(seen.contains(&name), "{name} was never recorded");
    }
    assert_eq!(
        counter(
            &all,
            names::RATELIMIT_REJECTIONS_TOTAL,
            &[("rate_class", "quote"), ("source", "wait_exceeded")]
        ),
        1
    );
}

// Real clock: each cancel fails at connect immediately, so no timer is involved.
#[tokio::test(flavor = "current_thread")]
async fn distinct_order_ids_add_no_series() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = metrics::set_default_local_recorder(&recorder);
    // Nothing listens: every cancel fails fast at connect, one attempt each.
    let base = crate::support::fault_http::refused_base_url().await;
    let mut urls = Urls::for_env(Environment::Live);
    urls.rest = url::Url::parse(&format!("{base}/v2")).unwrap();
    let client = DhanClient::builder()
        .urls(urls)
        .credentials(crate::support::mock::credentials())
        .rate_limiter(RateLimiter::disabled())
        .build()
        .unwrap();
    for n in 0..10_000u64 {
        let id = OrderId::new(format!("{}", 100_000_000_000 + n)).unwrap();
        let err = client.orders().cancel(&id).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Transport);
    }
    let all = series(&snapshotter);
    let requests: Vec<_> = all
        .iter()
        .filter(|(n, _, _)| n == names::HTTP_REQUESTS_TOTAL)
        .collect();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(
        counter(
            &all,
            names::HTTP_REQUESTS_TOTAL,
            &[("endpoint", "orders.cancel"), ("outcome", "transport")]
        ),
        10_000
    );
    let bound = dhani::obs::SERIES_BOUND
        .iter()
        .find(|(n, _)| *n == names::HTTP_REQUESTS_TOTAL)
        .unwrap()
        .1;
    assert!(requests.len() <= bound);
    for (_, labels, _) in &all {
        for (_, value) in labels {
            assert!(
                !value.starts_with("1000000"),
                "an order id became a label: {value}"
            );
        }
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_retried_429_is_not_a_rejection() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = metrics::set_default_local_recorder(&recorder);
    let server = FaultHttp::start(vec![
        Reply::text(429, "Too Many Requests"),
        Reply::json(200, EMPTY_LIST),
    ])
    .await;
    run(read(&client(&server, RateLimiter::disabled())))
        .await
        .unwrap();
    let all = series(&snapshotter);
    assert_eq!(
        counter(
            &all,
            names::RATELIMIT_REJECTIONS_TOTAL,
            &[("rate_class", "non_trading"), ("source", "remote")]
        ),
        0
    );
    assert_eq!(
        counter(
            &all,
            names::HTTP_RETRIES_TOTAL,
            &[("endpoint", "orders.list"), ("cause", "rate_limited")]
        ),
        1
    );
}
