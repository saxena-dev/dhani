//! `Transport`: the request pipeline, `classify()` and bounded body reads; also `Call`,
//! `HeaderSecret` and `OptionChainKey`.
//!
//! `Transport::execute` and its variants are the only code that touches the network. Each call
//! validates its request, checks credentials, builds the URL, headers and body, then runs an
//! attempt loop: admission, send with a per-attempt timeout, a bounded body read, classification
//! and the retry decision. Errors record how far the request got (`Stage`) and never carry a URL,
//! header, body or raw response text.
#![cfg_attr(
    not(test),
    allow(dead_code, reason = "wrapped by DhanClient and the facades")
)]

use std::borrow::Cow;
use std::sync::Mutex;
use std::time::Duration;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::header::{
    ACCEPT, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, RETRY_AFTER, USER_AGENT,
};
use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;
use tokio::time::Instant;
use tracing::{Instrument, Span};

use crate::backoff::SplitMix64;
use crate::config::{Environment, Urls};
use crate::credentials::Credentials;
use crate::error::{
    ConfigError, Error, ErrorKind, RateLimitSource, Stage, ValidationError, ValidationReason,
};
use crate::labels::{EndpointId, Method};
use crate::obs::{Redactor, spans};
use crate::rest::endpoint::{AuthMode, BodyPolicy, Endpoint, Host, ResponseShape};
use crate::rest::ratelimit::RateLimiter;
use crate::rest::response::{Success, classify, decode_json, read_bounded, shape_mismatch};
use crate::rest::retry::{self, Cause, Decision, RetryLimits};
use crate::rest::telemetry;
use crate::types::{ExchangeSegment, OrderId};

/// Characters left unescaped in a path segment: ASCII alphanumerics and `-`, `_`, `.`, `~`.
pub(crate) const PATH_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// One request as a facade describes it, after validation.
pub(crate) struct Call<'a> {
    /// Filled into the endpoint's path template in order.
    pub path_args: &'a [&'a str],
    pub query: &'a [(&'static str, Cow<'a, str>)],
    /// Already validated and serialised by the facade.
    pub body: Option<serde_json::Value>,
    /// `app_id`/`app_secret`, `partner_id`/`partner_secret` and similar per-call headers.
    pub extra_headers: &'a [(&'static str, HeaderSecret<'a>)],
    pub option_chain_key: Option<OptionChainKey>,
    /// Span field and modification-cap key.
    pub order_id: Option<&'a OrderId>,
    /// Span field only.
    pub correlation_id: Option<&'a str>,
    /// Per-call secrets masked in any stored broker text.
    pub secrets: &'a [&'a SecretString],
}

impl Call<'_> {
    /// A call with no arguments, headers, body or secrets.
    pub(crate) fn empty() -> Self {
        Call {
            path_args: &[],
            query: &[],
            body: None,
            extra_headers: &[],
            option_chain_key: None,
            order_id: None,
            correlation_id: None,
            secrets: &[],
        }
    }
}

/// A header value that may be secret. `Secret` values are sent marked sensitive.
pub(crate) enum HeaderSecret<'a> {
    Plain(&'a str),
    Secret(&'a SecretString),
}

/// Key of the one-request-per-three-seconds option-chain window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct OptionChainKey {
    pub scrip: u32,
    pub segment: ExchangeSegment,
    pub expiry: chrono::NaiveDate,
}

/// The transport's numeric settings, taken from the client's configuration types.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TransportSettings {
    /// Bound on one attempt (send and body read).
    pub attempt_timeout: Duration,
    /// Bound on the whole operation: admission, attempts and backoff.
    pub operation_timeout: Duration,
    pub retry: RetryLimits,
    pub max_request_bytes: usize,
    pub max_json_response_bytes: usize,
    pub max_csv_response_bytes: usize,
}

