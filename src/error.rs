//! The crate error model: `Error`, `ErrorKind`, `Stage`, `ApiError`, `ApiErrorCode`,
//! `DataErrorCode`, `ValidationError`, `ValidationReason`, `ConfigError`, `RateLimitInfo`,
//! `RateLimitSource` and the `Result` alias.

use std::fmt;
use std::time::Duration;

use crate::labels::{EndpointId, RateClass};
use crate::obs::Redactor;
use crate::types::BoundedText;

mod parse;
#[allow(
    unused_imports,
    reason = "used by the transport's response classification when it lands"
)]
pub(crate) use parse::{classify_kind, unparsed_body_detail};

/// The crate's result type.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Every failure the crate reports.
///
/// Inspect it through [`kind`](Error::kind), [`stage`](Error::stage) and the other accessors.
/// `Display` is a diagnostic summary, not a stable contract. No error ever contains a URL,
/// header, request body, credential or raw response value; stored broker text is sanitised.
///
/// Errors are built only inside the crate:
///
/// ```compile_fail,E0423
/// let _ = dhani::Error(Box::new(()));
/// ```
pub struct Error(Box<ErrorInner>);

struct ErrorInner {
    kind: ErrorKind,
    endpoint: Option<EndpointId>,
    http_status: Option<u16>,
    api: Option<ApiError>,
    validation: Option<ValidationError>,
    config: Option<ConfigError>,
    rate_limit: Option<RateLimitInfo>,
    stage: Stage,
    attempts: u32,
    timed_out: bool,
    detail: Option<BoundedText>,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

/// What went wrong, at the level a caller branches on.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// Invalid builder values, missing credentials or an unsupported environment.
    Config,
    /// Invalid credential text.
    Credential,
    /// A request was rejected locally; nothing was sent.
    Validation,
    /// A local rate-limit refusal, or a remote 429, `DH-904` or data code 805.
    RateLimited,
    /// An attempt or operation deadline passed.
    Timeout,
    /// Connect, TLS or I/O failure, or a truncated body.
    Transport,
    /// HTTP 401, `DH-901` or data codes 807 to 810.
    Auth,
    /// Any other parsed broker error (`DH-9xx`, data codes 8xx).
    Api,
    /// A non-2xx response without a parsable error body.
    HttpStatus,
    /// A 2xx body that does not match the declared shape.
    Decode,
}

impl ErrorKind {
    /// The label used in `Display` and in telemetry.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Credential => "credential",
            Self::Validation => "validation",
            Self::RateLimited => "rate_limited",
            Self::Timeout => "timeout",
            Self::Transport => "transport",
            Self::Auth => "auth",
            Self::Api => "api",
            Self::HttpStatus => "http_status",
            Self::Decode => "decode",
        }
    }
}

/// How far a request got before it failed.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
    /// The request never left the process.
    NotSent,
    /// The request was (or may have been) sent; no response was received.
    Sent,
    /// A response was received.
    ResponseReceived,
}

impl Error {
    /// What went wrong.
    pub fn kind(&self) -> ErrorKind {
        self.0.kind
    }

    /// The endpoint the failing call targeted, if any.
    pub fn endpoint(&self) -> Option<EndpointId> {
        self.0.endpoint
    }

    /// The HTTP status of the response, if one was received.
    pub fn http_status(&self) -> Option<u16> {
        self.0.http_status
    }

    /// The parsed broker error, if the response carried one.
    pub fn api(&self) -> Option<&ApiError> {
        self.0.api.as_ref()
    }

    /// The local validation failure, for [`ErrorKind::Validation`].
    pub fn validation(&self) -> Option<&ValidationError> {
        self.0.validation.as_ref()
    }

    /// The configuration problem, for [`ErrorKind::Config`].
    pub fn config(&self) -> Option<&ConfigError> {
        self.0.config.as_ref()
    }

    /// Rate-limit details, for [`ErrorKind::RateLimited`].
    pub fn rate_limit(&self) -> Option<&RateLimitInfo> {
        self.0.rate_limit.as_ref()
    }

    /// How far the request got.
    pub fn stage(&self) -> Stage {
        self.0.stage
    }

    /// How many attempts were made.
    pub fn attempts(&self) -> u32 {
        self.0.attempts
    }

    /// Whether a deadline expired.
    pub fn is_timeout(&self) -> bool {
        self.0.timed_out
    }

    /// Sanitised diagnostic text of at most 512 bytes, ending in a truncation marker when cut.
    pub fn detail(&self) -> Option<&str> {
        self.0.detail.as_ref().map(BoundedText::as_str)
    }

