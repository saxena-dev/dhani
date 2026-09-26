//! REST facades, one private module per API group, each glob re-exported.

mod account;
mod auth;
mod conditional;
mod edis;
mod forever_orders;
mod funds;
mod global;
mod historical;
#[cfg(feature = "instruments")]
mod instruments;
mod market_quote;
mod option_chain;
mod orders;
mod portfolio;
mod statements;
mod super_orders;
mod trader_control;

#[cfg(feature = "instruments")]
#[allow(
    unused_imports,
    reason = "glob re-export scheme is fixed before the items exist; each glob imports nothing until its module gains public items"
)]
pub use self::instruments::*;
#[allow(
    unused_imports,
    reason = "glob re-export scheme is fixed before the items exist; each glob imports nothing until its module gains public items"
)]
pub use self::{
    account::*, auth::*, conditional::*, edis::*, forever_orders::*, funds::*, global::*,
    historical::*, market_quote::*, option_chain::*, orders::*, portfolio::*, statements::*,
    super_orders::*, trader_control::*,
};
