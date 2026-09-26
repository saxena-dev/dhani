//! Asynchronous client library for the DhanHQ v2 trading API.
//!
//! `dhani` covers the DhanHQ v2 REST trading and data APIs, the auth and account APIs, the
//! Global Stocks REST APIs, the binary and JSON streaming feeds, and the sandbox environment.
//! Every request and feed is instrumented with `tracing` spans and events whose fields are
//! redacted, and optional metrics are emitted through the `metrics` facade.
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
//! | `decimal` | `to_decimal()` helpers on price fields |
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

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
