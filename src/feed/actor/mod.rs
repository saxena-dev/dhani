//! The single-owner feed actor: owner loop, connection lifecycle, delivery, subscriptions and
//! status.

mod delivery;
mod lifecycle;
mod owner;
mod status;
mod subscriptions;

#[allow(
    unused_imports,
    reason = "some re-exported modules intentionally have no public items"
)]
pub use self::{delivery::*, lifecycle::*, owner::*, status::*, subscriptions::*};