    /// `false` only with affirmative proof that the request never left the process. A mutation
    /// that fails with `true` may have taken effect at the broker.
    pub fn may_have_reached_server(&self) -> bool {
        self.stage() != Stage::NotSent
    }

    /// `"<kind>[ <endpoint label>][ HTTP <status>][ <api code>]: <detail>"`, from the fields above
    /// only.
    fn render(&self) -> String {
        let inner = &self.0;
        let mut out = String::from(inner.kind.as_str());
        if let Some(endpoint) = inner.endpoint {
            out.push(' ');
            out.push_str(endpoint.as_str());
        }
        if let Some(status) = inner.http_status {
            out.push_str(&format!(" HTTP {status}"));
        }
        if let Some(code) = inner.api.as_ref().and_then(|a| a.error_code.as_ref()) {
            out.push(' ');
            out.push_str(&code.label());
        }
        if let Some(detail) = &inner.detail {
            out.push_str(": ");
            out.push_str(detail.as_str());
        }
        out
    }
}

/// Crate-private builders; users cannot construct an `Error`.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "used by the transport, limiter and facades as they land"
    )
)]
impl Error {
    pub(crate) fn new(kind: ErrorKind, stage: Stage) -> Self {
        Self(Box::new(ErrorInner {
            kind,
            endpoint: None,
            http_status: None,
            api: None,
            validation: None,
            config: None,
            rate_limit: None,
            stage,
            attempts: 0,
            timed_out: false,
            detail: None,
            source: None,
        }))
    }

    /// A local validation failure: nothing was sent.
    pub(crate) fn from_validation(error: ValidationError) -> Self {
        Self::new(ErrorKind::Validation, Stage::NotSent).with_validation(error)
    }

    /// A configuration failure: nothing was sent.
    pub(crate) fn from_config(error: ConfigError) -> Self {
        Self::new(ErrorKind::Config, Stage::NotSent).with_config(error)
    }

    pub(crate) fn with_kind(mut self, kind: ErrorKind) -> Self {
        self.0.kind = kind;
        self
    }

    pub(crate) fn with_endpoint(mut self, endpoint: EndpointId) -> Self {
        self.0.endpoint = Some(endpoint);
        self
    }

    pub(crate) fn with_status(mut self, status: u16) -> Self {
        self.0.http_status = Some(status);
        self
    }

    pub(crate) fn with_api(mut self, api: ApiError) -> Self {
        self.0.api = Some(api);
        self
    }

    pub(crate) fn with_validation(mut self, error: ValidationError) -> Self {
        self.0.validation = Some(error);
        self
    }

    pub(crate) fn with_config(mut self, error: ConfigError) -> Self {
        self.0.config = Some(error);
        self
    }

    pub(crate) fn with_rate_limit(mut self, info: RateLimitInfo) -> Self {
        self.0.rate_limit = Some(info);
        self
    }

    pub(crate) fn with_stage(mut self, stage: Stage) -> Self {
        self.0.stage = stage;
        self
    }

    pub(crate) fn with_attempts(mut self, attempts: u32) -> Self {
        self.0.attempts = attempts;
        self
    }

    pub(crate) fn timed_out(mut self) -> Self {
        self.0.timed_out = true;
        self
    }

    /// Stores `text` after sanitising it with `redactor` and bounding it to 512 bytes.
    pub(crate) fn with_detail(mut self, redactor: &Redactor, text: &str) -> Self {
        self.0.detail = Some(BoundedText::new(redactor.sanitize(text)));
        self
    }

    /// Keeps `source` for `Error::source()`; its text is never rendered or debug-printed here,
    /// but callers' reporters may print it. Only pass errors whose text holds no URL, header,
    /// body or response value: reqwest errors after `without_url()`, never a `serde_json`
    /// error (serde repeats the offending input).
    pub(crate) fn with_source(
        mut self,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        self.0.source = Some(Box::new(source));
        self
    }
}

impl fmt::Display for Error {
    /// A diagnostic summary (see `render`); not a stable contract.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0
            .source
            .as_deref()
            .map(|s| s as &(dyn std::error::Error + 'static))
    }
}

impl fmt::Debug for Error {
    /// Every field except the source's text; nothing here can hold a secret.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = &self.0;
        f.debug_struct("Error")
            .field("kind", &inner.kind)
            .field("endpoint", &inner.endpoint)
            .field("http_status", &inner.http_status)
            .field("api", &inner.api)
            .field("validation", &inner.validation)
            .field("config", &inner.config)
            .field("rate_limit", &inner.rate_limit)
            .field("stage", &inner.stage)
            .field("attempts", &inner.attempts)
            .field("timed_out", &inner.timed_out)
            .field("detail", &inner.detail)
            .field("has_source", &inner.source.is_some())
            .finish()
    }
}

