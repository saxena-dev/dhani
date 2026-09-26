//! Construction reads no environment variable.
//!
//! `Environment::default()` and `Urls::for_env` return the documented values whatever the
//! process environment holds. Setting a variable is only sound while no other thread reads the
//! environment, so this is the only test in its binary.

mod support;

use dhani::config::{Environment, Urls};

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
fn construction_ignores_the_environment() {
    // SAFETY: the only test in this binary. The harness reads its own variables before running
    // it, and nothing else in the process reads or writes the environment meanwhile.
    unsafe {
        std::env::set_var("DHAN_CLIENT_ID", "SENTINEL-env-client-id");
        std::env::set_var("DHAN_ACCESS_TOKEN", "SENTINEL-env-access-token");
        std::env::set_var("DHANI_BASE_URL", "http://sentinel.invalid/v2");
        std::env::set_var("DHANI_ENV", "sandbox");
    }

    assert_eq!(Environment::default(), Environment::Live);
    let live = Urls::for_env(Environment::Live);
    assert_eq!(live.rest.as_str(), "https://api.dhan.co/v2");
    let sandbox = Urls::for_env(Environment::Sandbox);
    assert_eq!(sandbox.rest.as_str(), "https://sandbox.dhan.co/v2");
    assert_eq!(non_rest(&sandbox), non_rest(&live));
    assert!(!non_rest(&live).iter().any(|u| u.contains("sentinel")));

    #[cfg(feature = "rest")]
    {
        let client = dhani::DhanClient::builder()
            .build()
            .expect("default client builds");
        assert!(client.credentials().is_none());
        assert_eq!(client.environment(), Environment::Live);
    }
}