/// The shared HTTP machinery behind every `DhanClient` handle.
pub(crate) struct Transport {
    pub(crate) http: reqwest::Client,
    pub(crate) urls: Urls,
    #[allow(dead_code, reason = "reported by DhanClient::environment")]
    pub(crate) environment: Environment,
    pub(crate) settings: TransportSettings,
    pub(crate) limiter: RateLimiter,
    /// Seeds one generator per operation; poison-tolerant.
    pub(crate) jitter: Mutex<SplitMix64>,
    pub(crate) user_agent: HeaderValue,
}

/// A classified success with what the caller needs to finish decoding.
struct Done {
    success: Success,
    redactor: Redactor,
    attempts: u32,
    status: u16,
}

/// Fills `{…}` placeholders of `template` with `args` in order, percent-encoding each argument.
/// `None` if the number of arguments does not match.
pub(crate) fn fill_path(template: &str, args: &[&str]) -> Option<String> {
    let mut out = String::with_capacity(template.len() + 16);
    let mut args = args.iter();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let close = open + rest[open..].find('}')?;
        out.push_str(&rest[..open]);
        let arg = args.next()?;
        // Empty and dot segments would change the path's shape once normalised.
        if matches!(*arg, "" | "." | "..") {
            return None;
        }
        out.extend(utf8_percent_encode(arg, PATH_SEGMENT));
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    args.next().is_none().then_some(out)
}

/// The redactor for one call: the client's credentials plus the call's own secrets.
fn call_redactor(credentials: Option<&Credentials>, secrets: &[&SecretString]) -> Redactor {
    let mut redactor = Redactor::new();
    if let Some(c) = credentials {
        redactor.register(c.client_id().expose_secret());
        redactor.register(c.access_token().expose_secret());
    }
    for s in secrets {
        redactor.register(s.expose_secret());
    }
    redactor
}

fn header(value: &str, sensitive: bool) -> Result<HeaderValue, Error> {
    let mut v = HeaderValue::from_str(value).map_err(|_| {
        Error::new(ErrorKind::Credential, Stage::NotSent).with_detail(
            &Redactor::new(),
            "a header value contains characters that cannot be sent",
        )
    })?;
    v.set_sensitive(sensitive);
    Ok(v)
}

/// A request fully prepared before admission.
struct Prepared {
    method: reqwest::Method,
    url: url::Url,
    query: Vec<(&'static str, String)>,
    headers: HeaderMap,
    body: Option<Vec<u8>>,
    key: Option<OptionChainKey>,
    order_id: Option<OrderId>,
}

impl Transport {
    /// A transport over `http`; the client builder supplies validated settings.
    pub(crate) fn new(
        http: reqwest::Client,
        urls: Urls,
        environment: Environment,
        settings: TransportSettings,
        limiter: RateLimiter,
        user_agent: HeaderValue,
        jitter_seed: u64,
    ) -> Self {
        Transport {
            http,
            urls,
            environment,
            settings,
            limiter,
            jitter: Mutex::new(SplitMix64::new(jitter_seed)),
            user_agent,
        }
    }

