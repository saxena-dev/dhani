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

use crate::backoff::SplitMix64;
use crate::config::{Environment, Urls};
use crate::credentials::Credentials;
use crate::error::{
    ApiError, ConfigError, Error, ErrorKind, RateLimitInfo, Stage, ValidationError,
    ValidationReason, classify_kind, unparsed_body_detail,
};
use crate::labels::{EndpointId, Method};
use crate::obs::Redactor;
use crate::rest::endpoint::{AuthMode, BodyPolicy, Endpoint, Host, ResponseShape};
use crate::rest::ratelimit::RateLimiter;
use crate::rest::retry::{self, Cause, Decision, RetryLimits};
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
    #[allow(
        dead_code,
        reason = "recorded on the request span by the observability layer"
    )]
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

/// A successful response, before decoding into the caller's type.
#[derive(Debug, PartialEq)]
pub(crate) enum Success {
    Json(Vec<u8>),
    Empty,
    JsonOrEmpty(Option<Vec<u8>>),
    Csv(String),
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

/// Classifies a response by status and the endpoint's declared shape (step 8 of the pipeline).
pub(crate) fn classify(
    ep: &Endpoint,
    status: u16,
    bytes: Vec<u8>,
    redactor: &Redactor,
) -> Result<Success, Error> {
    let blank = bytes.iter().all(u8::is_ascii_whitespace);
    if !(200..300).contains(&status) {
        let api = ApiError::parse(status, &bytes, redactor);
        let (kind, source) = classify_kind(status, api.as_ref());
        let mut error = Error::new(kind, Stage::ResponseReceived)
            .with_endpoint(ep.id)
            .with_status(status);
        error = match api {
            Some(api) => error.with_api(api),
            None => error.with_detail(redactor, &unparsed_body_detail(&bytes)),
        };
        if let Some(source) = source {
            error = error.with_rate_limit(RateLimitInfo {
                source,
                class: ep.rate,
                waited: Duration::ZERO,
            });
        }
        return Err(error);
    }
    let decode = |detail: &str| {
        Error::new(ErrorKind::Decode, Stage::ResponseReceived)
            .with_endpoint(ep.id)
            .with_status(status)
            .with_detail(redactor, detail)
    };
    match ep.response {
        ResponseShape::Json if blank => Err(decode("empty response body")),
        ResponseShape::Json => Ok(Success::Json(bytes)),
        ResponseShape::Empty
            if blank || serde_json::from_slice::<serde::de::IgnoredAny>(&bytes).is_ok() =>
        {
            Ok(Success::Empty)
        }
        ResponseShape::Empty => Err(decode("response body is not JSON")),
        ResponseShape::JsonOrEmpty => {
            let trimmed = bytes.trim_ascii();
            if blank || trimmed == b"{}" || trimmed == b"null" {
                Ok(Success::JsonOrEmpty(None))
            } else {
                Ok(Success::JsonOrEmpty(Some(bytes)))
            }
        }
        ResponseShape::Csv => String::from_utf8(bytes)
            .map(Success::Csv)
            .map_err(|_| decode("response body is not UTF-8")),
    }
}

/// Decodes a JSON success body; a mismatch reports only serde's category, line and column,
/// never its message (which quotes values).
pub(crate) fn decode_json<T: DeserializeOwned>(
    ep: &Endpoint,
    bytes: &[u8],
    redactor: &Redactor,
) -> Result<T, Error> {
    serde_json::from_slice(bytes).map_err(|e| {
        let category = match e.classify() {
            serde_json::error::Category::Io => "io",
            serde_json::error::Category::Syntax => "syntax",
            serde_json::error::Category::Data => "data",
            serde_json::error::Category::Eof => "eof",
        };
        let detail = format!("response body does not match the expected shape ({category} error at line {} column {})", e.line(), e.column());
        Error::new(ErrorKind::Decode, Stage::ResponseReceived).with_endpoint(ep.id).with_detail(redactor, &detail)
    })
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
            let result = self.attempt(ep, &prepared, deadline, &redactor).await;
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
            let error = error.with_attempts(attempt);
            let Some(cause) = cause else {
                return Err(error);
            };
            let now = Instant::now();
            match retry::decide(
                ep,
                cause,
                attempt,
                rate_limit_retries,
                now.into_std(),
                deadline.into_std(),
                &self.settings.retry,
                &mut rng,
            ) {
                Decision::Retry { delay } => {
                    if matches!(cause, Cause::RemoteRateLimit { .. }) {
                        rate_limit_retries += 1;
                    }
                    tokio::time::sleep(delay).await;
                }
                Decision::Stop => return Err(error),
            }
        }
    }

    /// One attempt: admission, send, bounded read and classification. On failure, also the retry
    /// cause, if the failure is retryable.
    async fn attempt(
        &self,
        ep: &'static Endpoint,
        p: &Prepared,
        deadline: Instant,
        redactor: &Redactor,
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
        let budget = self.settings.attempt_timeout.min(deadline - now);
        let request = self.build(p).map_err(|e| {
            (
                Error::new(ErrorKind::Transport, Stage::NotSent)
                    .with_endpoint(ep.id)
                    .with_source(e.without_url()),
                None,
            )
        })?;
        grant.dispatch();
        let timed_out = || {
            (
                Error::new(ErrorKind::Timeout, Stage::Sent)
                    .with_endpoint(ep.id)
                    .timed_out(),
                Some(Cause::Timeout),
            )
        };
        let exchange = async {
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
            let status = response.status().as_u16();
            let retry_after = response
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| retry::parse_retry_after(v, std::time::SystemTime::now()));
            let bytes = read_bounded(ep, response, self.max_body(ep)).await?;
            Ok((status, retry_after, bytes))
        };
        let (status, retry_after, bytes) = tokio::time::timeout(budget, exchange)
            .await
            .map_err(|_| timed_out())??;
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

    /// Decodes a `Json` endpoint's response into `T`.
    pub(crate) async fn execute<'a, T: DeserializeOwned>(
        &self,
        credentials: Option<&Credentials>,
        ep: &'static Endpoint,
        prepare: impl FnOnce() -> Result<Call<'a>, ValidationError>,
    ) -> Result<T, Error> {
        let done = self.run(credentials, ep, prepare).await?;
        let (attempts, status) = (done.attempts, done.status);
        let finish = move |e: Error| e.with_attempts(attempts).with_status(status);
        match (done.success, done.redactor) {
            (Success::Json(bytes), redactor) => decode_json(ep, &bytes, &redactor).map_err(finish),
            (other, _) => Err(finish(shape_mismatch(ep, &other))),
        }
    }

    /// Runs an `Empty` endpoint.
    pub(crate) async fn execute_empty<'a>(
        &self,
        credentials: Option<&Credentials>,
        ep: &'static Endpoint,
        prepare: impl FnOnce() -> Result<Call<'a>, ValidationError>,
    ) -> Result<(), Error> {
        let done = self.run(credentials, ep, prepare).await?;
        let (attempts, status) = (done.attempts, done.status);
        let finish = move |e: Error| e.with_attempts(attempts).with_status(status);
        match (done.success, done.redactor) {
            (Success::Empty, _) => Ok(()),
            (other, _) => Err(finish(shape_mismatch(ep, &other))),
        }
    }

    /// Runs a `JsonOrEmpty` endpoint: `None` for an empty body, `{}` or `null`.
    pub(crate) async fn execute_opt<'a, T: DeserializeOwned>(
        &self,
        credentials: Option<&Credentials>,
        ep: &'static Endpoint,
        prepare: impl FnOnce() -> Result<Call<'a>, ValidationError>,
    ) -> Result<Option<T>, Error> {
        let done = self.run(credentials, ep, prepare).await?;
        let (attempts, status) = (done.attempts, done.status);
        let finish = move |e: Error| e.with_attempts(attempts).with_status(status);
        match (done.success, done.redactor) {
            (Success::JsonOrEmpty(None), _) => Ok(None),
            (Success::JsonOrEmpty(Some(bytes)), redactor) => {
                decode_json(ep, &bytes, &redactor).map(Some).map_err(finish)
            }
            (other, _) => Err(finish(shape_mismatch(ep, &other))),
        }
    }

    /// Runs a `Csv` endpoint and returns the text.
    pub(crate) async fn execute_text<'a>(
        &self,
        credentials: Option<&Credentials>,
        ep: &'static Endpoint,
        prepare: impl FnOnce() -> Result<Call<'a>, ValidationError>,
    ) -> Result<String, Error> {
        let done = self.run(credentials, ep, prepare).await?;
        let (attempts, status) = (done.attempts, done.status);
        let finish = move |e: Error| e.with_attempts(attempts).with_status(status);
        match (done.success, done.redactor) {
            (Success::Csv(text), _) => Ok(text),
            (other, _) => Err(finish(shape_mismatch(ep, &other))),
        }
    }
}

