//! Parsing of broker error bodies and the mapping of non-2xx responses to an [`ErrorKind`].

use serde_json::Value;

use super::{ApiError, ApiErrorCode, DataErrorCode, ErrorKind, RateLimitSource};
use crate::obs::Redactor;
use crate::types::BoundedText;

/// The camelCase keys of error shapes 1 and 2.
const ERROR_KEYS: [&str; 3] = ["errorType", "errorCode", "errorMessage"];

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "used by the transport's response classification")
)]
impl ApiError {
    /// Parses a broker error body. Three shapes are documented:
    ///
    /// 1. `{"errorType","errorCode","errorMessage"}` (DOC:465-470, DOC:8819-8872), where
    ///    `errorMessage` may be absent (DOC:8872);
    /// 2. the same fields plus `"status":"failure"` (DOC:5252-5260);
    /// 3. `{"data":{"<numeric code>":"<message>"}}`, the Data API style (DOC:8895-8898).
    ///
    /// Shapes 1 and 2 are chosen when any of the three camelCase keys is present, shape 3 when
    /// `data` is an object with exactly one key that parses as a `u16`. Anything else is `None`.
    /// Broker text is sanitised with `redactor` and bounded to 512 bytes. `status` is not used to
    /// choose a shape.
    pub(crate) fn parse(_status: u16, bytes: &[u8], redactor: &Redactor) -> Option<ApiError> {
        let Value::Object(body) = serde_json::from_slice::<Value>(bytes).ok()? else {
            return None;
        };
        let text = |v: &Value| match v {
            Value::String(s) => Some(s.clone()),
            // Only an integer is a plausible numeric code or message; other numbers are ignored.
            Value::Number(n) => n.as_u64().map(|n| n.to_string()),
            _ => None,
        };
        let bounded = |s: &str| BoundedText::new(redactor.sanitize(s));
        if ERROR_KEYS.iter().any(|k| body.contains_key(*k)) {
            return Some(ApiError {
                error_type: body
                    .get("errorType")
                    .and_then(text)
                    .map(|s| bounded(&s).as_str().to_owned()),
                error_code: body
                    .get("errorCode")
                    .and_then(text)
                    .map(|s| code_from_text(&s, redactor)),
                error_message: body.get("errorMessage").and_then(text).map(|s| bounded(&s)),
            });
        }
        let Some(Value::Object(data)) = body.get("data") else {
            return None;
        };
        let mut entries = data.iter();
        let (Some((key, message)), None) = (entries.next(), entries.next()) else {
            return None;
        };
        // `u16::from_str` accepts a leading '+', which a documented code never has.
        if key.is_empty() || !key.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let number: u16 = key.parse().ok()?;
        Some(ApiError {
            error_type: None,
            error_code: Some(DataErrorCode::from_u16(number).map_or_else(
                || ApiErrorCode::other(&redactor.sanitize(key)),
                ApiErrorCode::Data,
            )),
            error_message: text(message).map(|s| bounded(&s)),
        })
    }
}

/// Maps an `errorCode` string: `DH-901`..`DH-910` to their variants, a documented numeric code
/// to `Data`, anything else to `Other` holding the sanitised text, bounded to 32 bytes.
fn code_from_text(code: &str, redactor: &Redactor) -> ApiErrorCode {
    let dh = match code {
        "DH-901" => Some(ApiErrorCode::Dh901),
        "DH-902" => Some(ApiErrorCode::Dh902),
        "DH-903" => Some(ApiErrorCode::Dh903),
        "DH-904" => Some(ApiErrorCode::Dh904),
        "DH-905" => Some(ApiErrorCode::Dh905),
        "DH-906" => Some(ApiErrorCode::Dh906),
        "DH-907" => Some(ApiErrorCode::Dh907),
        "DH-908" => Some(ApiErrorCode::Dh908),
        "DH-909" => Some(ApiErrorCode::Dh909),
        "DH-910" => Some(ApiErrorCode::Dh910),
        _ => None,
    };
    dh.or_else(|| {
        Some(code)
            .filter(|c| !c.is_empty() && c.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|c| c.parse::<u16>().ok())
            .and_then(DataErrorCode::from_u16)
            .map(ApiErrorCode::Data)
    })
    .unwrap_or_else(|| ApiErrorCode::other(&redactor.sanitize(code)))
}

