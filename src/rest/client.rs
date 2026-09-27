//! `DhanClient` and `DhanClientBuilder`, with the bounded configuration types `Timeouts`,
//! `RetryPolicy` and `BodyLimits`.
//!
//! Every numeric bound here is SDK policy (a local decision, configurable within the stated
//! ranges), except the one-second minimum rate-limit backoff.

use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::HeaderValue;
use serde::de::DeserializeOwned;

use crate::backoff::SplitMix64;
use crate::config::{Environment, Urls};
use crate::credentials::Credentials;
use crate::error::{ConfigError, Error, Result, ValidationError};
use crate::labels::EndpointId;
use crate::rest::api::{
    Account, Auth, ConditionalOrders, Edis, ForeverOrders, Funds, GlobalStocks, Historical,
    MarketQuote, OptionChain, Orders, Portfolio, Statements, SuperOrders, TraderControl,
};
use crate::rest::endpoint::{self, Endpoint};
use crate::rest::ratelimit::RateLimiter;
use crate::rest::retry::RetryLimits;
use crate::rest::transport::{Call, Transport, TransportSettings};
use crate::types::{OrderId, RawJson};

fn check(
    field: &'static str,
    ok: bool,
    reason: &'static str,
) -> std::result::Result<(), ConfigError> {
    if ok {
        Ok(())
    } else {
        Err(ConfigError::new(field, reason))
    }
}

const MS: Duration = Duration::from_millis(1);
const SEC: Duration = Duration::from_secs(1);

/// Request deadlines (SDK policy).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeouts {
    /// TCP and TLS connect timeout: 100 ms..=60 s (default 5 s).
    pub connect: Duration,
    /// One attempt, from send to the end of the body: 1 s..=120 s (default 15 s).
    pub attempt: Duration,
    /// The whole operation, covering admission, attempts and backoff: `attempt`..=300 s
    /// (default 30 s).
    pub operation: Duration,
}

impl Timeouts {
    /// Range-checked timeouts.
    pub fn new(
        connect: Duration,
        attempt: Duration,
        operation: Duration,
    ) -> std::result::Result<Self, ConfigError> {
        let t = Timeouts {
            connect,
            attempt,
            operation,
        };
        t.validate()?;
        Ok(t)
    }

    pub(crate) fn validate(&self) -> std::result::Result<(), ConfigError> {
        check(
            "connect",
            (100 * MS..=60 * SEC).contains(&self.connect),
            "must be between 100 ms and 60 s",
        )?;
        check(
            "attempt",
            (SEC..=120 * SEC).contains(&self.attempt),
            "must be between 1 s and 120 s",
        )?;
        check(
            "operation",
            (self.attempt..=300 * SEC).contains(&self.operation),
            "must be between the attempt timeout and 300 s",
        )
    }
}

impl Default for Timeouts {
    fn default() -> Self {
        Timeouts {
            connect: 5 * SEC,
            attempt: 15 * SEC,
            operation: 30 * SEC,
        }
    }
}

/// Retry settings for reads and read-only queries (SDK policy). Mutations and session calls are
/// never retried.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Total attempts per operation, including the first: 1..=5 (default 3).
    pub max_attempts: u32,
    /// Base of the full-jitter backoff: 50 ms..=5 s (default 250 ms).
    pub initial_backoff: Duration,
    /// Cap of the full-jitter backoff: `initial_backoff`..=30 s (default 4 s).
    pub max_backoff: Duration,
    /// Extra attempts allowed after a remote rate limit, counted inside `max_attempts`: 0..=3
    /// (default 1).
    pub rate_limit_retries: u32,
    /// Base of the rate-limit backoff: 1 s..=30 s (default 1 s; the documentation asks for at
    /// least one second between retries, DOC:5328).
    pub rate_limit_initial_backoff: Duration,
    /// Seed for the jitter generator; random when `None`.
    pub jitter_seed: Option<u64>,
}

impl RetryPolicy {
    /// Range-checked policy with the default rate-limit settings.
    pub fn new(
        max_attempts: u32,
        initial: Duration,
        max: Duration,
    ) -> std::result::Result<Self, ConfigError> {
        let p = RetryPolicy {
            max_attempts,
            initial_backoff: initial,
            max_backoff: max,
            ..Self::default()
        };
        p.validate()?;
        Ok(p)
    }

    /// The same policy with a fixed jitter seed, for reproducible delays.
    pub fn with_jitter_seed(mut self, seed: u64) -> Self {
        self.jitter_seed = Some(seed);
        self
    }

