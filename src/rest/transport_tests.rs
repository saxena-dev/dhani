//! Unit tests of the transport pipeline's pure parts: path filling, classification, decoding and
//! request preparation. Network behaviour is covered by the integration tests.

use super::*;
use crate::credentials::{AccessToken, ClientId};
use crate::error::{ApiErrorCode, DataErrorCode, RateLimitSource};
use crate::rest::endpoint::by_id;

const SENTINEL_CLIENT: &str = "SENTINELCID1";
const SENTINEL_TOKEN: &str = "SENTINEL.TOKEN.VALUE";

fn settings() -> TransportSettings {
    TransportSettings {
        attempt_timeout: Duration::from_secs(15),
        operation_timeout: Duration::from_secs(30),
        retry: RetryLimits {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(250),
            max_backoff: Duration::from_secs(4),
            rate_limit_retries: 1,
            rate_limit_initial_backoff: Duration::from_secs(1),
        },
        max_request_bytes: 1024,
        max_json_response_bytes: 8 << 20,
        max_csv_response_bytes: 256 << 20,
    }
}

fn transport(env: Environment) -> Transport {
    Transport::new(
        // No proxy: loopback tests must not be routed through an HTTP_PROXY from the environment.
        reqwest::Client::builder().no_proxy().build().unwrap(),
        Urls::for_env(env),
        env,
        settings(),
        RateLimiter::disabled(),
        HeaderValue::from_static("dhani/test"),
        7,
    )
}

fn credentials() -> Credentials {
    Credentials::new(
        ClientId::new(SENTINEL_CLIENT).unwrap(),
        AccessToken::new(SENTINEL_TOKEN).unwrap(),
    )
}

fn prepared(
    t: &Transport,
    creds: Option<&Credentials>,
    id: EndpointId,
    call: Call<'_>,
) -> Result<Prepared, Error> {
    t.prepare(creds, by_id(id), call, &Redactor::new())
}

#[test]
fn fill_path_encodes_each_argument() {
    assert_eq!(
        fill_path("/orders/{order_id}", &["a b/c"]).as_deref(),
        Some("/orders/a%20b%2Fc")
    );
    assert_eq!(
        fill_path("/orders/{order_id}", &["A-z_0.9~"]).as_deref(),
        Some("/orders/A-z_0.9~")
    );
    assert_eq!(
        fill_path("/trades/{a}/{b}/{c}", &["2024-01-01", "2024-01-31", "0"]).as_deref(),
        Some("/trades/2024-01-01/2024-01-31/0")
    );
    assert_eq!(fill_path("/orders", &[]).as_deref(), Some("/orders"));
    assert_eq!(fill_path("/orders/{order_id}", &[]), None);
    assert_eq!(fill_path("/orders", &["x"]), None);
    assert_eq!(
        fill_path("/x/{y}", &["é?"]).as_deref(),
        Some("/x/%C3%A9%3F")
    );
    for dots in ["", ".", ".."] {
        assert_eq!(
            fill_path("/globalstocks/trades/{security_id}", &[dots]),
            None,
            "{dots:?}"
        );
    }
}

#[test]
fn correlation_ids_with_spaces_are_rejected_before_any_request() {
    let err = crate::types::CorrelationId::new("a b").unwrap_err();
    assert_eq!(err.reason, ValidationReason::InvalidCharacters);
}

fn classify_shape(id: EndpointId, status: u16, body: &[u8]) -> Result<Success, Error> {
    classify(by_id(id), status, body.to_vec(), &Redactor::new())
}

