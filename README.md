# dhani

`dhani` is an asynchronous Rust client library for the [DhanHQ v2](https://dhanhq.co/docs/v2/)
developer API: REST trading and data calls, the streaming feeds, and `tracing`-based
observability with redaction.

**Status: 0.1.0 has not been run against a Dhan account.** Every request and response shape is
built from Dhan's published documentation and the official Python SDK, and tested against local
mock servers. The defaults it chooses where the sources are silent or disagree are listed in
[Unverified against a live account](#unverified-against-a-live-account).

## What 0.1.0 covers

| Area | Calls |
|---|---|
| Orders | place, place sliced, modify, cancel, order book, order by ID, order by correlation ID, trade book, trades of an order |
| Portfolio | holdings, positions, convert position, exit all positions |
| Funds | fund limit, margin, multi-instrument margin |
| Statements | ledger, trade history |
| Market quote | LTP, OHLC and full quote, typed or raw JSON |
| Historical data | daily and intraday candles |
| Option chain | chain and expiry list |
| Instruments | the compact and detailed scrip master CSVs (feature `instruments`) |
| Auth and account | access token from PIN and TOTP, token renewal, profile |
| Feeds | Live Market Feed (ticker, quote and full modes) and Live Order Update (individual and partner) |
| Environments | live, and the REST sandbox |

Later 0.x releases add super orders, forever orders, conditional and multi orders, trader's
control, EDIS, the Global Stocks APIs and feed, the 20- and 200-level depth feeds, rolling
options, per-segment instruments, and the consent, partner-consent and static-IP flows. The REST
facades for super, forever and conditional orders, trader's control, EDIS and Global Stocks
(`client.super_orders()` and so on) already exist but have no calls yet.

The library reads no environment variables, installs no global subscriber or metrics recorder,
and runs no background task for REST calls: everything is configured explicitly.

## Installation

```toml
[dependencies]
dhani = "0.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

The minimum supported Rust version is 1.88. TLS is rustls with the aws-lc-rs provider, which
needs a C compiler and, on some targets, CMake.

| Feature | Enables |
|---|---|
| `rest` *(default)* | the REST client and every REST facade |
| `feed` *(default)* | the WebSocket feeds (implies `decoder`) |
| `decoder` | the pure binary and JSON feed decoders, with no async runtime or network code |
| `instruments` | scrip master CSV download and parsing (implies `rest`) |
| `metrics` | metric emission through the `metrics` facade |
| `decimal` | `dhani::types::to_decimal()`, converting an `f64` price to `Option<rust_decimal::Decimal>` (`None` out of range) |
| `live-tests` | compiles the crate's own live test lane (repository checkouts only); adds no library code |

## REST quick start

```rust,no_run
# #[cfg(feature = "rest")]
# async fn example() -> dhani::Result<()> {
use dhani::rest::PlaceOrderRequest;
use dhani::types::{
    ExchangeSegment, OrderType, ProductType, SecurityId, TransactionType, Validity,
};
use dhani::{AccessToken, ClientId, Credentials, DhanClient};

// Load these however you like: dhani never reads the environment.
let credentials = Credentials::new(
    ClientId::new("1000000009")?,
    AccessToken::new("<access token>")?,
);
let client = DhanClient::builder().credentials(credentials).build()?;

let order = PlaceOrderRequest::new(
    ExchangeSegment::NseEq,
    SecurityId::new("1333")?,
    TransactionType::Buy,
    1,
    OrderType::Limit,
    ProductType::Cnc,
    Validity::Day,
)
.with_price(1642.5);
let ack = client.orders().place(&order).await?;
println!("placed {}", ack.order_id);

for o in client.orders().list().await? {
    println!("{} {:?}", o.order_id, o.order_status);
}
# Ok(())
# }
# fn main() {}
```

- **Sandbox.** Add `.environment(Environment::Sandbox)` to the builder. It changes only the
  REST host; the feeds have no sandbox.
- **Tokens.** `client.auth().generate_access_token(&client_id, &pin, &totp)` issues a token from
  a PIN and the current TOTP code; `client.with_credentials(token.credentials())` switches to it
  and keeps the same transport and rate limiter. dhani does not compute TOTP codes.
- **Errors.** `Error::kind()` says what went wrong. Reads and queries are retried on transient
  failures; mutations and token calls never are, and `Error::may_have_reached_server()` says whether a failed
  mutation might still have taken effect.

## Market feed

```rust,no_run
# #[cfg(feature = "feed")]
# async fn example(credentials: dhani::Credentials) -> Result<(), Box<dyn std::error::Error>> {
use dhani::feed::{FeedEvent, Instrument, MarketFeed, Mode};
use dhani::types::{ExchangeSegment, SecurityId};
use futures_util::StreamExt; // futures-util = "0.3"

let (handle, mut events, task) = MarketFeed::builder(credentials).spawn()?;
let instrument = Instrument::new(ExchangeSegment::NseEq, SecurityId::new("1333")?)?;
handle.subscribe([instrument], Mode::Ticker).await?;

while let Some(event) = events.next().await {
    match event? {
        FeedEvent::Data(d) => println!("{:?}", d.value),
        FeedEvent::Lifecycle(l) => println!("{l:?}"),
        FeedEvent::DecodeError(e) => eprintln!("skipped a packet: {e:?}"),
        _ => {}
    }
}
handle.shutdown().await?;
task.join().await;
# Ok(())
# }
# fn main() {}
```

The feed reconnects and restores its subscriptions on its own, and reports every transition as
a `FeedEvent::Lifecycle`. `OrderUpdateFeed::builder(credentials)` works the same way for order
updates.

> **Connection limit.** Dhan allows at most five WebSocket connections per user. Whether the limit
> is shared across feed types is undocumented, so budget as if it is. A sixth connection evicts
> the oldest one, which ends with disconnect code 805. dhani does not count connections for you,
> and it treats an 805 as terminal instead of reconnecting.

## Rate limits

Every client applies Dhan's published limits locally before sending, and clones of a client
share one limiter. A call that would wait longer than `max_wait` (5 s by default) fails with a
`RateLimited` error instead.

| Class | Limits | Applies to |
|---|---|---|
| Order | 10 per second, 250 per minute, 1000 per hour, 7000 per day | order placement, modification and cancellation |
| Data | 5 per second, 100 000 per day | historical data, option chain, expiry list |
| Quote | 1 per second | LTP, OHLC and full quote |
| Non-trading | 20 per second | every other REST call, including order and trade books |
| Token generation | 1 per 2 minutes | access token from PIN and TOTP |
| Unmetered | none | other auth-host calls and scrip master downloads |

Two more rules apply on top:

- The option chain allows one call per 3 seconds for the same underlying and expiry.
- An order can be modified at most 25 times. dhani counts each modify attempt it sends and
  resets the count at midnight IST.

Days are IST days. A daily ceiling fails at once rather than waiting.

## Observability

dhani emits `tracing` spans and events and, with the `metrics` feature, metrics. Credentials,
tokens and client IDs are redacted in every span, event, metric label, error message and `Debug`
output dhani produces. To see INFO lifecycle lines plus warnings and
errors (with `tracing-subscriber = { version = "0.3", features = ["env-filter"] }`):

```rust,no_run
tracing_subscriber::fmt()
    .with_env_filter("info,dhani=info,dhani::decode=warn")
    .init();
```

To export metrics, enable the feature (`dhani = { version = "0.1", features = ["metrics"] }`) and
install any `metrics` recorder, for example `metrics-exporter-prometheus`:

```rust,ignore
metrics_exporter_prometheus::PrometheusBuilder::new().install()?;
```

The market-feed URL and the order-update login message carry the access token. `tungstenite`
logs both at TRACE through the `log` crate, which `tracing_subscriber::fmt().init()` forwards, so
never enable TRACE for the `tungstenite` target (for example, use
`RUST_LOG=trace,tungstenite=debug`).

## Examples

From a checkout of the repository, each example runs against a local mock server and needs no
account.

| Example | Shows |
|---|---|
| `cargo run --example rest_basic` | placing an order and listing the order book |
| `cargo run --example totp_login` | a token from PIN and TOTP, then switching the client to it |
| `cargo run --example market_feed` | subscribing and reading market-feed events |
| `cargo run --example order_updates` | receiving an order update |
| `cargo run --example observability` | the logging setup above, with a retry warning |

## Unverified against a live account

0.1.0 was built without access to a Dhan account. Where Dhan's documentation is silent or
contradicts itself, or disagrees with the Python SDK, dhani ships the default below. Each one is
verified against a live account and the sandbox in Phase 2, after this release. A later release
may change a default if the check disagrees with it. The IDs in brackets refer to the project's
register of open questions (OQ) and source discrepancies (D, S).

**Defaults in effect in 0.1.0**

| Topic | Default |
|---|---|
| Rate classes (OQ-3) | Dhan names the rate classes but does not map endpoints to them. Order mutations are `Order`; order and trade books, exit-all and every other unmapped REST call are `Non-trading`. |
| Profile (OQ-6) | No schema is published, so `profile()` returns a `Profile` wrapping the raw JSON. |
| Market feed packets (OQ-7) | Packet sizes are fixed by code, and every packet in a frame is decoded; after a packet with an unknown code, the rest of the frame is dropped unless the next code is documented. Trade times (`ltt`) are exposed raw, since their epoch and timezone are not established; `ltt_unix()` reads them as Unix seconds without converting. Index (1) and MarketStatus (7) packets arrive as `MarketPacket::Other`. `as_legacy_index()` decodes an Index packet with a layout found only in a commented-out legacy table. |
| Order update feed (OQ-10) | The login acknowledgement, error format, keepalive and disconnect codes are undocumented. Unrecognised messages arrive as `OrderUpdateEvent::Other` with their raw JSON, and the client pings every 20 s. |
| Detailed scrip master (OQ-14) | The security-ID column name is undocumented, so `SECURITY_ID` is also read. Unrecognised columns are kept in `extra`. |
| Segment codes (OQ-16) | `NSE_COMM` has no known binary feed code: dhani sends a subscription for it, but its packets report no segment (`PacketHeader::segment()` is `None`). The currency segment codes 3 and 7 come from the Python SDK. |
| Market-feed mode change (OQ-17) | It is undocumented whether subscribing in a new mode replaces the old one. dhani unsubscribes the old mode, then subscribes the new one. |
| MARKET orders (OQ-21) | `price` is omitted, as the documentation allows. The Python SDK sends `0`. |
| Sandbox feeds (OQ-22) | No sandbox WebSocket endpoints are documented. The feed builders take no environment and connect to the live endpoints unless given `.url(..)`; Dhan, not dhani, rejects a sandbox token on a live feed. |
| Multi-instrument margin (OQ-24) | Between 1 and 50 instruments per call. The upper bound is dhani's own. |
| Modification cap (OQ-28) | The 25-modification cap is applied to order modifications only, counted per attempt and reset at midnight IST. |
| Correlation IDs (OQ-29) | Up to 30 characters from `A-Z a-z 0-9 _ -`. A `.` is refused locally, since it is unclear whether the documentation allows it. |
| Market quote data (OQ-31) | `data` is read as a two-level map, segment then security ID, as in the published example. |
| Multi-margin response keys (D60) | The documentation shows camelCase numbers and the API spec shows snake_case strings. Both are accepted. |

**Defaults fixed for later releases** (these features are not in 0.1.0)

| Topic | Default |
|---|---|
| Depth feeds (OQ-8, S1–S6) | (S1) The 200-level feed uses the documented `/twohundreddepth` path, overridable. (S2) Unsubscribe uses code 25. (S3, OQ-8) The disconnect reason is read at the documented offset, with the Python SDK's offset as a fallback and ambiguity reported. (S4) Packet 41 is buy and 51 is sell. (S5) 200-level packets are parsed by length and row count. (S6, OQ-8) The 200-level unsubscribe shape is configurable and defaults to flat. |
| Global feed times (OQ-9) | The trade time is exposed raw, with a `lut_unix()` helper. The documentation's sample implies a 1980 epoch. |
| Partner consent (OQ-13) | Consent generation uses POST, as labelled, and consumption uses GET, where the documentation's examples contradict the labels. |
| Segment instruments (OQ-14) | `/instrument/{segment}` sends the segment as a string, with auth headers. |
| Global orders (OQ-15) | Static-IP whitelisting is not checked locally. |
| Modification cap scope (OQ-28) | Super and forever order modifications are not counted. |
| Rolling options (OQ-30) | `expiryCode` takes 1 to 3, as the documentation says. The API spec says 0 to 2. |
| Global market-status keys (D64) | Both documented spellings of each key are accepted. |

## License

MIT
