//! Curated re-exports for glob import: `DhanClient`, `Credentials`, `ClientId`, `AccessToken`,
//! `Environment`, `Error`, `ErrorKind`, `Result`, every enum in `types::common`, the ids in
//! `types::ids` and `Inbound`; with the `feed` feature also `MarketFeed`, `Instrument`, `Mode`
//! and `OrderUpdateFeed`.
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
//!
//! With the `feed` feature:
//!
//! ```
//! # #[cfg(feature = "feed")] {
//! use dhani::prelude::*;
//!
//! let credentials = Credentials::new(
//!     ClientId::new("1000000009").unwrap(),
//!     AccessToken::new("token").unwrap(),
//! );
//! let instrument = Instrument::new(ExchangeSegment::NseEq, SecurityId::new("1333").unwrap()).unwrap();
//! let _market = MarketFeed::builder(credentials.clone());
//! let _orders = OrderUpdateFeed::builder(credentials);
//! assert_eq!((instrument.security_id().as_ref(), Mode::Full), ("1333", Mode::Full));
//! # }
//! ```

pub use crate::config::Environment;
pub use crate::credentials::{AccessToken, ClientId, Credentials};
pub use crate::error::{Error, ErrorKind, Result};
#[cfg(feature = "feed")]
pub use crate::feed::{Instrument, MarketFeed, Mode, OrderUpdateFeed};
#[cfg(feature = "rest")]
pub use crate::rest::DhanClient;
pub use crate::types::{
    AlertId, AmoTime, CorrelationId, ExchangeSegment, ExpiryCode, ExpiryFlag, Inbound,
    InstrumentKind, Isin, LegName, OptionType, OrderId, OrderStatus, OrderType, PositionType,
    ProductType, SecurityId, TransactionType, Validity,
};
