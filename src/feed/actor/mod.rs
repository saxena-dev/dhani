//! The single-owner feed actor: owner loop, connection lifecycle, delivery, subscriptions and
//! status.

mod delivery;
mod lifecycle;
mod owner;
mod status;
mod subscriptions;

#[allow(
    unused_imports,
    reason = "glob re-export scheme is fixed before the items exist; each glob imports nothing until its module gains public items"
)]
pub use self::{delivery::*, lifecycle::*, owner::*, status::*, subscriptions::*};