/// A request value that failed local validation. Nothing is sent when validation fails.
///
/// `field` names the offending request field; the value itself is never included.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationError {
    /// The request field that failed validation.
    pub field: &'static str,
    /// Why it failed.
    pub reason: ValidationReason,
}

impl ValidationError {
    pub(crate) fn new(field: &'static str, reason: ValidationReason) -> Self {
        Self { field, reason }
    }
}

/// Why a request value failed validation.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationReason {
    /// A required value is absent.
    Missing,
    /// A value that must be non-empty is empty.
    Empty,
    /// A value is longer than `max` (characters, or bytes where the field says so).
    TooLong {
        /// The largest accepted length.
        max: usize,
    },
    /// A list has more than `max` entries.
    TooMany {
        /// The largest accepted count.
        max: usize,
    },
    /// A number is outside its accepted range.
    OutOfRange,
    /// A number is NaN or infinite.
    NotFinite,
    /// A number that must be positive is zero or negative.
    NotPositive,
    /// A value contains characters outside its accepted set.
    InvalidCharacters,
    /// An enum value is not one this build can send.
    UnknownEnumValue,
    /// Values are individually valid but inconsistent with each other; the text explains how.
    Inconsistent(&'static str),
    /// A request body is larger than `max` bytes.
    BodyTooLarge {
        /// The largest accepted body size in bytes.
        max: usize,
    },
}

/// An invalid configuration value. The value itself is never included.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{field}: {reason}")]
pub struct ConfigError {
    /// The configuration field.
    pub field: &'static str,
    /// Why it was rejected.
    pub reason: &'static str,
}

impl ConfigError {
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "used by the client builder and config validation")
    )]
    pub(crate) fn new(field: &'static str, reason: &'static str) -> Self {
        Self { field, reason }
    }
}

/// Details of a rate-limit failure.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RateLimitInfo {
    /// Who refused the request.
    pub source: RateLimitSource,
    /// The rate class the request was admitted under.
    pub class: RateClass,
    /// How long the request waited before failing.
    pub waited: Duration,
}

/// Who refused a request for rate-limit reasons.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateLimitSource {
    /// The local limiter would have had to wait past the allowed admission wait.
    LocalWaitExceeded,
    /// A local daily ceiling is exhausted.
    LocalCeiling,
    /// The broker refused the request (HTTP 429, `DH-904` or data code 805).
    Remote,
}

/// A parsed broker error body.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiError {
    /// The broker's error type. Its spelling varies between responses; never match on it.
    pub error_type: Option<String>,
    /// The broker's error code.
    pub error_code: Option<ApiErrorCode>,
    /// The broker's message, sanitised and bounded to 512 bytes.
    pub error_message: Option<BoundedText>,
}

/// A broker error code.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ApiErrorCode {
    /// `DH-901`: invalid authentication (DOC:4227).
    Dh901,
    /// `DH-902`: invalid access (DOC:4228).
    Dh902,
    /// `DH-903`: user account (DOC:4229).
    Dh903,
    /// `DH-904`: rate limit (DOC:4230).
    Dh904,
    /// `DH-905`: input exception (DOC:4231).
    Dh905,
    /// `DH-906`: order error (DOC:4232).
    Dh906,
    /// `DH-907`: data error (DOC:4233).
    Dh907,
    /// `DH-908`: internal server error (DOC:4234).
    Dh908,
    /// `DH-909`: network error (DOC:4235).
    Dh909,
    /// `DH-910`: others (DOC:4236).
    Dh910,
    /// A Data API code (DOC:4244-4255).
    Data(DataErrorCode),
    /// Any other code, kept as text (for example `E001`); bounded to 32 bytes.
    Other(String),
}

impl ApiErrorCode {
    const OTHER_MAX_BYTES: usize = 32;

    /// `Other` with `code` cut to at most 32 bytes on a character boundary.
    #[cfg_attr(not(test), allow(dead_code, reason = "used by ApiError::parse"))]
    pub(crate) fn other(code: &str) -> Self {
        let mut end = code.len().min(Self::OTHER_MAX_BYTES);
        while !code.is_char_boundary(end) {
            end -= 1;
        }
        Self::Other(code[..end].to_owned())
    }

