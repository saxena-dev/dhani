//! Which Dhan environment a client talks to, and the base URL of every host.
//!
//! Most programs need only [`Environment`]: the default, [`Environment::Live`], or
//! [`Environment::Sandbox`] to try calls against Dhan's sandbox with a sandbox token. [`Urls`]
//! holds every base URL behind them, for pointing a client or feed at another host, such as a
//! local mock server in your tests.
//!
//! The sandbox changes only the REST base URL. Of the calls in this release, it serves orders
//! and trades, holdings, positions and position conversion, the fund limit and
//! single-instrument margin, the ledger and trade history, and historical candles. Market
//! quotes, option chains, multi-instrument margin, exiting all positions, the profile, token
//! calls and the feeds are not in it.

use url::Url;

use crate::error::ConfigError;

/// The DhanHQ environment a client talks to.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Environment {
    /// The production environment (the default).
    #[default]
    Live,
    /// The DhanHQ sandbox (DOC:3840-3896), which dhani supports. Only the REST
    /// base URL differs from [`Live`]: the documentation gives no sandbox counterpart for the
    /// other hosts, and sandbox WebSocket endpoints are undocumented.
    ///
    /// [`Live`]: Environment::Live
    Sandbox,
}

/// Base URLs for every DhanHQ host the crate talks to.
///
/// Build one with [`Urls::for_env`] and override individual fields to point a client or feed at
/// another host (for example a local mock). The values are parsed [`Url`]s, so a malformed base
/// URL is rejected when it is constructed, not on the first request.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct Urls {
    /// REST API base: `https://api.dhan.co/v2` (DOC:5699) for [`Environment::Live`],
    /// `https://sandbox.dhan.co/v2` (DOC:3848-3850, DOC:5700) for [`Environment::Sandbox`].
    pub rest: Url,
    /// Auth host: `https://auth.dhan.co` (DOC:4323). It has no sandbox counterpart.
    pub auth: Url,
    /// Live Market Feed WebSocket: `wss://api-feed.dhan.co` (DOC:5859).
    pub market_feed: Url,
    /// Live Order Update WebSocket: `wss://api-order-update.dhan.co` (DOC:6072).
    pub order_update: Url,
    /// 20-level Full Market Depth WebSocket: `wss://depth-api-feed.dhan.co/twentydepth`
    /// (DOC:5457).
    pub depth_20: Url,
    /// 200-level Full Market Depth WebSocket: `wss://full-depth-api.dhan.co/twohundreddepth`
    /// (DOC:5465). The Python SDK connects to the host root instead; dhani follows the
    /// documentation and the URL can be overridden.
    pub depth_200: Url,
    /// Global Stocks Live Feed WebSocket: `wss://global-stocks-api-feed.dhan.co/` (DOC:1846).
    pub global_feed: Url,
    /// Compact instrument master CSV:
    /// `https://images.dhan.co/api-data/api-scrip-master.csv` (DOC:5729).
    pub scrip_master_compact: Url,
    /// Detailed instrument master CSV:
    /// `https://images.dhan.co/api-data/api-scrip-master-detailed.csv` (DOC:5735).
    pub scrip_master_detailed: Url,
    /// Global Stocks instrument master CSV:
    /// `https://api-global-stocks.dhan.co/api-data/us-stock-scrip-master.csv` (DOC:5806).
    pub global_scrip_master: Url,
}

const REST_LIVE: &str = "https://api.dhan.co/v2";
const REST_SANDBOX: &str = "https://sandbox.dhan.co/v2";
const AUTH: &str = "https://auth.dhan.co";
const MARKET_FEED: &str = "wss://api-feed.dhan.co";
const ORDER_UPDATE: &str = "wss://api-order-update.dhan.co";
const DEPTH_20: &str = "wss://depth-api-feed.dhan.co/twentydepth";
const DEPTH_200: &str = "wss://full-depth-api.dhan.co/twohundreddepth";
const GLOBAL_FEED: &str = "wss://global-stocks-api-feed.dhan.co/";
const SCRIP_MASTER_COMPACT: &str = "https://images.dhan.co/api-data/api-scrip-master.csv";
const SCRIP_MASTER_DETAILED: &str = "https://images.dhan.co/api-data/api-scrip-master-detailed.csv";
const GLOBAL_SCRIP_MASTER: &str =
    "https://api-global-stocks.dhan.co/api-data/us-stock-scrip-master.csv";

/// Parses one of the constant URLs above; they are fixed literals covered by unit tests.
fn fixed(url: &str) -> Url {
    Url::parse(url).expect("constant base URL is valid")
}

impl Urls {
    /// The documented base URLs for `env`.
    ///
    /// [`Environment::Sandbox`] changes only [`rest`](Urls::rest); every other field keeps its
    /// production value, because the documentation gives no sandbox counterpart for those hosts.
    pub fn for_env(env: Environment) -> Self {
        let rest = match env {
            Environment::Live => REST_LIVE,
            Environment::Sandbox => REST_SANDBOX,
        };
        Self {
            rest: fixed(rest),
            auth: fixed(AUTH),
            market_feed: fixed(MARKET_FEED),
            order_update: fixed(ORDER_UPDATE),
            depth_20: fixed(DEPTH_20),
            depth_200: fixed(DEPTH_200),
            global_feed: fixed(GLOBAL_FEED),
            scrip_master_compact: fixed(SCRIP_MASTER_COMPACT),
            scrip_master_detailed: fixed(SCRIP_MASTER_DETAILED),
            global_scrip_master: fixed(GLOBAL_SCRIP_MASTER),
        }
    }