    /// Steps 2–5 of the pipeline: credentials, URL, headers and body. Nothing is sent.
    fn prepare(
        &self,
        credentials: Option<&Credentials>,
        ep: &'static Endpoint,
        call: Call<'_>,
        redactor: &Redactor,
    ) -> Result<Prepared, Error> {
        let not_sent = |kind| Error::new(kind, Stage::NotSent).with_endpoint(ep.id);
        let creds = match ep.auth {
            AuthMode::AccessToken | AuthMode::AccessTokenAndClientIdHeader => {
                Some(credentials.ok_or_else(|| {
                    Error::from_config(ConfigError::new("credentials", "credentials required"))
                        .with_endpoint(ep.id)
                })?)
            }
            AuthMode::AppCredentials | AuthMode::PartnerCredentials | AuthMode::None => None,
        };
        let base = match ep.host {
            Host::Rest => &self.urls.rest,
            Host::Auth => &self.urls.auth,
            Host::ScripMaster => match ep.id {
                EndpointId::InstrumentsScripMasterDetailed => &self.urls.scrip_master_detailed,
                EndpointId::InstrumentsGlobalScripMaster => &self.urls.global_scrip_master,
                _ => &self.urls.scrip_master_compact,
            },
        };
        let path = fill_path(ep.path, call.path_args).ok_or_else(|| {
            Error::from_validation(ValidationError::new(
                "path",
                ValidationReason::Inconsistent("path arguments do not match the endpoint"),
            ))
            .with_endpoint(ep.id)
        })?;
        // Append rather than join, so a base path such as /v2 is kept.
        let url = url::Url::parse(&format!("{}{path}", base.as_str().trim_end_matches('/')))
            .map_err(|_| {
                not_sent(ErrorKind::Config).with_detail(redactor, "invalid request URL")
            })?;

        let mut headers = HeaderMap::new();
        if let Some(c) = creds {
            headers.insert(
                HeaderName::from_static("access-token"),
                header(c.access_token().expose_secret(), true)?,
            );
            headers.insert(
                HeaderName::from_static("client-id"),
                header(c.client_id().expose_secret(), true)?,
            );
            if matches!(ep.auth, AuthMode::AccessTokenAndClientIdHeader) {
                headers.insert(
                    HeaderName::from_static("dhanclientid"),
                    header(c.client_id().expose_secret(), true)?,
                );
            }
        }
        for (name, value) in call.extra_headers {
            let (text, sensitive) = match value {
                HeaderSecret::Plain(p) => (*p, false),
                HeaderSecret::Secret(s) => (s.expose_secret(), true),
            };
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
                Error::from_validation(ValidationError::new(
                    "header",
                    ValidationReason::InvalidCharacters,
                ))
                .with_endpoint(ep.id)
            })?;
            headers.insert(name, header(text, sensitive)?);
        }
        if !matches!(ep.response, ResponseShape::Csv) {
            headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        }
        headers.insert(USER_AGENT, self.user_agent.clone());