#[test]
fn classify_success_shapes() {
    // Json (orders.list): an empty or blank body is a decode error.
    for body in [&b""[..], b"  \n"] {
        let err = classify_shape(EndpointId::OrdersList, 200, body).unwrap_err();
        assert_eq!(
            (err.kind(), err.stage()),
            (ErrorKind::Decode, Stage::ResponseReceived)
        );
    }
    assert_eq!(
        classify_shape(EndpointId::OrdersList, 200, b"[]").unwrap(),
        Success::Json(b"[]".to_vec())
    );
    // Empty (portfolio.convert_position): empty, whitespace or JSON is fine unless it is an
    // object whose status is not "success"; other text is not.
    for body in [
        &b""[..],
        b" ",
        br#"{"status":"SUCCESS"}"#,
        br#"{"message":"done"}"#,
        b"null",
    ] {
        assert_eq!(
            classify_shape(EndpointId::PortfolioConvertPosition, 202, body).unwrap(),
            Success::Empty,
            "{body:?}"
        );
    }
    for body in [&br#"{"status":"ok"}"#[..], br#"{"status":"failure"}"#] {
        let err = classify_shape(EndpointId::PortfolioConvertPosition, 200, body).unwrap_err();
        assert_eq!(
            (err.kind(), err.http_status()),
            (ErrorKind::Api, Some(200)),
            "{body:?}"
        );
    }
    let err = classify_shape(EndpointId::PortfolioConvertPosition, 200, b"<html>").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Decode);
    // JsonOrEmpty (super_orders.cancel_leg).
    for body in [&b""[..], b"null", b"{}", b" {} "] {
        assert_eq!(
            classify_shape(EndpointId::SuperOrdersCancelLeg, 200, body).unwrap(),
            Success::JsonOrEmpty(None),
            "{body:?}"
        );
    }
    assert_eq!(
        classify_shape(EndpointId::SuperOrdersCancelLeg, 200, br#"{"orderId":"1"}"#).unwrap(),
        Success::JsonOrEmpty(Some(br#"{"orderId":"1"}"#.to_vec()))
    );
    // Csv: UTF-8 text only.
    assert_eq!(
        classify_shape(EndpointId::InstrumentsScripMasterCompact, 200, b"a,b\n").unwrap(),
        Success::Csv("a,b\n".to_owned())
    );
    let err = classify_shape(
        EndpointId::InstrumentsScripMasterCompact,
        200,
        &[0xff, 0xfe],
    )
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Decode);
}

#[test]
fn classify_error_statuses() {
    let err = classify_shape(
        EndpointId::OrdersPlace,
        400,
        br#"{"errorType":"Input_Exception","errorCode":"DH-905","errorMessage":"bad"}"#,
    )
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Api);
    assert_eq!(err.http_status(), Some(400));
    assert_eq!(
        err.api().and_then(|a| a.error_code.clone()),
        Some(ApiErrorCode::Dh905)
    );
    assert_eq!(err.stage(), Stage::ResponseReceived);
    assert_eq!(err.endpoint(), Some(EndpointId::OrdersPlace));

    let err = classify_shape(
        EndpointId::OrdersList,
        429,
        br#"{"data":{"805":"Too many"}}"#,
    )
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::RateLimited);
    let info = err.rate_limit().unwrap();
    assert_eq!(
        (info.source, info.class),
        (
            RateLimitSource::Remote,
            crate::labels::RateClass::NonTrading
        )
    );
    assert_eq!(
        err.api().and_then(|a| a.error_code.clone()),
        Some(ApiErrorCode::Data(DataErrorCode::TooManyRequests))
    );

    let err =
        classify_shape(EndpointId::OrdersList, 502, b"<html>ZZSENTINELZZ</html>").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::HttpStatus);
    assert_eq!(err.detail(), Some("non-JSON error body of 25 bytes"));
    assert!(!format!("{err} {err:?}").contains("ZZSENTINELZZ"));

    let err = classify_shape(EndpointId::OrdersList, 401, b"").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Auth);
}