    /// A policy that never retries (`max_attempts == 1`).
    pub fn none() -> Self {
        RetryPolicy {
            max_attempts: 1,
            ..Self::default()
        }
    }

    pub(crate) fn validate(&self) -> std::result::Result<(), ConfigError> {
        check(
            "max_attempts",
            (1..=5).contains(&self.max_attempts),
            "must be between 1 and 5",
        )?;
        check(
            "initial_backoff",
            (50 * MS..=5 * SEC).contains(&self.initial_backoff),
            "must be between 50 ms and 5 s",
        )?;
        check(
            "max_backoff",
            (self.initial_backoff..=30 * SEC).contains(&self.max_backoff),
            "must be between the initial backoff and 30 s",
        )?;
        check(
            "rate_limit_retries",
            self.rate_limit_retries <= 3,
            "must be between 0 and 3",
        )?;
        check(
            "rate_limit_initial_backoff",
            (SEC..=30 * SEC).contains(&self.rate_limit_initial_backoff),
            "must be between 1 s and 30 s",
        )
    }

    pub(crate) fn limits(&self) -> RetryLimits {
        RetryLimits {
            max_attempts: self.max_attempts,
            initial_backoff: self.initial_backoff,
            max_backoff: self.max_backoff,
            rate_limit_retries: self.rate_limit_retries,
            rate_limit_initial_backoff: self.rate_limit_initial_backoff,
        }
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy {
            max_attempts: 3,
            initial_backoff: 250 * MS,
            max_backoff: 4 * SEC,
            rate_limit_retries: 1,
            rate_limit_initial_backoff: SEC,
            jitter_seed: None,
        }
    }
}

const KIB: usize = 1024;
const MIB: usize = 1024 * KIB;

/// Size bounds on request and response bodies (SDK policy).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BodyLimits {
    /// Largest request body: 1 KiB..=16 MiB (default 1 MiB).
    pub max_request_bytes: usize,
    /// Largest JSON response body: 64 KiB..=256 MiB (default 8 MiB).
    pub max_json_response_bytes: usize,
    /// Largest CSV response body: 1 MiB..=1 GiB (default 256 MiB).
    pub max_csv_response_bytes: usize,
}

impl BodyLimits {
    /// Range-checked limits.
    pub fn new(
        max_request_bytes: usize,
        max_json_response_bytes: usize,
        max_csv_response_bytes: usize,
    ) -> std::result::Result<Self, ConfigError> {
        let l = BodyLimits {
            max_request_bytes,
            max_json_response_bytes,
            max_csv_response_bytes,
        };
        l.validate()?;
        Ok(l)
    }

    pub(crate) fn validate(&self) -> std::result::Result<(), ConfigError> {
        check(
            "max_request_bytes",
            (KIB..=16 * MIB).contains(&self.max_request_bytes),
            "must be between 1 KiB and 16 MiB",
        )?;
        check(
            "max_json_response_bytes",
            (64 * KIB..=256 * MIB).contains(&self.max_json_response_bytes),
            "must be between 64 KiB and 256 MiB",
        )?;
        check(
            "max_csv_response_bytes",
            (MIB..=1024 * MIB).contains(&self.max_csv_response_bytes),
            "must be between 1 MiB and 1 GiB",
        )
    }
}

impl Default for BodyLimits {
    fn default() -> Self {
        BodyLimits {
            max_request_bytes: MIB,
            max_json_response_bytes: 8 * MIB,
            max_csv_response_bytes: 256 * MIB,
        }
    }
}

/// Builds a [`DhanClient`]. Every value is validated at [`build`](DhanClientBuilder::build).
#[derive(Default)]
pub struct DhanClientBuilder {
    environment: Environment,
    urls: Option<Urls>,
    credentials: Option<Credentials>,
    timeouts: Timeouts,
    retry: RetryPolicy,
    limits: BodyLimits,
    rate_limiter: Option<RateLimiter>,
    http_client: Option<reqwest::Client>,
    user_agent_suffix: Option<String>,
}

