//! `BoundedText`: length-capped stored text, and `DecimalString`: validated decimal text for
//! request fields that DhanHQ expects as strings.

use std::fmt;

use serde::{Serialize, Serializer};

use crate::error::{ValidationError, ValidationReason};

/// Owned text capped at 512 bytes.
///
/// Longer input is cut on a character boundary at or below 512 bytes and ends with a
/// `…[truncated]` marker, so a truncated value is at most 512 bytes plus the 14-byte marker.
/// Stored broker messages and decode details are sanitised before they are wrapped (see
/// [`Redactor`](crate::obs::Redactor)); this type only bounds their size.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BoundedText(String);

impl BoundedText {
    /// Largest number of bytes kept before the truncation marker.
    const MAX_BYTES: usize = 512;
    const MARKER: &'static str = "…[truncated]";

    /// Wraps `text`, truncating it if it is longer than 512 bytes.
    pub fn new(text: impl Into<String>) -> Self {
        let mut text = text.into();
        if text.len() > Self::MAX_BYTES {
            let mut end = Self::MAX_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
            text.push_str(Self::MARKER);
        }
        Self(text)
    }

    /// The stored text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BoundedText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A non-negative decimal number as text, matching `^[0-9]+(\.[0-9]+)?$`, sent verbatim.
///
/// Some request fields carry money as a string (conditional and multi-order legs). There is
/// deliberately no `From<f64>`: formatting a float can silently change the caller's value (for
/// example `250.005`), and the documentation fixes no number of decimal places.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DecimalString(String);

impl DecimalString {
    const FIELD: &'static str = "decimal";

    /// Validates `text` against `^[0-9]+(\.[0-9]+)?$`.
    pub fn new(text: &str) -> Result<Self, ValidationError> {
        check_decimal(text)?;
        Ok(Self(text.to_owned()))
    }

    /// Re-checks the text; request validators call this, since a value converted from a
    /// negative `rust_decimal::Decimal` does not match the pattern.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "called by the conditional and multi-order request validators"
        )
    )]
    pub(crate) fn validate(&self) -> Result<(), ValidationError> {
        check_decimal(&self.0)
    }

    /// The decimal text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn check_decimal(text: &str) -> Result<(), ValidationError> {
    if text.is_empty() {
        return Err(ValidationError::new(
            DecimalString::FIELD,
            ValidationReason::Empty,
        ));
    }
    let (int, frac) = match text.split_once('.') {
        Some((int, frac)) => (int, Some(frac)),
        None => (text, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if digits(int) && frac.is_none_or(digits) {
        Ok(())
    } else {
        Err(ValidationError::new(
            DecimalString::FIELD,
            ValidationReason::InvalidCharacters,
        ))
    }
}

impl From<u32> for DecimalString {
    fn from(value: u32) -> Self {
        Self(value.to_string())
    }
}

#[cfg(feature = "decimal")]
impl From<rust_decimal::Decimal> for DecimalString {
    /// The decimal's plain text. A negative value produces text that does not match the
    /// pattern; request validation rejects it before anything is sent.
    fn from(value: rust_decimal::Decimal) -> Self {
        Self(value.to_string())
    }
}

impl fmt::Display for DecimalString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for DecimalString {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_text_truncates_on_a_char_boundary() {
        // 600 bytes of 3-byte characters: 512 is not a boundary (510 is).
        let input = "\u{20ac}".repeat(200);
        assert_eq!(input.len(), 600);
        let t = BoundedText::new(input);
        assert!(t.as_str().ends_with("…[truncated]"));
        let kept = t.as_str().strip_suffix("…[truncated]").unwrap();
        assert_eq!(kept, "\u{20ac}".repeat(170));
        assert!(kept.len() <= 512);
        assert_eq!(t.to_string(), t.as_str());

        let ascii = "a".repeat(600);
        let t = BoundedText::new(ascii);
        assert_eq!(t.as_str(), format!("{}…[truncated]", "a".repeat(512)));

        let exact = "b".repeat(512);
        assert_eq!(BoundedText::new(exact.clone()).as_str(), exact);
        assert_eq!(
            BoundedText::new("short"),
            BoundedText::new(String::from("short"))
        );
    }

    #[test]
    fn decimal_string_validation() {
        assert_eq!(DecimalString::new("250.00").unwrap().as_str(), "250.00");
        assert!(DecimalString::new("0").is_ok());
        assert!(DecimalString::new("007.5").is_ok());
        for bad in [
            "250.", "-1", ".5", "1.2.3", "1e5", " 1", "1,000", "+1", "NaN",
        ] {
            let err = DecimalString::new(bad).unwrap_err();
            assert_eq!(err.reason, ValidationReason::InvalidCharacters, "{bad:?}");
            assert_eq!(err.field, "decimal");
        }
        assert_eq!(
            DecimalString::new("").unwrap_err().reason,
            ValidationReason::Empty
        );
    }

    #[test]
    fn decimal_string_from_u32_and_serialisation() {
        let d = DecimalString::from(7u32);
        assert_eq!(d.as_str(), "7");
        assert_eq!(d.validate(), Ok(()));
        assert_eq!(
            serde_json::to_string(&DecimalString::new("250.00").unwrap()).unwrap(),
            r#""250.00""#
        );
        assert_eq!(DecimalString::from(u32::MAX).to_string(), "4294967295");
    }

    #[cfg(feature = "decimal")]
    #[test]
    fn decimal_string_from_rust_decimal() {
        let d = DecimalString::from(rust_decimal::Decimal::new(25000, 2));
        assert_eq!(d.as_str(), "250.00");
        assert_eq!(d.validate(), Ok(()));
        let negative = DecimalString::from(rust_decimal::Decimal::new(-15, 1));
        assert_eq!(negative.as_str(), "-1.5");
        assert_eq!(
            negative.validate().unwrap_err().reason,
            ValidationReason::InvalidCharacters
        );
    }
}