#[test]
fn decode_errors_never_quote_the_body() {
    #[derive(Debug, serde::Deserialize)]
    #[allow(dead_code, reason = "decoded only to provoke an error")]
    struct Ack {
        order_id: u32,
    }
    let redactor = Redactor::new();
    let ep = by_id(EndpointId::OrdersPlace);
    let err = decode_json::<Ack>(ep, br#"{"order_id":"ZZSENTINELZZ"}"#, &redactor).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Decode);
    let detail = err.detail().unwrap();
    assert!(
        detail.starts_with(
            "response body does not match the expected shape (data error at line 1 column"
        ),
        "{detail}"
    );
    assert!(!format!("{err} {err:?}").contains("ZZSENTINELZZ"));
}

#[test]
fn missing_credentials_fail_before_anything_is_sent() {
    let t = transport(Environment::Live);
    let Err(err) = prepared(&t, None, EndpointId::OrdersList, Call::empty()) else {
        panic!("expected an error")
    };
    assert_eq!(
        (err.kind(), err.stage()),
        (ErrorKind::Config, Stage::NotSent)
    );
    assert_eq!(
        err.config().map(|c| (c.field, c.reason)),
        Some(("credentials", "credentials required"))
    );
    // Auth-host token generation needs no stored credentials.
    assert!(prepared(&t, None, EndpointId::AuthGenerateAccessToken, Call::empty()).is_ok());
}

#[test]
fn urls_keep_the_base_path() {
    let creds = credentials();
    for (env, base) in [
        (Environment::Live, "https://api.dhan.co/v2"),
        (Environment::Sandbox, "https://sandbox.dhan.co/v2"),
    ] {
        let t = transport(env);
        let call = Call {
            path_args: &["123"],
            ..Call::empty()
        };
        let p = prepared(&t, Some(&creds), EndpointId::OrdersGet, call).unwrap();
        assert_eq!(p.url.as_str(), format!("{base}/orders/123"));
    }
    let t = transport(Environment::Live);
    let p = prepared(
        &t,
        None,
        EndpointId::InstrumentsScripMasterDetailed,
        Call::empty(),
    )
    .unwrap();
    assert_eq!(
        p.url.as_str(),
        "https://images.dhan.co/api-data/api-scrip-master-detailed.csv"
    );
    let p = prepared(&t, None, EndpointId::AuthGenerateAccessToken, Call::empty()).unwrap();
    assert_eq!(
        p.url.as_str(),
        "https://auth.dhan.co/app/generateAccessToken"
    );
}

#[test]
fn headers_follow_the_auth_mode_and_are_sensitive() {
    let t = transport(Environment::Live);
    let creds = credentials();
    let p = prepared(&t, Some(&creds), EndpointId::OrdersList, Call::empty()).unwrap();
    let request = t.build(&p).unwrap();
    let h = request.headers();
    assert_eq!(h["access-token"], SENTINEL_TOKEN);
    assert_eq!(h["client-id"], SENTINEL_CLIENT);
    assert!(h["access-token"].is_sensitive() && h["client-id"].is_sensitive());
    assert!(h.get("dhanclientid").is_none());
    assert_eq!(h[ACCEPT], "application/json");
    assert_eq!(h[USER_AGENT], "dhani/test");
    assert!(h.get(CONTENT_TYPE).is_none());
    assert!(request.body().is_none());

    let p = prepared(
        &t,
        Some(&creds),
        EndpointId::AccountRenewToken,
        Call::empty(),
    )
    .unwrap();
    assert_eq!(p.headers["dhanclientid"], SENTINEL_CLIENT);
    assert!(p.headers["dhanclientid"].is_sensitive());

    let p = prepared(
        &t,
        None,
        EndpointId::InstrumentsScripMasterCompact,
        Call::empty(),
    )
    .unwrap();
    assert!(p.headers.get(ACCEPT).is_none() && p.headers.get("access-token").is_none());

    let secret = SecretString::from("SENTINEL-APP-SECRET");
    let extra = [
        ("app_id", HeaderSecret::Plain("app-1")),
        ("app_secret", HeaderSecret::Secret(&secret)),
    ];
    let call = Call {
        extra_headers: &extra,
        ..Call::empty()
    };
    let p = prepared(&t, None, EndpointId::AuthGenerateConsent, call).unwrap();
    assert_eq!(p.headers["app_id"], "app-1");
    assert!(!p.headers["app_id"].is_sensitive());
    assert!(p.headers["app_secret"].is_sensitive());
}

#[test]
fn bodies_carry_the_client_id_and_are_bounded() {
    let t = transport(Environment::Live);
    let creds = credentials();
    let call = Call {
        body: Some(serde_json::json!({"quantity": 1, "dhanClientId": "stale"})),
        ..Call::empty()
    };
    let p = prepared(&t, Some(&creds), EndpointId::OrdersPlace, call).unwrap();
    let body: serde_json::Value = serde_json::from_slice(p.body.as_ref().unwrap()).unwrap();
    assert_eq!(
        body,
        serde_json::json!({"quantity": 1, "dhanClientId": SENTINEL_CLIENT})
    );
    assert_eq!(p.headers[CONTENT_TYPE], "application/json");

    // The kill switch sends no body.
    let p = prepared(
        &t,
        Some(&creds),
        EndpointId::TraderControlSetKillSwitch,
        Call::empty(),
    )
    .unwrap();
    assert!(p.body.is_none() && p.headers.get(CONTENT_TYPE).is_none());
    let call = Call {
        body: Some(serde_json::json!({})),
        ..Call::empty()
    };
    let Err(err) = prepared(
        &t,
        Some(&creds),
        EndpointId::TraderControlSetKillSwitch,
        call,
    ) else {
        panic!("body accepted")
    };
    assert_eq!(err.kind(), ErrorKind::Validation);

    let call = Call {
        body: Some(serde_json::json!({"pad": "x".repeat(2000)})),
        ..Call::empty()
    };
    let Err(err) = prepared(&t, Some(&creds), EndpointId::OrdersPlace, call) else {
        panic!("oversized body accepted")
    };
    assert_eq!(
        err.validation().map(|v| &v.reason),
        Some(&ValidationReason::BodyTooLarge { max: 1024 })
    );
    assert_eq!(err.stage(), Stage::NotSent);

    let call = Call {
        body: Some(serde_json::json!([1])),
        ..Call::empty()
    };
    assert!(prepared(&t, Some(&creds), EndpointId::OrdersPlace, call).is_err());

    // A body-bearing endpoint called without a body is a facade bug, reported before sending.
    let Err(err) = prepared(&t, Some(&creds), EndpointId::OrdersPlace, Call::empty()) else {
        panic!("accepted")
    };
    assert_eq!(
        err.validation().map(|v| &v.reason),
        Some(&ValidationReason::Missing)
    );
}

#[test]
fn query_parameters_are_sent_as_given() {
    let t = transport(Environment::Live);
    let creds = credentials();
    let query = [
        ("from-date", Cow::Borrowed("2024-01-01")),
        ("to-date", Cow::Borrowed("2024-01-31")),
    ];
    let call = Call {
        query: &query,
        ..Call::empty()
    };
    let p = prepared(&t, Some(&creds), EndpointId::StatementsLedger, call).unwrap();
    let request = t.build(&p).unwrap();
    assert_eq!(
        request.url().as_str(),
        "https://api.dhan.co/v2/ledger?from-date=2024-01-01&to-date=2024-01-31"
    );
}

#[test]
fn path_argument_mismatch_is_a_validation_error() {
    let t = transport(Environment::Live);
    let Err(err) = prepared(
        &t,
        Some(&credentials()),
        EndpointId::OrdersGet,
        Call::empty(),
    ) else {
        panic!("accepted")
    };
    assert_eq!(
        (err.kind(), err.stage()),
        (ErrorKind::Validation, Stage::NotSent)
    );
}

/// A loopback HTTP/1.1 server that answers each request with the next scripted response and
/// counts the requests it saw.
async fn scripted_server(
    responses: Vec<&'static str>,
) -> (url::Url, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
        for response in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 16 * 1024];
            let mut read = 0;
            // Read the head, then any Content-Length body.
            loop {
                let n = socket.read(&mut buf[read..]).await.unwrap();
                read += n;
                let text = String::from_utf8_lossy(&buf[..read]).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if read >= end + 4 + length || n == 0 {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
    });
    (url::Url::parse(&format!("http://{addr}/v2")).unwrap(), hits)
}

