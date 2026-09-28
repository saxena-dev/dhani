//! The REST client and every REST call.
//!
//! [`DhanClient`] owns a pooled HTTP transport, its settings, a shared [`RateLimiter`] and an
//! optional set of [`Credentials`](crate::Credentials), and hands out one facade per part of the
//! API: [`orders()`](DhanClient::orders), [`portfolio()`](DhanClient::portfolio),
//! [`funds()`](DhanClient::funds), [`market_quote()`](DhanClient::market_quote) and so on. Each
//! facade method takes a request type from this module and returns a typed response.
//!
//! # What a call does
//!
//! 1. The request is validated. An invalid one is a
//!    [`Validation`](crate::ErrorKind::Validation) error, and nothing is sent.
//! 2. The rate limiter admits it, waiting up to
//!    [`AdmissionLimits::max_wait`] (5 seconds by default) for capacity, or refuses it with
//!    [`RateLimited`](crate::ErrorKind::RateLimited).
//! 3. It is sent with the credential headers Dhan expects. JSON bodies get your client ID at
//!    the top level, as Dhan's Python SDK sends it.
//! 4. The response is classified. Success needs a 2xx status and a body of the endpoint's shape;
//!    a 2xx body that reports a failure is an error. Everything else is an
//!    [`Error`](crate::Error) that says what failed and how far the request got.
//!
//! # Retries
//!
//! Reads and read-only queries retry, with jittered backoff, a timeout or a transport failure
//! before any response arrives, a 502, 503 or 504 answer, and one remote rate limit (429,
//! `DH-904` or data error 805) after at least a second, within [`RetryPolicy`] and the
//! operation deadline in [`Timeouts`]. A failure while reading a response body is returned
//! without a retry. Every call that can change your
//! account, and every token call, makes exactly one attempt: if its response is lost,
//! [`Error::may_have_reached_server`](crate::Error::may_have_reached_server) says so, and you
//! decide whether to check and try again.
//!
//! # Rate limits
//!
//! [`RateLimiter`] applies Dhan's published limits per rate class, the per-key option-chain
//! window and the 25-modification cap per order, before anything is sent. Clones of a client
//! share one limiter; pass the same limiter to clients you build separately for one account.
//!
//! # Cancellation
//!
//! Call futures are lazy: dropping one before it is first polled sends nothing, and dropping
//! one while it waits for admission cancels local work only. Dropping one after the request was
//! sent does not cancel anything at Dhan; an order may still be placed.

mod api;
mod client;
mod endpoint;
mod models;
mod ratelimit;
mod response;
mod retry;
mod telemetry;
mod transport;

#[allow(
    unused_imports,
    reason = "some re-exported modules intentionally have no public items"
)]
pub use self::{api::*, models::*};
pub use client::{BodyLimits, DhanClient, DhanClientBuilder, RetryPolicy, Timeouts};
pub use ratelimit::{AdmissionLimits, QuotaProfile, RateLimiter, WallClock, Window, WindowPeriod};
