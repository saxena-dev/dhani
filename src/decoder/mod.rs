//! Pure decoders for the binary feeds and the JSON order-update feed.
//!
//! The decoders use no runtime, network, clock or telemetry: they depend only on `std`, `serde`,
//! `serde_json` and the crate `types` module.

mod depth;
mod error;
mod global;
mod layout;
mod market;
mod order_update;

#[allow(
    unused_imports,
    reason = "glob re-export scheme is fixed before the items exist; each glob imports nothing until its module gains public items"
)]
pub use self::{depth::*, error::*, global::*, layout::*, market::*, order_update::*};