const OK_JSON: &str = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n[]";
const UNAVAILABLE: &str =
    "HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
const DH905: &str = "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\ncontent-length: 52\r\nconnection: close\r\n\r\n{\"errorType\":\"Input_Exception\",\"errorCode\":\"DH-905\"}";

fn loopback(base: url::Url) -> Transport {
    let mut t = transport(Environment::Live);
    t.urls.rest = base;
    t
}

#[tokio::test]
async fn a_read_is_retried_after_503_and_then_succeeds() {
    let (base, hits) = scripted_server(vec![UNAVAILABLE, OK_JSON]).await;
    let t = loopback(base);
    let creds = credentials();
    let rows: Vec<serde_json::Value> = t
        .execute(Some(&creds), by_id(EndpointId::OrdersList), || {
            Ok(Call::empty())
        })
        .await
        .unwrap();
    assert!(rows.is_empty());
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(t.limiter.__outstanding(), 0);
}

#[tokio::test]
async fn a_mutation_gets_exactly_one_attempt() {
    let (base, hits) = scripted_server(vec![UNAVAILABLE, OK_JSON]).await;
    let t = loopback(base);
    let creds = credentials();
    let call = || {
        Ok(Call {
            body: Some(serde_json::json!({"quantity": 1})),
            ..Call::empty()
        })
    };
    let err = t
        .execute::<serde_json::Value>(Some(&creds), by_id(EndpointId::OrdersPlace), call)
        .await
        .unwrap_err();
    assert_eq!(
        (err.kind(), err.stage(), err.http_status()),
        (ErrorKind::HttpStatus, Stage::ResponseReceived, Some(503))
    );
    assert_eq!(err.attempts(), 1);
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_broker_error_is_not_retried() {
    let (base, hits) = scripted_server(vec![DH905, OK_JSON]).await;
    let t = loopback(base);
    let err = t
        .execute::<serde_json::Value>(Some(&credentials()), by_id(EndpointId::OrdersList), || {
            Ok(Call::empty())
        })
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Api);
    assert_eq!(
        err.api().and_then(|a| a.error_code.clone()),
        Some(ApiErrorCode::Dh905)
    );
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn validation_failures_send_nothing() {
    let (base, hits) = scripted_server(vec![OK_JSON]).await;
    let t = loopback(base);
    let invalid = || {
        Err(ValidationError::new(
            "quantity",
            ValidationReason::NotPositive,
        ))
    };
    let err = t
        .execute::<serde_json::Value>(
            Some(&credentials()),
            by_id(EndpointId::OrdersPlace),
            invalid,
        )
        .await
        .unwrap_err();
    assert_eq!(
        (err.kind(), err.stage()),
        (ErrorKind::Validation, Stage::NotSent)
    );
    assert!(!err.may_have_reached_server());
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(t.limiter.__outstanding(), 0);
}

const ACCEPTED_EMPTY: &str =
    "HTTP/1.1 202 Accepted\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
const OK_NULL: &str = "HTTP/1.1 200 OK\r\ncontent-length: 4\r\nconnection: close\r\n\r\nnull";
const OK_ACK: &str =
    "HTTP/1.1 200 OK\r\ncontent-length: 15\r\nconnection: close\r\n\r\n{\"orderId\":\"1\"}";
const OK_CSV: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/csv\r\ncontent-length: 8\r\nconnection: close\r\n\r\na,b\n1,2\n";

#[tokio::test]
async fn empty_optional_and_text_variants() {
    let (base, _) = scripted_server(vec![ACCEPTED_EMPTY, OK_NULL, OK_ACK]).await;
    let t = loopback(base);
    let creds = credentials();
    let convert = || {
        Ok(Call {
            body: Some(serde_json::json!({"quantity": 1})),
            ..Call::empty()
        })
    };
    t.execute_empty(
        Some(&creds),
        by_id(EndpointId::PortfolioConvertPosition),
        convert,
    )
    .await
    .unwrap();
    let leg = || {
        Ok(Call {
            path_args: &["1", "TARGET_LEG"],
            ..Call::empty()
        })
    };
    let none: Option<serde_json::Value> = t
        .execute_opt(Some(&creds), by_id(EndpointId::SuperOrdersCancelLeg), leg)
        .await
        .unwrap();
    assert_eq!(none, None);
    let some: Option<serde_json::Value> = t
        .execute_opt(Some(&creds), by_id(EndpointId::SuperOrdersCancelLeg), leg)
        .await
        .unwrap();
    assert_eq!(some, Some(serde_json::json!({"orderId": "1"})));

    let (base, _) = scripted_server(vec![OK_CSV]).await;
    let mut t = transport(Environment::Live);
    t.urls.scrip_master_compact = base;
    let text = t
        .execute_text(
            None,
            by_id(EndpointId::InstrumentsScripMasterCompact),
            || Ok(Call::empty()),
        )
        .await
        .unwrap();
    assert_eq!(text, "a,b\n1,2\n");
    assert!(matches!(t.environment, Environment::Live));
}

#[tokio::test]
async fn an_oversized_response_body_is_refused() {
    let (base, _) = scripted_server(vec![OK_CSV]).await;
    let mut t = transport(Environment::Live);
    t.urls.scrip_master_compact = base;
    t.settings.max_csv_response_bytes = 4;
    let err = t
        .execute_text(
            None,
            by_id(EndpointId::InstrumentsScripMasterCompact),
            || Ok(Call::empty()),
        )
        .await
        .unwrap_err();
    assert_eq!(
        (err.kind(), err.stage(), err.http_status()),
        (ErrorKind::Decode, Stage::ResponseReceived, Some(200))
    );
    assert_eq!(err.detail(), Some("response body exceeds its bound"));
}

#[tokio::test]
async fn decode_failures_report_attempts_and_status() {
    let (base, _) = scripted_server(vec![OK_JSON]).await;
    let t = loopback(base);
    let err = t
        .execute::<std::collections::BTreeMap<String, u32>>(
            Some(&credentials()),
            by_id(EndpointId::OrdersList),
            || Ok(Call::empty()),
        )
        .await
        .unwrap_err();
    assert_eq!(
        (err.kind(), err.attempts(), err.http_status()),
        (ErrorKind::Decode, 1, Some(200))
    );
}

/// Sends the response head and part of the body, then holds the connection open.
async fn stalling_body_server() -> url::Url {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 16 * 1024];
        // The request head fits one read; its content is irrelevant here.
        let _ = socket.read(&mut buf).await.unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 10\r\n\r\n[1,")
            .await
            .unwrap();
        // Hold the connection until the test ends.
        std::future::pending::<()>().await;
    });
    url::Url::parse(&format!("http://{addr}/v2")).unwrap()
}

