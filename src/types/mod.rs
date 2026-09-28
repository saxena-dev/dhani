//! The shared vocabulary of requests and responses.
//!
//! - **Enums** such as [`ExchangeSegment`], [`OrderType`] and [`OrderStatus`], with their exact
//!   wire spellings.
//! - **[`Inbound<T>`]**, which every response enum field uses: a value dhani knows is
//!   [`Inbound::Known`], and one Dhan added later is kept as [`Inbound::Unknown`] rather than
//!   failing the whole response.
//! - **IDs** such as [`OrderId`], [`SecurityId`] and [`CorrelationId`], checked when you build
//!   them so that no string can change a request path.
//! - **[`WireTime`]**, a timestamp exactly as Dhan sent it, **[`RawJson`]** for responses kept
//!   undecoded, and **[`BoundedText`]** for broker text that has been sanitised and bounded.

mod common;
mod enums;
mod ids;
mod raw;
pub(crate) mod serde_ext;
mod text;
mod time;

pub use common::*;
pub use enums::{Inbound, UnknownValue, WireEnum};
pub(crate) use enums::{deserialize_inbound, deserialize_strict, wire_enum};
pub use ids::{AlertId, CorrelationId, Isin, OrderId, SecurityId};
pub use raw::RawJson;
pub use text::{BoundedText, DecimalString};
pub use time::{IST, WireTime, epoch_to_ist};

/// Converts a price to `rust_decimal::Decimal`; `None` for NaN, infinities and values outside
/// `Decimal`'s range (the upstream fixtures contain `-3.402823669209385e+38`).
///
/// ```
/// let d = dhani::types::to_decimal(1.5).unwrap();
/// assert_eq!(d.to_string(), "1.5");
/// ```
#[cfg(feature = "decimal")]
#[cfg_attr(docsrs, doc(cfg(feature = "decimal")))]
pub fn to_decimal(value: f64) -> Option<rust_decimal::Decimal> {
    rust_decimal::Decimal::try_from(value).ok()
}

#[cfg(all(test, feature = "decimal"))]
mod tests {
    use super::to_decimal;

    #[test]
    fn to_decimal_rejects_non_finite_and_out_of_range() {
        assert_eq!(
            to_decimal(1.5).map(|d| d.to_string()).as_deref(),
            Some("1.5")
        );
        assert_eq!(
            to_decimal(250.0).map(|d| d.to_string()).as_deref(),
            Some("250")
        );
        assert_eq!(to_decimal(f64::NAN), None);
        assert_eq!(to_decimal(f64::INFINITY), None);
        assert_eq!(to_decimal(-3.402823669209385e+38), None);
    }
}