    /// Checks that every REST and CSV URL uses `https` and every feed URL uses `wss`, each with a
    /// host. Plain `http`/`ws` is accepted only for a loopback host (a local test server). The
    /// client builder calls this, so a bad base URL fails at `build()`, not on the first request.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let https = [
            ("urls.rest", &self.rest),
            ("urls.auth", &self.auth),
            ("urls.scrip_master_compact", &self.scrip_master_compact),
            ("urls.scrip_master_detailed", &self.scrip_master_detailed),
            ("urls.global_scrip_master", &self.global_scrip_master),
        ];
        let wss = [
            ("urls.market_feed", &self.market_feed),
            ("urls.order_update", &self.order_update),
            ("urls.depth_20", &self.depth_20),
            ("urls.depth_200", &self.depth_200),
            ("urls.global_feed", &self.global_feed),
        ];
        for (field, url) in https {
            check_url(field, url, "https", "must use https")?;
        }
        for (field, url) in wss {
            check_url(field, url, "wss", "must use wss")?;
        }
        Ok(())
    }
}

fn check_url(
    field: &'static str,
    url: &Url,
    scheme: &str,
    reason: &'static str,
) -> Result<(), ConfigError> {
    // Plain http/ws is accepted only for loopback hosts (local test servers).
    let plain = if scheme == "https" { "http" } else { "ws" };
    let loopback = matches!(url.host(), Some(url::Host::Ipv4(ip)) if ip.is_loopback())
        || matches!(url.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback())
        || url.host_str() == Some("localhost");
    if url.scheme() != scheme && !(url.scheme() == plain && loopback) {
        return Err(ConfigError::new(field, reason));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(ConfigError::new(field, "must have a host"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_environment_is_live() {
        assert_eq!(Environment::default(), Environment::Live);
    }

    #[test]
    fn live_urls_match_the_documented_literals() {
        let u = Urls::for_env(Environment::Live);
        // A special-scheme URL with an empty path serialises with a trailing "/".
        assert_eq!(u.rest.as_str(), "https://api.dhan.co/v2");
        assert_eq!(u.auth.as_str(), "https://auth.dhan.co/");
        assert_eq!(u.market_feed.as_str(), "wss://api-feed.dhan.co/");
        assert_eq!(u.order_update.as_str(), "wss://api-order-update.dhan.co/");
        assert_eq!(
            u.depth_20.as_str(),
            "wss://depth-api-feed.dhan.co/twentydepth"
        );
        assert_eq!(
            u.depth_200.as_str(),
            "wss://full-depth-api.dhan.co/twohundreddepth"
        );
        assert_eq!(
            u.global_feed.as_str(),
            "wss://global-stocks-api-feed.dhan.co/"
        );
        assert_eq!(
            u.scrip_master_compact.as_str(),
            "https://images.dhan.co/api-data/api-scrip-master.csv"
        );
        assert_eq!(
            u.scrip_master_detailed.as_str(),
            "https://images.dhan.co/api-data/api-scrip-master-detailed.csv"
        );
        assert_eq!(
            u.global_scrip_master.as_str(),
            "https://api-global-stocks.dhan.co/api-data/us-stock-scrip-master.csv"
        );
    }

    /// Every field except `rest`, in declaration order.
    fn non_rest(u: &Urls) -> [&str; 9] {
        [
            u.auth.as_str(),
            u.market_feed.as_str(),
            u.order_update.as_str(),
            u.depth_20.as_str(),
            u.depth_200.as_str(),
            u.global_feed.as_str(),
            u.scrip_master_compact.as_str(),
            u.scrip_master_detailed.as_str(),
            u.global_scrip_master.as_str(),
        ]
    }

    #[test]
    fn documented_urls_validate() {
        assert_eq!(Urls::for_env(Environment::Live).validate(), Ok(()));
        assert_eq!(Urls::for_env(Environment::Sandbox).validate(), Ok(()));
    }

    #[test]
    fn validate_rejects_wrong_schemes() {
        let mut urls = Urls::for_env(Environment::Live);
        urls.rest = Url::parse("http://api.dhan.co/v2").unwrap();
        let err = urls.validate().unwrap_err();
        assert_eq!((err.field, err.reason), ("urls.rest", "must use https"));

        // Loopback test servers may use plain http and ws.
        let mut urls = Urls::for_env(Environment::Live);
        urls.rest = Url::parse("http://127.0.0.1:8080/v2").unwrap();
        urls.auth = Url::parse("http://localhost:8080").unwrap();
        urls.market_feed = Url::parse("ws://[::1]:9000").unwrap();
        assert_eq!(urls.validate(), Ok(()));
        urls.rest = Url::parse("http://10.0.0.1/v2").unwrap();
        assert_eq!(urls.validate().unwrap_err().field, "urls.rest");

        let mut urls = Urls::for_env(Environment::Live);
        urls.market_feed = Url::parse("https://api-feed.dhan.co").unwrap();
        let err = urls.validate().unwrap_err();
        assert_eq!(
            (err.field, err.reason),
            ("urls.market_feed", "must use wss")
        );

        // `url` refuses https/wss URLs without a host, so a non-special scheme stands in for one.
        let mut urls = Urls::for_env(Environment::Live);
        urls.auth = Url::parse("data:text/plain,x").unwrap();
        assert_eq!(urls.validate().unwrap_err().field, "urls.auth");
    }

    #[test]
    fn sandbox_changes_only_the_rest_base() {
        let live = Urls::for_env(Environment::Live);
        let sandbox = Urls::for_env(Environment::Sandbox);
        assert_eq!(sandbox.rest.as_str(), "https://sandbox.dhan.co/v2");
        assert_eq!(non_rest(&sandbox), non_rest(&live));
    }
}
