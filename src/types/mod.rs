//! Shared domain types: wire enums, ids, wire timestamps, raw JSON and bounded text.

mod common;
mod enums;
mod ids;
mod raw;
mod serde_ext;
mod text;
mod time;

pub use enums::{Inbound, UnknownValue, WireEnum};
#[allow(
    unused_imports,
    reason = "used by wire_enum! expansions; the shared enums in types::common are the first users"
)]
pub(crate) use enums::{deserialize_inbound, deserialize_strict, wire_enum};
pub use ids::{AlertId, CorrelationId, Isin, OrderId, SecurityId};
pub use raw::RawJson;
