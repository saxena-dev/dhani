//! Identifier newtypes: `OrderId`, `SecurityId`, `CorrelationId`, `AlertId` and `Isin`.
//!
//! `new()` validates a value before it can be sent. Deserialisation is lenient: any JSON string
//! up to 128 bytes is accepted without a charset or length check, because responses (and the
//! upstream fixtures) carry placeholders such as `"string"`. An ID taken from a response is
//! revalidated with `validate()` when it is reused in a request.
//!
//! Lengths and charsets are SDK policy (safe path segments), except the `CorrelationId` rule
//! and the ISIN shape, which come from the documentation.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{ValidationError, ValidationReason};

/// Longest string any ID accepts when deserialised from a response.
const MAX_DESERIALIZED_BYTES: usize = 128;

/// Checks `1..=max` characters, each accepted by `allowed`.
fn check(
    field: &'static str,
    s: &str,
    max: usize,
    allowed: fn(char) -> bool,
) -> Result<(), ValidationError> {
    if s.is_empty() {
        return Err(ValidationError::new(field, ValidationReason::Empty));
    }
    if s.chars().count() > max {
        return Err(ValidationError::new(
            field,
            ValidationReason::TooLong { max },
        ));
    }
    if !s.chars().all(allowed) {
        return Err(ValidationError::new(
            field,
            ValidationReason::InvalidCharacters,
        ));
    }
    Ok(())
}

fn alnum_dash(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-'
}

fn security_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

fn correlation_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-')
}

/// Declares an ID newtype with its validating constructor, lenient `Deserialize` and the
/// standard conversions. `$validate` checks a candidate string.
macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident, $field:literal, $validate:expr) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// Validates `value` and wraps it.
            pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
                let value = value.into();
                ($validate)(value.as_str())?;
                Ok(Self(value))
            }

            /// Re-checks a value obtained from a response before it is reused in a request.
            pub fn validate(&self) -> Result<(), ValidationError> {
                ($validate)(self.0.as_str())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = ValidationError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::new(s)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = ValidationError;

            fn try_from(s: &str) -> Result<Self, Self::Error> {
                Self::new(s)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let value = String::deserialize(d)?;
                if value.len() > MAX_DESERIALIZED_BYTES {
                    return Err(serde::de::Error::custom(concat!(
                        $field,
                        " longer than 128 bytes"
                    )));
                }
                Ok(Self(value))
            }
        }
    };
}

id_type!(
    /// An order ID: 1..=64 characters from `[A-Za-z0-9-]`.
    OrderId,
    "order_id",
    |s| check("order_id", s, 64, alnum_dash)
);

id_type!(
    /// A security ID: 1..=32 characters from `[A-Za-z0-9._-]`.
    ///
    /// Indian instruments use numeric IDs; Global Stocks uses tickers such as `"AAPL"`
    /// (DOC:2587).
    SecurityId,
    "security_id",
    |s| check("security_id", s, 32, security_char)
);

id_type!(
    /// A caller-chosen correlation ID: 1..=30 characters from `[A-Za-z0-9_-]` (DOC:3729).
    ///
    /// The documentation's list ends in a dot that may be punctuation; the dot is rejected
    /// locally, since a wrong exclusion only fails before sending. Responses may carry
    /// characters this rule forbids (DOC:6321), so response fields use a plain string.
    CorrelationId,
    "correlation_id",
    |s| check("correlation_id", s, 30, correlation_char)
);

id_type!(
    /// A conditional-trigger alert ID: 1..=64 characters from `[A-Za-z0-9-]`.
    AlertId,
    "alert_id",
    |s| check("alert_id", s, 64, alnum_dash)
);

id_type!(
    /// An ISIN: 12 ASCII alphanumerics, or exactly `ALL` ([`Isin::all`]) where an endpoint
    /// accepts it (DOC:495-497, DOC:5182-5244).
    Isin,
    "isin",
    check_isin
);

fn check_isin(s: &str) -> Result<(), ValidationError> {
    const LEN: usize = 12;
    if s == Isin::ALL {
        return Ok(());
    }
    let reason = if s.is_empty() {
        ValidationReason::Empty
    } else if s.chars().count() > LEN {
        ValidationReason::TooLong { max: LEN }
    } else if !s.chars().all(|c| c.is_ascii_alphanumeric()) {
        ValidationReason::InvalidCharacters
    } else if s.len() < LEN {
        ValidationReason::Inconsistent("an ISIN has exactly 12 characters")
    } else {
        return Ok(());
    };
    Err(ValidationError::new("isin", reason))
}

impl Isin {
    const ALL: &'static str = "ALL";

    /// The `ALL` wildcard, meaning every holding (DOC:497).
    pub fn all() -> Self {
        Self(Self::ALL.to_owned())
    }
}

