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

use crate::error::{ValidationError, ValidationReason};

/// A request body as JSON; the request types always serialise, so the error is unreachable
/// in practice.
pub(crate) fn json_body(
    value: impl serde::Serialize,
) -> Result<serde_json::Value, ValidationError> {
    serde_json::to_value(value).map_err(|_| {
        ValidationError::new(
            "body",
            ValidationReason::Inconsistent("the request does not serialise"),
        )
    })
}

#[cfg(feature = "instruments")]
#[allow(
    unused_imports,
    reason = "some re-exported modules intentionally have no public items"
)]
pub use self::instruments::*;
#[allow(
    unused_imports,
    reason = "some re-exported modules intentionally have no public items"
)]
pub use self::{
    account::*, auth::*, conditional::*, edis::*, forever_orders::*, funds::*, global::*,
    historical::*, market_quote::*, option_chain::*, orders::*, portfolio::*, statements::*,
    super_orders::*, trader_control::*,
};