// real-time: a paused clock auto-advances while the client waits on the socket, so the timeout
// could fire before the response head is read. A 1 s attempt bound leaves a wide margin.
#[tokio::test]
async fn a_body_that_stalls_after_the_status_is_not_retried() {
    let base = stalling_body_server().await;
    let mut t = loopback(base);
    t.settings.attempt_timeout = std::time::Duration::from_secs(1);
    let err = t
        .execute::<serde_json::Value>(Some(&credentials()), by_id(EndpointId::OrdersList), || {
            Ok(Call::empty())
        })
        .await
        .unwrap_err();
    assert_eq!(
        (err.kind(), err.stage(), err.http_status()),
        (ErrorKind::Timeout, Stage::ResponseReceived, Some(200))
    );
    assert!(err.is_timeout());
    assert_eq!(err.attempts(), 1);
    assert_eq!(err.detail(), None);
}

// real-time: a paused clock auto-advances while the client waits on the socket. The quote window
// is widened to 60 s, so however slow the host, the retry's admission is refused at once with
// max_wait = 0.
#[tokio::test]
async fn a_retry_refused_by_the_local_limiter_reports_one_attempt() {
    use crate::rest::ratelimit::{AdmissionLimits, QuotaProfile};
    let (base, hits) = scripted_server(vec![UNAVAILABLE, OK_JSON]).await;
    let mut t = loopback(base);
    use crate::labels::RateClass;
    use crate::rest::ratelimit::Window;
    let slow_quotes = QuotaProfile::dhan_v2()
        .with_windows(
            RateClass::Quote,
            vec![Window {
                limit: 1,
                period: crate::rest::ratelimit::WindowPeriod::Rolling(
                    std::time::Duration::from_secs(60),
                ),
            }],
        )
        .unwrap();
    t.limiter = RateLimiter::new(
        slow_quotes,
        AdmissionLimits::new(std::time::Duration::ZERO, 256).unwrap(),
    );
    let body = serde_json::json!({"NSE_EQ": [1333]});
    let err = t
        .execute::<serde_json::Value>(
            Some(&credentials()),
            by_id(EndpointId::MarketQuoteLtp),
            || {
                Ok(Call {
                    body: Some(body),
                    ..Call::empty()
                })
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        (err.kind(), err.stage(), err.attempts()),
        (ErrorKind::RateLimited, Stage::NotSent, 1)
    );
    assert_eq!(
        err.rate_limit().map(|r| r.source),
        Some(RateLimitSource::LocalWaitExceeded)
    );
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[path = "transport_telemetry_tests.rs"]
mod telemetry;
