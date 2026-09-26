//! Curated re-exports for glob import: `DhanClient`, `Credentials`, `ClientId`, `AccessToken`,
//! `Environment`, `Error`, `ErrorKind`, `Result`, every enum in `types::common`, the ids in
//! `types::ids` and `Inbound`; with the `feed` feature also `MarketFeed`, `Instrument`, `Mode`
//! and `OrderUpdateFeed`.
//!
//! The client and feed entries join as those parts of the crate land.
//!
//! ```
//! use dhani::prelude::*;
//!
//! let segment = ExchangeSegment::NseEq;
//! let status = Inbound::<OrderStatus>::Known(OrderStatus::Traded);
//! let order = OrderId::new("112111182198").unwrap();
//! let credentials = Credentials::new(
//!     ClientId::new("1000000009").unwrap(),
//!     AccessToken::new("token").unwrap(),
//! );
//! assert_eq!(segment.to_string(), "NSE_EQ");
//! assert_eq!(status.known(), Some(OrderStatus::Traded));
//! assert_eq!(order.as_ref(), "112111182198");
//! assert_eq!(credentials.client_id().expose_secret(), "1000000009");
//! assert_eq!(Environment::default(), Environment::Live);
//! ```

pub use crate::config::Environment;
pub use crate::credentials::{AccessToken, ClientId, Credentials};
pub use crate::error::{Error, ErrorKind, Result};
pub use crate::types::{
    AlertId, AmoTime, CorrelationId, ExchangeSegment, ExpiryCode, ExpiryFlag, Inbound,
    InstrumentKind, Isin, LegName, OptionType, OrderId, OrderStatus, OrderType, PositionType,
    ProductType, SecurityId, TransactionType, Validity,
};