/// The error kind of a non-2xx response, first match wins (§5.6.3 of the design):
///
/// 1. status 429, `DH-904` or data code 805: `RateLimited`, remote source;
/// 2. status 401, `DH-901` or data codes 807–810: `Auth`;
/// 3. any other parsed broker error: `Api`;
/// 4. no parsable body: `HttpStatus`.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "used by the transport's response classification")
)]
pub(crate) fn classify_kind(
    status: u16,
    api: Option<&ApiError>,
) -> (ErrorKind, Option<RateLimitSource>) {
    use DataErrorCode::{
        AccessTokenExpired, AccessTokenInvalid, AuthenticationFailed, ClientIdInvalid,
    };
    let code = api.and_then(|a| a.error_code.as_ref());
    let rate_limited = matches!(
        code,
        Some(ApiErrorCode::Dh904 | ApiErrorCode::Data(DataErrorCode::TooManyRequests))
    );
    if status == 429 || rate_limited {
        return (ErrorKind::RateLimited, Some(RateLimitSource::Remote));
    }
    let auth = matches!(
        code,
        Some(
            ApiErrorCode::Dh901
                | ApiErrorCode::Data(
                    AccessTokenExpired
                        | AuthenticationFailed
                        | AccessTokenInvalid
                        | ClientIdInvalid
                )
        )
    );
    if status == 401 || auth {
        return (ErrorKind::Auth, None);
    }
    match api {
        Some(_) => (ErrorKind::Api, None),
        None => (ErrorKind::HttpStatus, None),
    }
}

