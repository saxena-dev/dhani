//! WebSocket feeds: builders, handles and public feed types, including `FeedLimits`,
//! `ReconnectPolicy` and `OverflowPolicy`.

pub(crate) mod actor;
mod builder;
mod depth;
mod global;
mod handle;
mod market;
mod order_update;
mod protocol;
mod tls;

#[allow(
    unused_imports,
    reason = "glob re-export scheme is fixed before the items exist; each glob imports nothing until its module gains public items"
)]
pub use self::{
    builder::*, depth::*, global::*, handle::*, market::*, order_update::*, protocol::*, tls::*,
};