        let body = match (ep.body, call.body) {
            (BodyPolicy::None, None) => None,
            (BodyPolicy::None, Some(_)) => {
                return Err(Error::from_validation(ValidationError::new(
                    "body",
                    ValidationReason::Inconsistent("this endpoint takes no body"),
                ))
                .with_endpoint(ep.id));
            }
            (BodyPolicy::Json | BodyPolicy::JsonWithClientId, None) => {
                return Err(Error::from_validation(ValidationError::new(
                    "body",
                    ValidationReason::Missing,
                ))
                .with_endpoint(ep.id));
            }
            (BodyPolicy::Json, body) => body,
            (BodyPolicy::JsonWithClientId, Some(mut body)) => {
                let object = body.as_object_mut().ok_or_else(|| {
                    Error::from_validation(ValidationError::new(
                        "body",
                        ValidationReason::Inconsistent("the body must be a JSON object"),
                    ))
                    .with_endpoint(ep.id)
                })?;
                if let Some(c) = creds {
                    object.insert(
                        "dhanClientId".to_owned(),
                        serde_json::Value::String(c.client_id().expose_secret().to_owned()),
                    );
                }
                Some(body)
            }
        };
        let body = match body {
            Some(value) => {
                let bytes = serde_json::to_vec(&value).map_err(|_| {
                    not_sent(ErrorKind::Validation)
                        .with_detail(redactor, "request body could not be serialised")
                })?;
                let max = self.settings.max_request_bytes;
                if bytes.len() > max {
                    return Err(Error::from_validation(ValidationError::new(
                        "body",
                        ValidationReason::BodyTooLarge { max },
                    ))
                    .with_endpoint(ep.id));
                }
                headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
                Some(bytes)
            }
            None => None,
        };
        let method = match ep.method {
            Method::Get => reqwest::Method::GET,
            Method::Post => reqwest::Method::POST,
            Method::Put => reqwest::Method::PUT,
            Method::Delete => reqwest::Method::DELETE,
        };
        Ok(Prepared {
            method,
            url,
            query: call
                .query
                .iter()
                .map(|(k, v)| (*k, v.to_string()))
                .collect(),
            headers,
            body,
            key: call.option_chain_key,
            order_id: call.order_id.cloned(),
        })
    }

    fn build(&self, p: &Prepared) -> Result<reqwest::Request, reqwest::Error> {
        let mut b = self
            .http
            .request(p.method.clone(), p.url.clone())
            .headers(p.headers.clone());
        if !p.query.is_empty() {
            b = b.query(&p.query);
        }
        if let Some(body) = &p.body {
            b = b.body(body.clone());
        }
        b.build()
    }

    /// Steps 1–9 of the pipeline, ending in a classified success.
    async fn run<'a>(
        &self,
        credentials: Option<&Credentials>,
        ep: &'static Endpoint,
        prepare: impl FnOnce() -> Result<Call<'a>, ValidationError>,
    ) -> Result<Done, Error> {
        let start = Instant::now();
        let deadline = start + self.settings.operation_timeout;
        let call = prepare().map_err(|v| Error::from_validation(v).with_endpoint(ep.id))?;
        let span = Span::current();
        if let Some(id) = call.order_id {
            span.record("order_id", id.as_ref());
        }
        if let Some(id) = call.correlation_id {
            span.record("correlation_id", id);
        }
        let redactor = call_redactor(credentials, call.secrets);
        let prepared = self.prepare(credentials, ep, call, &redactor)?;
        let seed = self
            .jitter
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .next_u64();
        let mut rng = SplitMix64::new(seed);
        let mut attempt = 0u32;
        let mut rate_limit_retries = 0u32;
        loop {
            attempt += 1;
            let result = self
                .attempt(ep, &prepared, deadline, &redactor, attempt)
                .await;
            let (error, cause) = match result {
                Ok((success, status)) => {
                    return Ok(Done {
                        success,
                        redactor,
                        attempts: attempt,
                        status,
                    });
                }
                Err(failure) => failure,
            };
            // A local rate-limit refusal happens before anything is handed to the HTTP client, so
            // it is not an attempt.
            let refused_locally = matches!(
                error.rate_limit().map(|r| r.source),
                Some(RateLimitSource::LocalWaitExceeded | RateLimitSource::LocalCeiling)
            );
            let error = error.with_attempts(attempt - u32::from(refused_locally));
            let Some(cause) = cause else {
                return Err(error);
            };
            let now = Instant::now();
            let decision = retry::decide(
                ep,
                cause,
                attempt,
                rate_limit_retries,
                now.into_std(),
                deadline.into_std(),
                &self.settings.retry,
                &mut rng,
            );
            if let Cause::RemoteRateLimit { retry_after } = cause {
                let will_retry = matches!(decision, Decision::Retry { .. });
                telemetry::remote_rate_limited(ep, &error, will_retry, retry_after);
            }
            match decision {
                Decision::Retry { delay } => {
                    if matches!(cause, Cause::RemoteRateLimit { .. }) {
                        rate_limit_retries += 1;
                    }
                    telemetry::retry_scheduled(ep, attempt, cause, delay);
                    tokio::time::sleep(delay).await;
                }
                Decision::Stop => return Err(error),
            }
        }
    }

    /// One attempt: admission, send, bounded read and classification. On failure, also the retry
    /// cause, if the failure is retryable. `number` is 1-based.
    async fn attempt(
        &self,
        ep: &'static Endpoint,
        p: &Prepared,
        deadline: Instant,
        redactor: &Redactor,
        number: u32,
    ) -> Result<(Success, u16), (Error, Option<Cause>)> {
        let mut grant = self
            .limiter
            .acquire(ep, p.key, p.order_id.as_ref(), deadline)
            .await
            .map_err(|e| (e, None))?;
        let now = Instant::now();
        if now >= deadline {
            return Err((
                Error::new(ErrorKind::Timeout, Stage::NotSent)
                    .with_endpoint(ep.id)
                    .timed_out(),
                None,
            ));
        }
        let request = self.build(p).map_err(|e| {
            (
                Error::new(ErrorKind::Transport, Stage::NotSent)
                    .with_endpoint(ep.id)
                    .with_source(e.without_url()),
                None,
            )
        })?;
        grant.dispatch();
        let span = spans::http_attempt(&Span::current(), number);
        let started = Instant::now();
        let result = self
            .exchange(ep, request, deadline, redactor)
            .instrument(span.clone())
            .await;
        let outcome = result
            .as_ref()
            .map(|(_, status)| *status)
            .map_err(|(e, _)| e);
        telemetry::attempt_finished(&span, ep, outcome, started.elapsed());
        result
    }

    /// Sends a dispatched request, reads its body within the attempt deadline and classifies it.
    async fn exchange(
        &self,
        ep: &'static Endpoint,
        request: reqwest::Request,
        deadline: Instant,
        redactor: &Redactor,
    ) -> Result<(Success, u16), (Error, Option<Cause>)> {
        let timed_out = || {
            (
                Error::new(ErrorKind::Timeout, Stage::Sent)
                    .with_endpoint(ep.id)
                    .timed_out(),
                Some(Cause::Timeout),
            )
        };
        // One deadline bounds the send and the body read together, never past the operation.
        let attempt_deadline = (Instant::now() + self.settings.attempt_timeout).min(deadline);
        let send = async {
            let response = self.http.execute(request).await.map_err(|e| {
                // Only a connect failure proves the request never left the process.
                let stage = if e.is_connect() {
                    Stage::NotSent
                } else {
                    Stage::Sent
                };
                let error = if e.is_timeout() {
                    Error::new(ErrorKind::Timeout, stage).timed_out()
                } else {
                    Error::new(ErrorKind::Transport, stage)
                };
                let cause = if e.is_timeout() {
                    Cause::Timeout
                } else {
                    Cause::Transport
                };
                (
                    error.with_endpoint(ep.id).with_source(e.without_url()),
                    Some(cause),
                )
            })?;
            Ok(response)
        };
        let response = tokio::time::timeout_at(attempt_deadline, send)
            .await
            .map_err(|_| timed_out())??;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|v| retry::parse_retry_after(v, std::time::SystemTime::now()));
        // A response has arrived: a body that stalls past the attempt deadline is reported with
        // its status and is not retried.
        let bytes = tokio::time::timeout_at(
            attempt_deadline,
            read_bounded(ep, response, self.max_body(ep)),
        )
        .await
        .map_err(|_| {
            (
                Error::new(ErrorKind::Timeout, Stage::ResponseReceived)
                    .with_endpoint(ep.id)
                    .with_status(status)
                    .timed_out(),
                None,
            )
        })??;
        classify(ep, status, bytes, redactor)
            .map(|s| (s, status))
            .map_err(|error| {
                let cause = match (error.kind(), status) {
                    (ErrorKind::RateLimited, _) => Some(Cause::RemoteRateLimit { retry_after }),
                    (ErrorKind::HttpStatus, 502) => Some(Cause::Status502),
                    (ErrorKind::HttpStatus, 503) => Some(Cause::Status503),
                    (ErrorKind::HttpStatus, 504) => Some(Cause::Status504),
                    _ => None,
                };
                (error, cause)
            })
    }

    fn max_body(&self, ep: &Endpoint) -> usize {
        match ep.response {
            ResponseShape::Csv => self.settings.max_csv_response_bytes,
            _ => self.settings.max_json_response_bytes,
        }
    }

    /// Runs `body` inside a new `dhani.http.request` span (opened before the call is prepared)
    /// and finishes the span with the call's one terminal event.
    async fn traced<T>(
        &self,
        ep: &'static Endpoint,
        body: impl Future<Output = Result<(T, u16, u32), Error>>,
    ) -> Result<T, Error> {
        let span = spans::http_request(ep.id, ep.method, ep.rate, ep.retry, None, None);
        let started = Instant::now();
        // A caller that drops the call mid-flight gets no terminal event; the span records why.
        let mut pending = telemetry::Pending::new(&span, started);
        let result = body.instrument(span.clone()).await;
        pending.finished();
        let outcome = result
            .as_ref()
            .map(|(_, status, attempts)| (*status, *attempts));
        telemetry::request_finished(&span, ep, outcome, started.elapsed());
        result.map(|(value, _, _)| value)
    }

    /// Decodes a `Json` endpoint's response into `T`.
    pub(crate) async fn execute<'a, T: DeserializeOwned>(
        &self,
        credentials: Option<&Credentials>,
        ep: &'static Endpoint,
        prepare: impl FnOnce() -> Result<Call<'a>, ValidationError>,
    ) -> Result<T, Error> {
        self.traced(ep, async {
            let done = self.run(credentials, ep, prepare).await?;
            let value = match &done.success {
                Success::Json(bytes) => decode_json(ep, bytes, &done.redactor),
                other => Err(shape_mismatch(ep, other)),
            };
            done.finish(value)
        })
        .await
    }

    /// Runs an `Empty` endpoint.
    pub(crate) async fn execute_empty<'a>(
        &self,
        credentials: Option<&Credentials>,
        ep: &'static Endpoint,
        prepare: impl FnOnce() -> Result<Call<'a>, ValidationError>,
    ) -> Result<(), Error> {
        self.traced(ep, async {
            let done = self.run(credentials, ep, prepare).await?;
            let value = match &done.success {
                Success::Empty => Ok(()),
                other => Err(shape_mismatch(ep, other)),
            };
            done.finish(value)
        })
        .await
    }

    /// Runs a `JsonOrEmpty` endpoint: `None` for an empty body, `{}` or `null`.
    pub(crate) async fn execute_opt<'a, T: DeserializeOwned>(
        &self,
        credentials: Option<&Credentials>,
        ep: &'static Endpoint,
        prepare: impl FnOnce() -> Result<Call<'a>, ValidationError>,
    ) -> Result<Option<T>, Error> {
        self.traced(ep, async {
            let done = self.run(credentials, ep, prepare).await?;
            let value = match &done.success {
                Success::JsonOrEmpty(None) => Ok(None),
                Success::JsonOrEmpty(Some(bytes)) => {
                    decode_json(ep, bytes, &done.redactor).map(Some)
                }
                other => Err(shape_mismatch(ep, other)),
            };
            done.finish(value)
        })
        .await
    }

    /// Runs a `Csv` endpoint and returns the text.
    pub(crate) async fn execute_text<'a>(
        &self,
        credentials: Option<&Credentials>,
        ep: &'static Endpoint,
        prepare: impl FnOnce() -> Result<Call<'a>, ValidationError>,
    ) -> Result<String, Error> {
        self.traced(ep, async {
            let mut done = self.run(credentials, ep, prepare).await?;
            let value = match std::mem::replace(&mut done.success, Success::Empty) {
                Success::Csv(text) => Ok(text),
                other => Err(shape_mismatch(ep, &other)),
            };
            done.finish(value)
        })
        .await
    }

    /// Runs any endpoint and returns its body as raw JSON: a CSV body as a JSON string, an
    /// empty body as `None`. Backs the client's test hook.
    pub(crate) async fn execute_raw<'a>(
        &self,
        credentials: Option<&Credentials>,
        ep: &'static Endpoint,
        prepare: impl FnOnce() -> Result<Call<'a>, ValidationError>,
    ) -> Result<Option<crate::types::RawJson>, Error> {
        self.traced(ep, async {
            let mut done = self.run(credentials, ep, prepare).await?;
            let value = match std::mem::replace(&mut done.success, Success::Empty) {
                Success::Json(bytes) | Success::JsonOrEmpty(Some(bytes)) => {
                    decode_json(ep, &bytes, &done.redactor).map(Some)
                }
                Success::Empty | Success::JsonOrEmpty(None) => Ok(None),
                Success::Csv(text) => {
                    Ok(Some(crate::types::RawJson(serde_json::Value::String(text))))
                }
            };
            done.finish(value)
        })
        .await
    }
}

impl Done {
    /// Attaches the attempt count and status to a decoding outcome.
    fn finish<T>(&self, value: Result<T, Error>) -> Result<(T, u16, u32), Error> {
        match value {
            Ok(v) => Ok((v, self.status, self.attempts)),
            Err(e) => Err(e.with_attempts(self.attempts).with_status(self.status)),
        }
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