impl fmt::Debug for DhanClientBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DhanClientBuilder")
            .field("environment", &self.environment)
            .field("credentials", &self.credentials)
            .field("timeouts", &self.timeouts)
            .field("retry", &self.retry)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl DhanClientBuilder {
    /// The environment whose documented URLs are used (default `Live`).
    pub fn environment(mut self, env: Environment) -> Self {
        self.environment = env;
        self
    }

    /// Explicit base URLs, overriding the environment's.
    pub fn urls(mut self, urls: Urls) -> Self {
        self.urls = Some(urls);
        self
    }

    /// Credentials for authenticated calls; auth-host calls need none.
    pub fn credentials(mut self, credentials: Credentials) -> Self {
        self.credentials = Some(credentials);
        self
    }

    /// Request deadlines.
    pub fn timeouts(mut self, timeouts: Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Retry settings for reads and queries.
    pub fn retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// Body size bounds.
    pub fn limits(mut self, limits: BodyLimits) -> Self {
        self.limits = limits;
        self
    }

    /// A rate limiter to share with other clients of the same account.
    pub fn rate_limiter(mut self, limiter: RateLimiter) -> Self {
        self.rate_limiter = Some(limiter);
        self
    }

    /// A caller-supplied HTTP client (for a proxy or custom roots). dhani's attempt and
    /// operation deadlines still apply; its connect timeout is the client's own. Configure it not
    /// to follow redirects: a redirect would forward the credential headers to another host.
    pub fn http_client(mut self, client: reqwest::Client) -> Self {
        self.http_client = Some(client);
        self
    }

    /// Text appended to the `User-Agent` header: `dhani/<version> <suffix>`.
    pub fn user_agent_suffix(mut self, suffix: impl Into<String>) -> Self {
        self.user_agent_suffix = Some(suffix.into());
        self
    }

    /// Validates the configuration and builds the client. Invalid values fail with
    /// [`ErrorKind::Config`](crate::ErrorKind::Config).
    pub fn build(self) -> Result<DhanClient> {
        let urls = self.urls.unwrap_or_else(|| Urls::for_env(self.environment));
        urls.validate().map_err(Error::from_config)?;
        self.timeouts.validate().map_err(Error::from_config)?;
        self.retry.validate().map_err(Error::from_config)?;
        self.limits.validate().map_err(Error::from_config)?;
        let agent = match &self.user_agent_suffix {
            Some(suffix) => format!("dhani/{} {suffix}", env!("CARGO_PKG_VERSION")),
            None => format!("dhani/{}", env!("CARGO_PKG_VERSION")),
        };
        let user_agent = HeaderValue::from_str(&agent).map_err(|_| {
            Error::from_config(ConfigError::new(
                "user_agent_suffix",
                "must be visible ASCII",
            ))
        })?;
        let http = match self.http_client {
            Some(client) => client,
            None => reqwest::Client::builder()
                .connect_timeout(self.timeouts.connect)
                .user_agent(user_agent.clone())
                .gzip(true)
                // Never follow redirects: they would carry the credential headers elsewhere and
                // could re-send a mutation's body.
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| {
                    Error::from_config(ConfigError::new("http_client", "could not be built"))
                })?,
        };
        let settings = TransportSettings {
            attempt_timeout: self.timeouts.attempt,
            operation_timeout: self.timeouts.operation,
            retry: self.retry.limits(),
            max_request_bytes: self.limits.max_request_bytes,
            max_json_response_bytes: self.limits.max_json_response_bytes,
            max_csv_response_bytes: self.limits.max_csv_response_bytes,
        };
        let seed = self
            .retry
            .jitter_seed
            .unwrap_or_else(|| SplitMix64::from_entropy().next_u64());
        let transport = Transport::new(
            http,
            urls,
            self.environment,
            settings,
            self.rate_limiter.unwrap_or_default(),
            user_agent,
            seed,
        );
        Ok(DhanClient {
            transport: Arc::new(transport),
            credentials: self.credentials,
        })
    }
}

/// The DhanHQ REST client: cheap to clone, `Send + Sync`, one handle per set of credentials.
#[derive(Clone)]
pub struct DhanClient {
    transport: Arc<Transport>,
    credentials: Option<Credentials>,
}

const _: () = {
    fn assert<T: Send + Sync + Clone + 'static>() {}
    let _ = assert::<DhanClient>;
};

impl fmt::Debug for DhanClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DhanClient")
            .field("environment", &self.transport.environment)
            .field("credentials", &self.credentials)
            .finish_non_exhaustive()
    }
}