/// The detail for a non-2xx response whose body [`ApiError::parse`] rejected; it describes the
/// body without repeating it.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "used by the transport's response classification")
)]
pub(crate) fn unparsed_body_detail(bytes: &[u8]) -> String {
    if serde_json::from_slice::<Value>(bytes).is_ok() {
        "unrecognised error body".to_owned()
    } else {
        format!("non-JSON error body of {} bytes", bytes.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(body: &str) -> Option<ApiError> {
        ApiError::parse(400, body.as_bytes(), &Redactor::new())
    }

    fn api(code: ApiErrorCode) -> ApiError {
        ApiError {
            error_type: None,
            error_code: Some(code),
            error_message: None,
        }
    }

    #[test]
    fn shape_one_documented_template_and_examples() {
        // DOC:465-470: the template with empty strings.
        let e = parse(
            "{\n    \"errorType\": \"\",\n    \"errorCode\": \"\",\n    \"errorMessage\": \"\"\n}",
        )
        .unwrap();
        assert_eq!(e.error_type.as_deref(), Some(""));
        assert_eq!(e.error_code, Some(ApiErrorCode::Other(String::new())));
        assert_eq!(e.error_message.as_ref().map(BoundedText::as_str), Some(""));
        // DOC:8821 (shape 1 example).
        let e = parse(r#"{"errorType": "Invalid_Authentication", "errorCode": "DH-901", "errorMessage": "Client ID or user generated access token is invalid or expired."}"#).unwrap();
        assert_eq!(e.error_type.as_deref(), Some("Invalid_Authentication"));
        assert_eq!(e.error_code, Some(ApiErrorCode::Dh901));
        assert_eq!(
            e.error_message.as_ref().map(BoundedText::as_str),
            Some("Client ID or user generated access token is invalid or expired.")
        );
    }

    #[test]
    fn shape_one_without_error_message() {
        // DOC:8872.
        let e = parse(r#"{"errorType":"Input_Exception","errorCode":"DH-905"}"#).unwrap();
        assert_eq!(e.error_code, Some(ApiErrorCode::Dh905));
        assert_eq!(e.error_message, None);
        assert_eq!(e.error_type.as_deref(), Some("Input_Exception"));
    }

    #[test]
    fn shape_two_with_status_failure() {
        // DOC:5252-5260.
        let e = parse(r#"{"status":"failure","errorType":"VALIDATION_ERROR","errorCode":"E001","errorMessage":"Invalid security ID"}"#).unwrap();
        assert_eq!(e.error_code, Some(ApiErrorCode::Other("E001".to_owned())));
        assert_eq!(e.error_type.as_deref(), Some("VALIDATION_ERROR"));
        assert_eq!(
            e.error_message.as_ref().map(BoundedText::as_str),
            Some("Invalid security ID")
        );
    }

    #[test]
    fn shape_three_data_code() {
        // DOC:8895-8898.
        let e = parse(r#"{"data": {"805": "Too many requests. Further requests may result in the user being blocked."}}"#).unwrap();
        assert_eq!(
            e.error_code,
            Some(ApiErrorCode::Data(DataErrorCode::TooManyRequests))
        );
        assert_eq!(e.error_type, None);
        assert_eq!(
            e.error_message.as_ref().map(BoundedText::as_str),
            Some("Too many requests. Further requests may result in the user being blocked.")
        );
        assert_eq!(
            parse(r#"{"data":{"805":"Too many"}}"#).unwrap().error_code,
            Some(ApiErrorCode::Data(DataErrorCode::TooManyRequests))
        );
        // An undocumented numeric key is kept as text.
        assert_eq!(
            parse(r#"{"data":{"803":"x"}}"#).unwrap().error_code,
            Some(ApiErrorCode::Other("803".to_owned()))
        );
    }

    #[test]
    fn other_bodies_are_not_parsed() {
        for body in [
            "<html><body>Bad Gateway</body></html>",
            "",
            "[]",
            "\"text\"",
            "{}",
            r#"{"status":"failure"}"#,
            r#"{"data":{}}"#,
            r#"{"data":{"805":"a","806":"b"}}"#,
            r#"{"data":{"code":"x"}}"#,
            r#"{"data":{"99999":"x"}}"#,
            r#"{"data":[1]}"#,
        ] {
            assert_eq!(parse(body), None, "{body:?}");
        }
    }

    #[test]
    fn error_codes_map_exactly() {
        let codes = [
            ("DH-901", ApiErrorCode::Dh901),
            ("DH-902", ApiErrorCode::Dh902),
            ("DH-903", ApiErrorCode::Dh903),
            ("DH-904", ApiErrorCode::Dh904),
            ("DH-905", ApiErrorCode::Dh905),
            ("DH-906", ApiErrorCode::Dh906),
            ("DH-907", ApiErrorCode::Dh907),
            ("DH-908", ApiErrorCode::Dh908),
            ("DH-909", ApiErrorCode::Dh909),
            ("DH-910", ApiErrorCode::Dh910),
            ("805", ApiErrorCode::Data(DataErrorCode::TooManyRequests)),
            ("814", ApiErrorCode::Data(DataErrorCode::InvalidRequest)),
            ("dh-901", ApiErrorCode::Other("dh-901".to_owned())),
            ("DH-911", ApiErrorCode::Other("DH-911".to_owned())),
            ("803", ApiErrorCode::Other("803".to_owned())),
        ];
        for (text, expected) in codes {
            assert_eq!(code_from_text(text, &Redactor::new()), expected, "{text}");
        }
        // A numeric errorCode is read as its text.
        assert_eq!(
            parse(r#"{"errorCode":805}"#).unwrap().error_code,
            Some(ApiErrorCode::Data(DataErrorCode::TooManyRequests))
        );
        // Bounded to 32 bytes (spaces keep the text clear of the long-run rule).
        let long = parse(&format!(r#"{{"errorCode":"{}"}}"#, "Z ".repeat(50))).unwrap();
        assert_eq!(long.error_code, Some(ApiErrorCode::Other("Z ".repeat(16))));
    }

    #[test]
    fn broker_text_is_sanitised_and_bounded() {
        let mut r = Redactor::new();
        r.register("SENTINELCLIENT9");
        let body = format!(
            r#"{{"errorType":"token=SECRETVALUE","errorCode":"DH-901","errorMessage":"client SENTINELCLIENT9 {}"}}"#,
            "é".repeat(400)
        );
        let e = ApiError::parse(401, body.as_bytes(), &r).unwrap();
        let message = e.error_message.unwrap();
        assert!(!message.as_str().contains("SENTINELCLIENT9"));
        assert!(message.as_str().len() <= 512 + "…[truncated]".len());
        assert_eq!(e.error_type.as_deref(), Some("token=<redacted>"));
    }

    #[test]
    fn registered_secrets_never_survive_in_any_field() {
        let mut r = Redactor::new();
        r.register("SENTINELCLIENT9");
        r.register("482913");
        let body = r#"{"errorType":"SENTINELCLIENT9","errorCode":"SENTINELCLIENT9 pin=1234","errorMessage":"totp 482913"}"#;
        let e = ApiError::parse(400, body.as_bytes(), &r).unwrap();
        let debug = format!("{e:?}");
        for secret in ["SENTINELCLIENT9", "482913", "1234"] {
            assert!(!debug.contains(secret), "{debug}");
        }
        assert_eq!(
            e.error_code,
            Some(ApiErrorCode::Other("<redacted> pin=<redacted>".to_owned()))
        );
        // A shape-3 key that echoes a registered value (for example a 4-digit PIN) is masked too.
        r.register("4829");
        let e = ApiError::parse(400, br#"{"data":{"4829":"x"}}"#, &r).unwrap();
        assert_eq!(
            e.error_code,
            Some(ApiErrorCode::Other("<redacted>".to_owned()))
        );
    }

    #[test]
    fn only_plain_digit_keys_and_integer_codes_count() {
        assert_eq!(parse(r#"{"data":{"+805":"x"}}"#), None);
        assert_eq!(
            parse(r#"{"errorCode":"+805"}"#).unwrap().error_code,
            Some(ApiErrorCode::Other("+805".to_owned()))
        );
        assert_eq!(parse(r#"{"data":{" 805":"x"}}"#), None);
        assert_eq!(parse(r#"{"errorCode":8.05e2}"#).unwrap().error_code, None);
        assert_eq!(parse(r#"{"errorCode":-805}"#).unwrap().error_code, None);
    }

    #[test]
    fn kind_mapping_table_row_by_row() {
        use ApiErrorCode::*;
        use DataErrorCode::*;
        let remote = Some(RateLimitSource::Remote);
        // Row 1: 429, DH-904 or data 805, whatever the other fields say.
        assert_eq!(classify_kind(429, None), (ErrorKind::RateLimited, remote));
        assert_eq!(
            classify_kind(400, Some(&api(Dh904))),
            (ErrorKind::RateLimited, remote)
        );
        assert_eq!(
            classify_kind(500, Some(&api(Data(TooManyRequests)))),
            (ErrorKind::RateLimited, remote)
        );
        assert_eq!(
            classify_kind(429, Some(&api(Dh901))),
            (ErrorKind::RateLimited, remote)
        );
        // Row 2: 401, DH-901 or data 807-810.
        assert_eq!(classify_kind(401, None), (ErrorKind::Auth, None));
        assert_eq!(
            classify_kind(400, Some(&api(Dh901))),
            (ErrorKind::Auth, None)
        );
        for code in [
            AccessTokenExpired,
            AuthenticationFailed,
            AccessTokenInvalid,
            ClientIdInvalid,
        ] {
            assert_eq!(
                classify_kind(400, Some(&api(Data(code)))),
                (ErrorKind::Auth, None),
                "{code:?}"
            );
        }
        assert_eq!(
            classify_kind(401, Some(&api(Dh906))),
            (ErrorKind::Auth, None)
        );
        // Row 3: any other parsed error.
        assert_eq!(
            classify_kind(500, Some(&api(Dh906))),
            (ErrorKind::Api, None)
        );
        assert_eq!(
            classify_kind(400, Some(&api(Data(DataApisNotSubscribed)))),
            (ErrorKind::Api, None)
        );
        assert_eq!(
            classify_kind(403, Some(&api(Other("E001".to_owned())))),
            (ErrorKind::Api, None)
        );
        let empty = ApiError {
            error_type: Some("x".to_owned()),
            error_code: None,
            error_message: None,
        };
        assert_eq!(classify_kind(400, Some(&empty)), (ErrorKind::Api, None));
        // Row 4: no parsable body.
        assert_eq!(classify_kind(502, None), (ErrorKind::HttpStatus, None));
        assert_eq!(classify_kind(400, None), (ErrorKind::HttpStatus, None));
    }

    #[test]
    fn unparsed_body_details_describe_without_repeating() {
        assert_eq!(
            unparsed_body_detail(b"<html>SECRET</html>"),
            "non-JSON error body of 19 bytes"
        );
        assert_eq!(
            unparsed_body_detail(br#"{"note":"SECRET"}"#),
            "unrecognised error body"
        );
        assert_eq!(unparsed_body_detail(b""), "non-JSON error body of 0 bytes");
    }
}