/// A facade called the wrong `execute*` variant for its endpoint; nothing about the response is
/// reported.
fn shape_mismatch(ep: &Endpoint, _got: &Success) -> Error {
    Error::new(ErrorKind::Decode, Stage::ResponseReceived)
        .with_endpoint(ep.id)
        .with_detail(
            &Redactor::new(),
            "response shape does not match the endpoint",
        )
}

/// Reads the body in chunks, refusing more than `max` bytes (checked against Content-Length
/// first). A failure here arrived with a status, so it is never retried.
async fn read_bounded(
    ep: &Endpoint,
    mut response: reqwest::Response,
    max: usize,
) -> Result<Vec<u8>, (Error, Option<Cause>)> {
    let status = response.status().as_u16();
    let too_large = || {
        (
            Error::new(ErrorKind::Transport, Stage::ResponseReceived)
                .with_endpoint(ep.id)
                .with_status(status)
                .with_detail(
                    &Redactor::new(),
                    &format!("response body exceeds {max} bytes"),
                ),
            None,
        )
    };
    if response.content_length().is_some_and(|n| n > max as u64) {
        return Err(too_large());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| {
        (
            Error::new(ErrorKind::Transport, Stage::ResponseReceived)
                .with_endpoint(ep.id)
                .with_status(status)
                .with_source(e.without_url()),
            None,
        )
    })? {
        if body.len() + chunk.len() > max {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
