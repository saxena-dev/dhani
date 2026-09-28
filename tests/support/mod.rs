//! Shared integration-test support. Each test binary includes this module with `mod support;`
//! and uses only the helpers it needs.
#![allow(dead_code)]

pub mod fixtures;
pub mod trace;

#[cfg(feature = "decoder")]
pub mod encode;
#[cfg(feature = "rest")]
pub mod fault_http;
#[cfg(feature = "rest")]
pub mod mock;
#[cfg(feature = "feed")]
pub mod ws;