    /// The code as the broker writes it: `DH-905`, `805` or the other text.
    fn label(&self) -> String {
        match self {
            Self::Data(code) => code.code().to_string(),
            Self::Other(text) => text.clone(),
            dh => {
                let n = match dh {
                    Self::Dh901 => 901,
                    Self::Dh902 => 902,
                    Self::Dh903 => 903,
                    Self::Dh904 => 904,
                    Self::Dh905 => 905,
                    Self::Dh906 => 906,
                    Self::Dh907 => 907,
                    Self::Dh908 => 908,
                    Self::Dh909 => 909,
                    _ => 910,
                };
                format!("DH-{n}")
            }
        }
    }
}

/// A Data API error code, also used as a feed disconnect reason (DOC:4244-4255).
///
/// Meanings follow the documentation where the Python SDK differs (808 and 809, Appendix A D18).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DataErrorCode {
    /// 800: internal server error.
    InternalServerError,
    /// 804: requested number of instruments exceeds the limit.
    InstrumentLimitExceeded,
    /// 805: too many requests or connections.
    TooManyRequests,
    /// 806: Data APIs not subscribed.
    DataApisNotSubscribed,
    /// 807: access token expired.
    AccessTokenExpired,
    /// 808: authentication failed (client ID or access token invalid).
    AuthenticationFailed,
    /// 809: access token invalid.
    AccessTokenInvalid,
    /// 810: client ID invalid.
    ClientIdInvalid,
    /// 811: invalid expiry date.
    InvalidExpiryDate,
    /// 812: invalid date format.
    InvalidDateFormat,
    /// 813: invalid security ID.
    InvalidSecurityId,
    /// 814: invalid request.
    InvalidRequest,
}

impl DataErrorCode {
    const ALL: [Self; 12] = [
        Self::InternalServerError,
        Self::InstrumentLimitExceeded,
        Self::TooManyRequests,
        Self::DataApisNotSubscribed,
        Self::AccessTokenExpired,
        Self::AuthenticationFailed,
        Self::AccessTokenInvalid,
        Self::ClientIdInvalid,
        Self::InvalidExpiryDate,
        Self::InvalidDateFormat,
        Self::InvalidSecurityId,
        Self::InvalidRequest,
    ];

