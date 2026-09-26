//! The REST client: `DhanClient`, `DhanClientBuilder`, `Timeouts`, `RetryPolicy`, `BodyLimits`,
//! `RateLimiter`, `QuotaProfile` and every REST facade with its request and response types.

mod api;
mod client;
mod endpoint;
mod models;
mod ratelimit;
mod retry;
mod transport;

#[allow(
    unused_imports,
    reason = "glob re-export scheme is fixed before the items exist; each glob imports nothing until its module gains public items"
)]
pub use self::{api::*, models::*};
pub use client::{BodyLimits, DhanClient, DhanClientBuilder, RetryPolicy, Timeouts};
pub use ratelimit::{AdmissionLimits, QuotaProfile, RateLimiter, WallClock, Window, WindowPeriod};
