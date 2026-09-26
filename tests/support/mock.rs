//! wiremock helpers: `client_for`, `expect_headers` and `body_json_eq`.

use dhani::config::{Environment, Urls};
use dhani::rest::RateLimiter;
use dhani::{AccessToken, ClientId, Credentials, DhanClient};
use wiremock::{Match, MockServer, Request};

/// The sentinel client ID every mock client authenticates with.
pub const CLIENT_ID: &str = "9999888877";
/// The sentinel, JWT-shaped access token every mock client authenticates with.
pub const ACCESS_TOKEN: &str = "eyJhbGciOiJIUzUxMiJ9.eyJzdWIiOiI5OTk5ODg4ODc3In0.U0VOVElORUwtU0lH";

/// The sentinel credentials.
pub fn credentials() -> Credentials {
    Credentials::new(
        ClientId::new(CLIENT_ID).unwrap(),
        AccessToken::new(ACCESS_TOKEN).unwrap(),
    )
}

/// URLs pointing every REST and CSV host at `server` (REST under `/v2`).
pub fn urls_for(server: &MockServer) -> Urls {
    let base = server.uri();
    let url = |path: &str| url::Url::parse(&format!("{base}{path}")).expect("mock URL");
    let mut urls = Urls::for_env(Environment::Live);
    urls.rest = url("/v2");
    urls.auth = url("");
    urls.scrip_master_compact = url("/csv/api-scrip-master.csv");
    urls.scrip_master_detailed = url("/csv/api-scrip-master-detailed.csv");
    urls.global_scrip_master = url("/csv/us-stock-scrip-master.csv");
    urls
}

/// A client for `server` with the sentinel credentials and no local rate limiting.
pub fn client_for(server: &MockServer) -> DhanClient {
    DhanClient::builder()
        .urls(urls_for(server))
        .rate_limiter(RateLimiter::disabled())
        .credentials(credentials())
        .build()
        .expect("mock client builds")
}

/// Matches requests carrying the sentinel `access-token` and `client-id` headers.
pub struct AuthHeaders;

impl Match for AuthHeaders {
    fn matches(&self, request: &Request) -> bool {
        let value = |name: &str| request.headers.get(name).and_then(|v| v.to_str().ok());
        value("access-token") == Some(ACCESS_TOKEN) && value("client-id") == Some(CLIENT_ID)
    }
}

/// The access-token and client-id matcher.
pub fn expect_headers() -> AuthHeaders {
    AuthHeaders
}

/// Matches a JSON body equal to `expected` (key order ignored).
pub fn body_json_eq(expected: serde_json::Value) -> impl Match {
    wiremock::matchers::body_json(expected)
}
