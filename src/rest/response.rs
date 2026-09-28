//! Response handling: bounded body reads, classification by status and declared shape
//! (the last step of a call), and JSON decoding that never quotes the body.

use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::error::{
    ApiError, Error, ErrorKind, RateLimitInfo, Stage, classify_kind, unparsed_body_detail,
};
use crate::obs::Redactor;
use crate::rest::endpoint::{Endpoint, ResponseShape};
use crate::rest::retry::Cause;

/// A successful response, before decoding into the caller's type.
#[derive(Debug, PartialEq)]
pub(crate) enum Success {
    Json(Vec<u8>),
    Empty,
    JsonOrEmpty(Option<Vec<u8>>),
    Csv(String),
}

/// Classifies a response by status and the endpoint's declared shape.
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
        ResponseShape::Empty if blank => Ok(Success::Empty),
        ResponseShape::Empty => match serde_json::from_slice::<serde_json::Value>(&bytes) {
            // A 2xx body that reports a failure is not a success: exit-all, for one,
            // answers 200 with {status, message}.
            Ok(serde_json::Value::Object(body)) if failed_status(&body).is_some() => {
                let error = Error::new(ErrorKind::Api, Stage::ResponseReceived)
                    .with_endpoint(ep.id)
                    .with_status(status);
                Err(match ApiError::parse(status, &bytes, redactor) {
                    Some(api) => error.with_api(api),
                    None => error.with_detail(
                        redactor,
                        &format!(
                            "the response reported status {:?}",
                            failed_status(&body).unwrap_or_default()
                        ),
                    ),
                })
            }
            Ok(_) => Ok(Success::Empty),
            Err(_) => Err(decode("response body is not JSON")),
        },
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

/// The top-level `status` string of a body, when it is present and not `success` (any case).
fn failed_status(body: &serde_json::Map<String, serde_json::Value>) -> Option<&str> {
    body.get("status")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.eq_ignore_ascii_case("success"))
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

/// A facade called the wrong `execute*` variant for its endpoint; nothing about the response is
/// reported.
pub(crate) fn shape_mismatch(ep: &Endpoint, _got: &Success) -> Error {
    Error::new(ErrorKind::Decode, Stage::ResponseReceived)
        .with_endpoint(ep.id)
        .with_detail(
            &Redactor::new(),
            "response shape does not match the endpoint",
        )
}

/// Reads the body in chunks, refusing more than `max` bytes (checked against Content-Length
/// first). A failure here arrived with a status, so it is never retried.
pub(crate) async fn read_bounded(
    ep: &Endpoint,
    mut response: reqwest::Response,
    max: usize,
) -> Result<Vec<u8>, (Error, Option<Cause>)> {
    let status = response.status().as_u16();
    let too_large = || {
        (
            Error::new(ErrorKind::Decode, Stage::ResponseReceived)
                .with_endpoint(ep.id)
                .with_status(status)
                .with_detail(&Redactor::new(), "response body exceeds its bound"),
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
