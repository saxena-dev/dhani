//! The live and sandbox lanes: tests that talk to Dhan.
//!
//! Every test here is `#[ignore]` and needs credentials, so nothing runs by default and CI only
//! compiles this file (`cargo test --features live-tests --test live --no-run`). To run a lane:
//!
//! ```text
//! cargo test --features live-tests --test live -- --ignored sandbox_
//! cargo test --features live-tests --test live -- --ignored live_ro_
//! cargo test --features live-tests --test live -- --ignored live_feed_
//! ```
//!
//! Every test holds one process-wide lock while it talks to Dhan, so tests never overlap: each
//! has its own client and rate limiter, and running them together could exceed Dhan's limits.
//!
//! Credentials come from environment variables, read in this file only:
//! `DHANI_SANDBOX_CLIENT_ID` and `DHANI_SANDBOX_ACCESS_TOKEN` for the sandbox (a token from
//! developer.dhanhq.co), `DHANI_LIVE_CLIENT_ID` and `DHANI_LIVE_ACCESS_TOKEN` for the live
//! account. A missing variable makes a test print `skipped: <var> not set` and pass. The one
//! test that places a real order also needs `DHANI_LIVE_ALLOW_ORDERS=1`.
//!
//! The live tests write what they receive under `target/captures/`: REST quote bodies as JSON in
//! `rest/`, and feed frames in `ws/<feed>-<date>.bin`. A capture file is a sequence of records,
//! each a little-endian `u64` receipt time in Unix nanoseconds, a little-endian `u32` length,
//! then that many bytes of one WebSocket frame; a later run on the same day appends to it.
//! Captures can hold account data: never commit one without reviewing it.
#![cfg(feature = "live-tests")]

#[cfg(feature = "feed")]
#[path = "live/feed.rs"]
mod feed;

#[cfg(feature = "rest")]
#[path = "live/rest.rs"]
mod rest;

/// Helpers shared by both lanes.
#[cfg(any(feature = "rest", feature = "feed"))]
mod lane {
    use std::path::PathBuf;

    use chrono::{FixedOffset, NaiveDate, Utc};
    use dhani::{AccessToken, ClientId, Credentials};
    use tokio::sync::{Mutex, MutexGuard};

    static SERIAL: Mutex<()> = Mutex::const_new(());

    /// Waits until no other test in this binary is talking to Dhan. Hold the guard for the
    /// whole test.
    pub async fn serial() -> MutexGuard<'static, ()> {
        SERIAL.lock().await
    }

    /// Reads `var`; prints the skip line and returns `None` when it is unset or empty.
    pub fn env(var: &str) -> Option<String> {
        match std::env::var(var) {
            Ok(value) if !value.is_empty() => Some(value),
            _ => {
                println!("skipped: {var} not set");
                None
            }
        }
    }

    /// The credentials in `id_var` and `token_var`, or `None` (with a skip line) if either is
    /// unset.
    fn credentials_from(id_var: &str, token_var: &str) -> Option<Credentials> {
        let client_id = env(id_var)?;
        let access_token = env(token_var)?;
        Some(Credentials::new(
            ClientId::new(client_id).unwrap_or_else(|e| panic!("{id_var} is not a client ID: {e}")),
            AccessToken::new(access_token)
                .unwrap_or_else(|e| panic!("{token_var} is not an access token: {e}")),
        ))
    }

    #[cfg(feature = "rest")]
    pub fn sandbox_credentials() -> Option<Credentials> {
        credentials_from("DHANI_SANDBOX_CLIENT_ID", "DHANI_SANDBOX_ACCESS_TOKEN")
    }

    pub fn live_credentials() -> Option<Credentials> {
        credentials_from("DHANI_LIVE_CLIENT_ID", "DHANI_LIVE_ACCESS_TOKEN")
    }

    /// Today in India (UTC+05:30), which is what Dhan's dates mean.
    pub fn today() -> NaiveDate {
        let ist = FixedOffset::east_opt(5 * 3600 + 30 * 60).expect("a valid offset");
        Utc::now().with_timezone(&ist).date_naive()
    }

    /// `target/captures/<kind>`, created if missing.
    pub fn capture_dir(kind: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("captures")
            .join(kind);
        std::fs::create_dir_all(&dir).expect("create the capture directory");
        dir
    }
}