impl SecurityId {
    /// The numeric value of an all-digit ID that fits in `u32`, as quote request bodies and
    /// feed subscriptions need; `None` for tickers and anything else. Leading zeros are accepted
    /// and dropped (`"01333"` gives `Some(1333)`).
    pub fn as_numeric(&self) -> Option<u32> {
        if self.0.is_empty() || !self.0.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        self.0.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reason<T>(r: Result<T, ValidationError>) -> ValidationReason {
        match r {
            Err(e) => e.reason,
            Ok(_) => panic!("expected a validation error"),
        }
    }

    #[test]
    fn correlation_id_rules() {
        let e = CorrelationId::new("a b").unwrap_err();
        assert_eq!(
            e,
            ValidationError::new("correlation_id", ValidationReason::InvalidCharacters)
        );
        assert_eq!(
            reason(CorrelationId::new("x".repeat(31))),
            ValidationReason::TooLong { max: 30 }
        );
        assert_eq!(
            reason(CorrelationId::new("a.b")),
            ValidationReason::InvalidCharacters
        );
        assert_eq!(reason(CorrelationId::new("")), ValidationReason::Empty);
        // Length is checked before the charset.
        assert_eq!(
            reason(CorrelationId::new(format!("{} ", "x".repeat(30)))),
            ValidationReason::TooLong { max: 30 }
        );
        assert!(CorrelationId::new("x".repeat(30)).is_ok());
        assert!(CorrelationId::new("Ab_9-z").is_ok());
    }

    #[test]
    fn security_id_numeric_and_ticker() {
        let ticker = SecurityId::new("AAPL").unwrap();
        assert_eq!(ticker.as_numeric(), None);
        assert_eq!(SecurityId::new("1333").unwrap().as_numeric(), Some(1333));
        assert_eq!(SecurityId::new("BRK.B").unwrap().as_numeric(), None);
        assert_eq!(SecurityId::new("4294967296").unwrap().as_numeric(), None);
        assert_eq!(SecurityId::new("01333").unwrap().as_numeric(), Some(1333));
        assert_eq!(
            SecurityId::new("4294967295").unwrap().as_numeric(),
            Some(u32::MAX)
        );
        let lenient: SecurityId = serde_json::from_str(r#""+5""#).unwrap();
        assert_eq!(lenient.as_numeric(), None);
        assert_eq!(
            reason(SecurityId::new("x".repeat(33))),
            ValidationReason::TooLong { max: 32 }
        );
        assert_eq!(
            reason(SecurityId::new("a/b")),
            ValidationReason::InvalidCharacters
        );
    }

    #[test]
    fn isin_rules() {
        assert!(Isin::new("INE002A01018").is_ok());
        assert_eq!(Isin::all().to_string(), "ALL");
        assert_eq!(Isin::new("ALL").unwrap(), Isin::all());
        assert_eq!(
            reason(Isin::new("INE002A0101")),
            ValidationReason::Inconsistent("an ISIN has exactly 12 characters")
        );
        assert_eq!(
            reason(Isin::new("INE002A010189")),
            ValidationReason::TooLong { max: 12 }
        );
        assert_eq!(
            reason(Isin::new("INE002A0101-")),
            ValidationReason::InvalidCharacters
        );
        assert_eq!(
            reason(Isin::new("all")),
            ValidationReason::Inconsistent("an ISIN has exactly 12 characters")
        );
        assert_eq!(Isin::new("").unwrap_err().field, "isin");
    }

    #[test]
    fn order_and_alert_id_rules() {
        assert!(OrderId::new("112111182198").is_ok());
        assert_eq!(
            reason(OrderId::new("x".repeat(65))),
            ValidationReason::TooLong { max: 64 }
        );
        assert_eq!(
            reason(OrderId::new("12_3")),
            ValidationReason::InvalidCharacters
        );
        assert_eq!(reason(AlertId::new("")), ValidationReason::Empty);
        assert_eq!(AlertId::new("a b").unwrap_err().field, "alert_id");
    }

    #[test]
    fn deserialisation_is_lenient_but_bounded() {
        // The upstream fixtures carry the Swagger placeholder "string"; it happens to satisfy
        // the OrderId charset, so revalidation passes.
        let placeholder: OrderId = serde_json::from_str(r#""string""#).unwrap();
        assert_eq!(placeholder.as_ref(), "string");
        assert_eq!(placeholder.validate(), Ok(()));

        // Anything up to 128 bytes decodes without checks, and fails revalidation if invalid.
        let isin: Isin = serde_json::from_str(r#""string""#).unwrap();
        assert!(isin.validate().is_err());
        let spaced: CorrelationId = serde_json::from_str(r#""a b""#).unwrap();
        assert_eq!(
            spaced.validate().unwrap_err().reason,
            ValidationReason::InvalidCharacters
        );
        let max = "x".repeat(128);
        assert!(serde_json::from_str::<OrderId>(&format!("\"{max}\"")).is_ok());

        // The bound is in bytes: 43 three-byte characters are 129 bytes.
        assert!(
            serde_json::from_str::<OrderId>(&format!("\"{}\"", "\u{20ac}".repeat(43))).is_err()
        );
        let long = "x".repeat(129);
        let err = serde_json::from_str::<OrderId>(&format!("\"{long}\"")).unwrap_err();
        assert!(
            err.to_string()
                .starts_with("order_id longer than 128 bytes"),
            "{err}"
        );
        assert!(serde_json::from_str::<OrderId>("123").is_err());
    }

    #[test]
    fn every_id_round_trips_through_serde_json_and_from_str() {
        fn round_trip<T>(value: T, text: &str)
        where
            T: Serialize
                + for<'de> Deserialize<'de>
                + FromStr<Err = ValidationError>
                + fmt::Display
                + AsRef<str>
                + PartialEq
                + fmt::Debug,
        {
            let json = serde_json::to_string(&value).unwrap();
            assert_eq!(json, format!("\"{text}\""));
            assert_eq!(serde_json::from_str::<T>(&json).unwrap(), value);
            assert_eq!(text.parse::<T>().unwrap(), value);
            assert_eq!(value.to_string(), text);
            assert_eq!(value.as_ref(), text);
        }
        round_trip(OrderId::new("112111182198").unwrap(), "112111182198");
        round_trip(SecurityId::new("1333").unwrap(), "1333");
        round_trip(SecurityId::try_from("AAPL").unwrap(), "AAPL");
        round_trip(CorrelationId::new("my-order_1").unwrap(), "my-order_1");
        round_trip(AlertId::new("12345").unwrap(), "12345");
        round_trip(Isin::new("INE002A01018").unwrap(), "INE002A01018");
        round_trip(Isin::all(), "ALL");
    }
}
