//! Pure decoders for the market feed's binary packets and the order-update feed's JSON.
//!
//! The feeds use these decoders as frames arrive, and you can use them directly on captured
//! bytes: they need no runtime, network, clock or telemetry, so a build with only the `decoder`
//! feature has no Tokio or network dependency.
//!
//! ```
//! use dhani::decoder::split_market;
//!
//! fn decode(frame: &[u8]) {
//!     for packet in split_market(frame) {
//!         match packet {
//!             Ok(packet) => println!("{packet:?}"),
//!             // Decoding stops at the first error; the rest of the frame is unreadable.
//!             Err(e) => eprintln!("{e:?}"),
//!         }
//!     }
//! }
//! # decode(&[]);
//! ```
//!
//! A frame may hold several packets, and [`split_market`] walks all of them. Prices are `f32`
//! exactly as sent, and time fields are exposed raw. [`parse_order_update`] turns one
//! order-update message into an [`OrderUpdateEvent`]; a message it does not recognise is kept
//! as [`OrderUpdateEvent::Other`] with its raw JSON. Every offset and length is a named constant
//! in the public [`layout`] module.

mod depth;
mod error;
mod global;
pub mod layout;
mod market;
mod order_update;

#[allow(
    unused_imports,
    reason = "some re-exported modules intentionally have no public items"
)]
pub use self::{depth::*, error::*, global::*, market::*, order_update::*};
