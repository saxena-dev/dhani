//! Asynchronous client library for the DhanHQ v2 trading API.
//!
//! This release covers the REST orders, portfolio, funds, statements, market quote, historical
//! data, option chain and instrument master APIs, access tokens from PIN and TOTP, the account
//! profile, the Live Market Feed and the Live Order Update feed, and the REST sandbox. Super,
//! forever and conditional orders, trader's control, EDIS and Global Stocks follow in later 0.x
//! releases, whose REST facades exist but have no calls yet, as do the depth and Global Stocks
//! feeds.
//!
//! Every request and feed is instrumented with `tracing` spans and events whose fields are
//! redacted, and optional metrics are emitted through the `metrics` facade.
//!
//! **0.1.0 has not been run against a Dhan account.** The README lists the defaults it ships
//! with where Dhan's documentation is silent or inconsistent.
//!
//! The library reads no environment variables, installs no global subscriber or recorder and
//! runs no background task for REST calls: all configuration is passed in explicitly.
//!
//! # Cargo features
//!
//! | Feature | Enables |
//! |---|---|
//! | `rest` *(default)* | the REST client and every REST facade |
//! | `feed` *(default)* | the WebSocket feeds (implies `decoder`) |
//! | `decoder` | the pure binary and JSON feed decoders |
//! | `instruments` | CSV instrument master download and parse (implies `rest`) |
//! | `metrics` | metric emission through the `metrics` facade |
//! | `decimal` | [`types::to_decimal`], converting an `f64` price to `Option<rust_decimal::Decimal>` |
//! | `live-tests` | compiles the live and sandbox test lane; adds no library code |
#![cfg_attr(docsrs, feature(doc_cfg))]
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

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
