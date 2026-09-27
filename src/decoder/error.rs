//! `DecodeError` and `DecodeErrorKind`.

use std::fmt;

/// A frame or packet that could not be decoded. It carries only positions and codes, never
/// payload bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeError {
    /// What went wrong.
    pub kind: DecodeErrorKind,
    /// Byte offset in the frame where the failing packet starts.
    pub offset: usize,
    /// The packet's response or message code, when one was read.
    pub packet_code: Option<u8>,
}

/// Why a frame or packet could not be decoded.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DecodeErrorKind {
    /// The frame ends inside a header or packet.
    Truncated,
    /// A packet's length field is impossible (shorter than its header or past the frame end).
    BadLength,
    /// A packet code that is neither documented nor length-delimited.
    UnknownCode,
    /// Bytes remain after the last packet that cannot start another one.
    TrailingBytes,
    /// A packet's length field disagrees with its code's documented length.
    LengthMismatch,
    /// A 200-level depth packet's row count disagrees with its length.
    RowCountMismatch,
    /// A depth disconnect packet carries two different plausible reason codes.
    AmbiguousDisconnect,
    /// A binary frame arrived on a text-only feed.
    UnexpectedBinary,
    /// A text frame arrived on a binary-only feed.
    UnexpectedText,
    /// A text frame is not the expected JSON.
    Json,
}

impl DecodeErrorKind {
    /// The snake_case label, used in telemetry.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Truncated => "truncated",
            Self::BadLength => "bad_length",
            Self::UnknownCode => "unknown_code",
            Self::TrailingBytes => "trailing_bytes",
            Self::LengthMismatch => "length_mismatch",
            Self::RowCountMismatch => "row_count_mismatch",
            Self::AmbiguousDisconnect => "ambiguous_disconnect",
            Self::UnexpectedBinary => "unexpected_binary",
            Self::UnexpectedText => "unexpected_text",
            Self::Json => "json",
        }
    }
}

impl fmt::Display for DecodeErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at offset {}", self.kind, self.offset)?;
        if let Some(code) = self.packet_code {
            write!(f, " (packet code {code})")?;
        }
        Ok(())
    }
}

impl std::error::Error for DecodeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_snake_case_and_distinct() {
        let kinds = [
            DecodeErrorKind::Truncated,
            DecodeErrorKind::BadLength,
            DecodeErrorKind::UnknownCode,
            DecodeErrorKind::TrailingBytes,
            DecodeErrorKind::LengthMismatch,
            DecodeErrorKind::RowCountMismatch,
            DecodeErrorKind::AmbiguousDisconnect,
            DecodeErrorKind::UnexpectedBinary,
            DecodeErrorKind::UnexpectedText,
            DecodeErrorKind::Json,
        ];
        let labels: Vec<_> = kinds.iter().map(|k| k.as_str()).collect();
        assert_eq!(
            labels,
            [
                "truncated",
                "bad_length",
                "unknown_code",
                "trailing_bytes",
                "length_mismatch",
                "row_count_mismatch",
                "ambiguous_disconnect",
                "unexpected_binary",
                "unexpected_text",
                "json"
            ]
        );
    }

    #[test]
    fn display_names_the_kind_offset_and_code() {
        let with_code = DecodeError {
            kind: DecodeErrorKind::UnknownCode,
            offset: 16,
            packet_code: Some(9),
        };
        assert_eq!(
            with_code.to_string(),
            "unknown_code at offset 16 (packet code 9)"
        );
        let without = DecodeError {
            kind: DecodeErrorKind::Truncated,
            offset: 3,
            packet_code: None,
        };
        assert_eq!(without.to_string(), "truncated at offset 3");
    }
}
