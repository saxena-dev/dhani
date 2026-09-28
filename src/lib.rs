//! An asynchronous Rust SDK for [Dhan](https://dhan.co/)'s
//! [DhanHQ](https://docs.dhanhq.co/) API (version 2): the REST API for orders, portfolio,
//! funds, statements and market data, and the WebSocket feeds for live market data and order
//! updates.
//!
//! > **Dhani** (Hindi: धनी) n.: A person of wealth; someone who is rich.
//!
//! dhani is built so that nothing goes wrong quietly. A failed call is never reported as
//! success, an order is never sent twice behind your back, a rate limit is enforced before
//! Dhan has to, a feed never drops data without telling you, and credentials stay out of
//! dhani's own errors, spans, events, metric labels and `Debug` output. When something does go
//! wrong, the error tells you what failed and how far the request got, so you can decide what
//! to do next.
//!
//! # Where to start
//!
//! | To… | Start with |
//! |---|---|
//! | get an access token from your PIN and TOTP (time-based one-time password) | [`Auth::generate_access_token`](rest::Auth::generate_access_token) |
//! | call the REST API | [`DhanClient`] |
//! | try calls in the sandbox | [`Environment::Sandbox`] |
//! | place, modify or cancel orders | [`Orders`](rest::Orders) and [`PlaceOrderRequest`](rest::PlaceOrderRequest) |
//! | read holdings, positions and funds | [`Portfolio`](rest::Portfolio) and [`Funds`](rest::Funds) |
//! | read quotes, candles and option chains | [`MarketQuote`](rest::MarketQuote), [`Historical`](rest::Historical) and [`OptionChain`](rest::OptionChain) |
//! | stream live market data | [`MarketFeed`](feed::MarketFeed) |
//! | follow your orders as they change | [`OrderUpdateFeed`](feed::OrderUpdateFeed) |
//! | find an instrument's security ID | `DhanClient::instruments`, with the `instruments` feature |
//! | decode captured feed bytes offline | [`decoder`] |
//! | handle failures | [`Error`] and [`ErrorKind`] |
//! | change deadlines, retries or rate limits | [`Timeouts`](rest::Timeouts), [`RetryPolicy`](rest::RetryPolicy) and [`RateLimiter`](rest::RateLimiter) |
//! | collect traces and metrics | [`obs`] |
//!
//! # A first program
//!
//! Credentials are your client ID and an access token, which you generate on Dhan's web
//! platform or with [`Auth::generate_access_token`](rest::Auth::generate_access_token). With
//! them, one client serves the whole REST API, and one feed streams market data.
//!
//! ```no_run
//! # #[cfg(all(feature = "rest", feature = "feed"))]
//! # mod example {
//! use dhani::feed::{FeedEvent, Instrument, MarketFeed, Mode};
//! use dhani::types::{ExchangeSegment, SecurityId};
//! use dhani::{AccessToken, ClientId, Credentials, DhanClient};
//! use futures_util::StreamExt;
//!
//! # pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let credentials = Credentials::new(
//!     ClientId::new("1000000009")?,
//!     AccessToken::new("<access token>")?,
//! );
//!
//! // REST: create one client and share clones of it.
//! let client = DhanClient::builder().credentials(credentials.clone()).build()?;
//! let funds = client.funds().limits().await?;
//! println!("available balance: {:?}", funds.available_balance);
//!
//! // Feeds: one background task owns the connection; you hold the handle.
//! let (handle, mut events, task) = MarketFeed::builder(credentials).spawn()?;
//! let hdfc_bank = Instrument::new(ExchangeSegment::NseEq, SecurityId::new("1333")?)?;
//! handle.subscribe([hdfc_bank], Mode::Ticker).await?;
//! while let Some(event) = events.next().await {
//!     match event? {
//!         FeedEvent::Data(d) => println!("{:?}", d.value),
//!         FeedEvent::Lifecycle(change) => println!("{change:?}"),
//!         _ => {} // FeedEvent is non-exhaustive.
//!     }
//! }
//! println!("feed ended: {:?}", task.join().await);
//! # Ok(()) }
//! # }
//! ```
//!
//! The [README](https://github.com/saxena-dev/dhani#readme) walks through getting a token,
//! placing an order safely, streaming data and handling errors, step by step.
//!
//! # Features
//!
//! | Feature | What it adds | Default |
//! |---|---|---|
//! | `rest` | [`DhanClient`] and every REST call in [`rest`] | yes |
//! | `feed` | the WebSocket feeds in [`feed`] (implies `decoder`) | yes |
//! | `decoder` | pure decoders for binary feed packets and order-update JSON, in [`decoder`] | no |
//! | `instruments` | download and parsing of the scrip master, Dhan's CSV list of every instrument (implies `rest`) | no |
//! | `metrics` | metric emission through the `metrics` facade | no |
//! | `decimal` | `types::to_decimal`, converting an `f64` price to `Option<rust_decimal::Decimal>` | no |
//! | `live-tests` | compiles the crate's own live test lane; adds no library code | no |
//!
//! Credentials, configuration, errors, the shared types in [`types`] and the telemetry schema
//! in [`obs`] are always available and need no async runtime. A build with only `decoder`, or
//! no features at all, has no Tokio or network dependency.
//!
//! # What you can rely on
//!
//! - **Success means success.** A call succeeds only when Dhan answers with a 2xx status and a
//!   body of the expected shape; a 2xx answer that reports a failure is an error.
//! - **Orders are sent once.** Every call that can change your account, and every token
//!   call, makes exactly one attempt. Reads retry failures before a response arrives, 502, 503
//!   and 504 answers, and one remote rate limit ([the exact rule](rest#retries)).
//! - **Rate limits are enforced locally**, at the rates Dhan publishes. A request waits briefly
//!   for capacity (5 seconds by default) and is refused with
//!   [`RateLimited`](ErrorKind::RateLimited) rather than sent to be rejected by Dhan.
//! - **Requests are validated before sending.** An invalid request is a
//!   [`Validation`](ErrorKind::Validation) error, and nothing leaves your machine.
//! - **Everything is bounded.** Queues, bodies, frames and waits have documented defaults and
//!   ranges, and exceeding one is an explicit error.
//! - **Unknown values are preserved** as [`Inbound::Unknown`](types::Inbound::Unknown), never
//!   coerced into a known one.
//!
//! # What dhani leaves to you
//!
//! It never stores, refreshes or revokes credentials on its own, and it does not compute TOTP
//! codes. It persists nothing, and it does not judge whether market data is fresh or whether
//! an order filled: it reports what Dhan said and when. It reads no environment variables, runs
//! no background task for REST calls, and installs no global `tracing` subscriber or metrics
//! recorder.
//!
//! # Reading these docs
//!
//! The item documentation cites its sources, so you can check any behaviour against Dhan's own
//! words:
//!
//! - **`DOC:<lines>`** cites lines of Dhan's documentation export,
//!   <https://docs.dhanhq.co/docs-export.md>. The exact version, with its SHA-256, and the page
//!   each range belongs to are recorded in
//!   [`docs/dhan-sources.toml`](https://github.com/saxena-dev/dhani/blob/main/docs/dhan-sources.toml).
//! - **`OAS:`** cites an operation or schema in Dhan's OpenAPI spec,
//!   <https://docs.dhanhq.co/openapi/dhan-api-v2.yaml>.
//! - **`LEGACY:<page>`** cites a page of Dhan's older documentation site,
//!   `https://dhanhq.co/docs/v2/<page>/`.
//! - **`PY:<path>:<lines>`** cites [DhanHQ-py](https://github.com/dhan-oss/DhanHQ-py), Dhan's
//!   Python SDK, at the commit dhani's tests pin.
//! - **SDK policy** marks a value dhani chose itself, such as a bound or a range, rather than
//!   one Dhan documents.
//!
//! # Status and disclaimer
//!
//! dhani is pre-1.0 and still changing. Breaking changes ship only in minor-version bumps, each
//! listed in the [changelog](https://github.com/saxena-dev/dhani/blob/main/CHANGELOG.md).
//! dhani is an independent open-source project: it is not affiliated with Dhan in any way, and
//! Dhan neither makes, endorses nor supports it.
//!
//! The software is provided "as is", without warranty of any kind. The author and contributors
//! take no responsibility for any financial losses, damages or other issues arising from its
//! use. Nothing in this crate is a statement that data is fresh, complete or fit for trading.
#![cfg_attr(docsrs, feature(doc_cfg))]
// The crate docs link items of every feature; a build with fewer features cannot resolve them.
#![cfg_attr(
    not(all(feature = "rest", feature = "feed")),
    allow(rustdoc::broken_intra_doc_links)
)]
#![warn(missing_docs)]

mod backoff;
pub mod config;
pub mod credentials;
pub mod error;
pub mod labels;
pub mod obs;
pub mod prelude;
pub mod types;

#[cfg(feature = "decoder")]
pub mod decoder;
#[cfg(feature = "feed")]
pub mod feed;
#[cfg(feature = "rest")]
pub mod rest;

pub use config::Environment;
pub use credentials::{AccessToken, ClientId, Credentials};
pub use error::{Error, ErrorKind, Result};
#[cfg(feature = "rest")]
pub use rest::{DhanClient, DhanClientBuilder};

// The README's examples are complete programs using both the REST client and the feeds.
#[cfg(all(doctest, feature = "rest", feature = "feed"))]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
