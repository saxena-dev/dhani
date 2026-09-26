//! The crate error model: `Error`, `ErrorKind`, `Stage`, `ApiError`, `ApiErrorCode`,
//! `DataErrorCode`, `ValidationError`, `ValidationReason`, `ConfigError`, `RateLimitInfo`,
//! `RateLimitSource` and the `Result` alias.

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