macro_rules! facades {
    ($($(#[$m:meta])* $name:ident => $ty:ident;)+) => {
        $(
            $(#[$m])*
            pub fn $name(&self) -> $ty<'_> {
                $ty::new(self)
            }
        )+
    };
}

impl DhanClient {
    /// A builder with the documented defaults.
    pub fn builder() -> DhanClientBuilder {
        DhanClientBuilder::default()
    }

    /// A new handle on the same transport and rate limiter with different credentials (token
    /// rotation).
    pub fn with_credentials(&self, credentials: Credentials) -> DhanClient {
        DhanClient {
            transport: Arc::clone(&self.transport),
            credentials: Some(credentials),
        }
    }

    /// This handle's credentials.
    pub fn credentials(&self) -> Option<&Credentials> {
        self.credentials.as_ref()
    }

    /// The environment the client was built for.
    pub fn environment(&self) -> Environment {
        self.transport.environment
    }

    /// The rate limiter this client admits requests through.
    pub fn rate_limiter(&self) -> &RateLimiter {
        &self.transport.limiter
    }

    facades! {
        /// Orders and trades.
        orders => Orders;
        /// Super orders.
        super_orders => SuperOrders;
        /// Forever orders.
        forever_orders => ForeverOrders;
        /// Conditional and multi orders.
        conditional => ConditionalOrders;
        /// Holdings and positions.
        portfolio => Portfolio;
        /// Funds and margin.
        funds => Funds;
        /// Ledger and trade history.
        statements => Statements;
        /// Kill switch and P&L exit.
        trader_control => TraderControl;
        /// EDIS.
        edis => Edis;
        /// Market quotes.
        market_quote => MarketQuote;
        /// Historical data.
        historical => Historical;
        /// Option chain and expiry list.
        option_chain => OptionChain;
        /// Token renewal, profile and static IP.
        account => Account;
        /// Consent flows and token generation on the auth host.
        auth => Auth;
        /// Global Stocks.
        global => GlobalStocks;
    }

    /// The instrument master.
    #[cfg(feature = "instruments")]
    pub fn instruments(&self) -> crate::rest::api::Instruments<'_> {
        crate::rest::api::Instruments::new(self)
    }

    /// Runs any endpoint through the full pipeline and returns its body as raw JSON (CSV text as
    /// a JSON string; nothing for an empty body). Test support until every facade exists.
    #[doc(hidden)]
    pub async fn __execute_for_tests(
        &self,
        id: EndpointId,
        path_args: &[&str],
        query: &[(&'static str, String)],
        body: Option<serde_json::Value>,
        order_id: Option<&OrderId>,
    ) -> Result<Option<RawJson>> {
        let query: Vec<(&'static str, Cow<'_, str>)> = query
            .iter()
            .map(|(k, v)| (*k, Cow::Borrowed(v.as_str())))
            .collect();
        let prepare = || {
            Ok(Call {
                path_args,
                query: &query,
                body,
                order_id,
                ..Call::empty()
            })
        };
        self.transport
            .execute_raw(self.credentials.as_ref(), endpoint::by_id(id), prepare)
            .await
    }
}

/// The pipeline entry points the facades call.
#[allow(
    dead_code,
    reason = "called by the endpoint methods of each facade group"
)]
impl DhanClient {
    pub(crate) async fn execute<'a, T: DeserializeOwned>(
        &self,
        ep: &'static Endpoint,
        prepare: impl FnOnce() -> std::result::Result<Call<'a>, ValidationError>,
    ) -> Result<T> {
        self.transport
            .execute(self.credentials.as_ref(), ep, prepare)
            .await
    }

    pub(crate) async fn execute_empty<'a>(
        &self,
        ep: &'static Endpoint,
        prepare: impl FnOnce() -> std::result::Result<Call<'a>, ValidationError>,
    ) -> Result<()> {
        self.transport
            .execute_empty(self.credentials.as_ref(), ep, prepare)
            .await
    }

    pub(crate) async fn execute_opt<'a, T: DeserializeOwned>(
        &self,
        ep: &'static Endpoint,
        prepare: impl FnOnce() -> std::result::Result<Call<'a>, ValidationError>,
    ) -> Result<Option<T>> {
        self.transport
            .execute_opt(self.credentials.as_ref(), ep, prepare)
            .await
    }

    pub(crate) async fn execute_csv<'a, T>(
        &self,
        ep: &'static Endpoint,
        prepare: impl FnOnce() -> std::result::Result<Call<'a>, ValidationError>,
        parse: impl FnOnce(&str) -> std::result::Result<T, String>,
    ) -> Result<T> {
        self.transport
            .execute_csv(self.credentials.as_ref(), ep, prepare, parse)
            .await
    }

    pub(crate) async fn execute_text<'a>(
        &self,
        ep: &'static Endpoint,
        prepare: impl FnOnce() -> std::result::Result<Call<'a>, ValidationError>,
    ) -> Result<String> {
        self.transport
            .execute_text(self.credentials.as_ref(), ep, prepare)
            .await
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