    /// The code for a documented number, or `None`.
    pub fn from_u16(code: u16) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.code() == code)
    }

    /// The numeric code.
    pub fn code(self) -> u16 {
        match self {
            Self::InternalServerError => 800,
            Self::InstrumentLimitExceeded => 804,
            Self::TooManyRequests => 805,
            Self::DataApisNotSubscribed => 806,
            Self::AccessTokenExpired => 807,
            Self::AuthenticationFailed => 808,
            Self::AccessTokenInvalid => 809,
            Self::ClientIdInvalid => 810,
            Self::InvalidExpiryDate => 811,
            Self::InvalidDateFormat => 812,
            Self::InvalidSecurityId => 813,
            Self::InvalidRequest => 814,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SENTINEL_TOKEN: &str = "SENTINEL-ACCESS-TOKEN-7f3a";
    const SENTINEL_CLIENT: &str = "SENTINELCLIENT42";

    fn redactor() -> Redactor {
        let mut r = Redactor::new();
        r.register(SENTINEL_TOKEN);
        r.register(SENTINEL_CLIENT);
        r
    }

    #[test]
    fn error_is_pointer_sized() {
        assert_eq!(std::mem::size_of::<Error>(), std::mem::size_of::<usize>());
    }

    #[test]
    fn data_error_codes_round_trip() {
        assert_eq!(
            DataErrorCode::from_u16(805),
            Some(DataErrorCode::TooManyRequests)
        );
        assert_eq!(DataErrorCode::from_u16(803), None);
        assert_eq!(DataErrorCode::from_u16(815), None);
        let codes = [800, 804, 805, 806, 807, 808, 809, 810, 811, 812, 813, 814];
        for n in codes {
            assert_eq!(
                DataErrorCode::from_u16(n).map(DataErrorCode::code),
                Some(n),
                "{n}"
            );
        }
        assert_eq!(
            DataErrorCode::from_u16(808),
            Some(DataErrorCode::AuthenticationFailed)
        );
        assert_eq!(
            DataErrorCode::from_u16(809),
            Some(DataErrorCode::AccessTokenInvalid)
        );
        assert_eq!(
            DataErrorCode::from_u16(810),
            Some(DataErrorCode::ClientIdInvalid)
        );
    }

    #[test]
    fn render_follows_the_documented_shape() {
        let api = ApiError {
            error_type: Some("Input_Exception".to_owned()),
            error_code: Some(ApiErrorCode::Dh905),
            error_message: None,
        };
        let e = Error::new(ErrorKind::Api, Stage::ResponseReceived)
            .with_endpoint(EndpointId::OrdersPlace)
            .with_status(400)
            .with_api(api)
            .with_detail(&Redactor::new(), "missing field price");
        assert_eq!(
            e.to_string(),
            "api orders.place HTTP 400 DH-905: missing field price"
        );
        let bare = Error::new(ErrorKind::Timeout, Stage::Sent).timed_out();
        assert_eq!(bare.to_string(), "timeout");
        let data = Error::new(ErrorKind::RateLimited, Stage::ResponseReceived)
            .with_status(429)
            .with_api(ApiError {
                error_type: None,
                error_code: Some(ApiErrorCode::Data(DataErrorCode::TooManyRequests)),
                error_message: None,
            });
        assert_eq!(data.to_string(), "rate_limited HTTP 429 805");
    }

    #[test]
    fn detail_is_sanitised() {
        let e = Error::new(ErrorKind::Decode, Stage::ResponseReceived)
            .with_detail(&Redactor::new(), "token eyJaaaaaaaa.bbbbbbbbbb.cc");
        assert_eq!(e.detail(), Some("token <redacted>"));
        assert!(e.to_string().contains("<redacted>"));
    }

    #[test]
    fn seeded_sentinels_never_appear_in_display_or_debug() {
        struct Source;
        impl fmt::Debug for Source {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "source with {SENTINEL_TOKEN}")
            }
        }
        impl fmt::Display for Source {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "source with {SENTINEL_TOKEN}")
            }
        }
        impl std::error::Error for Source {}

        let r = redactor();
        let e = Error::new(ErrorKind::Auth, Stage::ResponseReceived)
            .with_status(401)
            .with_detail(
                &r,
                &format!("client {SENTINEL_CLIENT} token={SENTINEL_TOKEN} rejected"),
            )
            .with_source(Source);
        for text in [e.to_string(), format!("{e:?}"), format!("{e:#?}")] {
            assert!(!text.contains(SENTINEL_TOKEN), "{text}");
            assert!(!text.contains(SENTINEL_CLIENT), "{text}");
        }
        assert!(format!("{e:?}").contains("has_source: true"));
        let source = std::error::Error::source(&e).expect("source is kept");
        assert!(
            source.to_string().contains(SENTINEL_TOKEN),
            "the source itself is available to callers"
        );
    }

    #[test]
    fn accessors_and_stage() {
        let v = ValidationError::new("quantity", ValidationReason::NotPositive);
        let e = Error::from_validation(v.clone()).with_endpoint(EndpointId::OrdersPlace);
        assert_eq!(e.kind(), ErrorKind::Validation);
        assert_eq!(e.stage(), Stage::NotSent);
        assert!(!e.may_have_reached_server());
        assert_eq!(e.validation(), Some(&v));
        assert_eq!(e.endpoint(), Some(EndpointId::OrdersPlace));
        assert_eq!(e.attempts(), 0);
        assert!(!e.is_timeout());
        assert_eq!(e.http_status(), None);
        assert_eq!(e.api(), None);
        assert_eq!(e.detail(), None);

        let c = Error::from_config(ConfigError::new(
            "timeouts.connect",
            "must be between 100 ms and 60 s",
        ));
        assert_eq!(c.kind(), ErrorKind::Config);
        assert_eq!(
            c.config().unwrap().to_string(),
            "timeouts.connect: must be between 100 ms and 60 s"
        );

        let info = RateLimitInfo {
            source: RateLimitSource::Remote,
            class: RateClass::Order,
            waited: Duration::ZERO,
        };
        let r = Error::new(ErrorKind::RateLimited, Stage::Sent)
            .with_rate_limit(info.clone())
            .with_attempts(2)
            .with_stage(Stage::ResponseReceived)
            .with_kind(ErrorKind::RateLimited);
        assert_eq!(r.rate_limit(), Some(&info));
        assert_eq!(r.attempts(), 2);
        assert!(r.may_have_reached_server());
    }

    #[test]
    fn other_codes_are_bounded() {
        assert_eq!(
            ApiErrorCode::other("E001"),
            ApiErrorCode::Other("E001".to_owned())
        );
        let long = format!("{}\u{20ac}", "x".repeat(31));
        assert_eq!(
            ApiErrorCode::other(&long),
            ApiErrorCode::Other("x".repeat(31))
        );
        assert_eq!(ApiErrorCode::Dh901.label(), "DH-901");
        assert_eq!(ApiErrorCode::Dh910.label(), "DH-910");
    }
}
