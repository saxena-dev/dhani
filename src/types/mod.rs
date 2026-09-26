//! Shared domain types: wire enums, ids, wire timestamps, raw JSON and bounded text.

mod common;
mod enums;
mod ids;
mod raw;
mod serde_ext;
mod text;
mod time;

pub use common::*;
pub use enums::{Inbound, UnknownValue, WireEnum};
pub(crate) use enums::{deserialize_inbound, deserialize_strict, wire_enum};
pub use ids::{AlertId, CorrelationId, Isin, OrderId, SecurityId};
pub use raw::RawJson;
pub use text::{BoundedText, DecimalString};
pub use time::{IST, WireTime, epoch_to_ist};
