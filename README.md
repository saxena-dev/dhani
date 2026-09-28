dhani
=====

[![CI](https://github.com/saxena-dev/dhani/actions/workflows/ci.yml/badge.svg)](https://github.com/saxena-dev/dhani/actions/workflows/ci.yml)
[![Coverage](https://codecov.io/gh/saxena-dev/dhani/graph/badge.svg)](https://codecov.io/gh/saxena-dev/dhani)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org/)
[![Docs.rs](https://docs.rs/dhani/badge.svg)](https://docs.rs/dhani)
[![Crates.io](https://img.shields.io/crates/v/dhani.svg)](https://crates.io/crates/dhani)
[![Downloads](https://img.shields.io/crates/d/dhani.svg)](https://crates.io/crates/dhani)

**dhani** is an asynchronous Rust SDK for [Dhan](https://dhan.co/)'s
[DhanHQ](https://docs.dhanhq.co/) developer API (version 2): the REST API for orders, portfolio,
funds, statements and market data, and the WebSocket feeds for live market data and order
updates.

> **Dhani** (Hindi: धनी) n.: A person of wealth; someone who is rich.

A trading program earns its keep by being boringly correct. The bugs that cost money are
rarely the ones that crash: they are an order placed twice because a timeout was retried, a
rate limit discovered by being blocked, a feed that quietly stopped after a reconnect, an
access token copied into a log file. dhani is built so that none of these can happen without
you being told, with enough detail to decide what to do next.

## Why dhani

- **Failures are never disguised as success.** A call succeeds only when Dhan answers with a
  2xx status and a body of the expected shape. A 2xx answer that reports a failure is an
  error, and every error says what failed, where, and how far the request got.
- **Orders are never sent twice behind your back.** Every call that can change your account,
  and every token call, makes exactly one attempt. If the response is lost, the error says
  Dhan *may* have received the request, so you can check before trying again. Reads, which
  are safe to repeat, retry failures that happen before a response arrives, and 502, 503 and
  504 answers, on their own ([the exact rule](#rate-limits-retries-and-deadlines)).
- **Dhan's rate limits are enforced before you hit them.** Every published limit, from 10
  orders a second to 25 modifications per order, is applied locally. A request waits briefly
  for capacity, 5 seconds by default, and is refused with a `RateLimited` error rather than
  sent to be rejected by Dhan. Other programs using the same account still count against
  Dhan's limits, so leave them headroom.
- **Requests are checked before they are sent.** A quantity of zero, a LIMIT order without a
  price or a malformed ID is a `Validation` error, and nothing leaves your machine.
- **Secrets stay secret.** dhani keeps access tokens, client IDs, PINs and TOTP (time-based
  one-time password) codes out of its own errors, spans, events, metric labels and `Debug`
  output. Two things are yours to handle: raw messages you receive, such as unrecognised
  order updates, may contain your client ID, and the WebSocket library underneath can log
  credentials at TRACE (see [Observability](#observability)).
- **Nothing grows without bound.** Every queue, body, frame and wait has a documented default
  and a range. Exceeding one is an explicit error, not a slow leak.
- **The feeds never drop data silently.** Reconnects restore your subscriptions, every
  transition is reported as an event, and by default a consumer that falls behind gets an
  error, not a gap. If you choose to skip ahead instead, every drop is reported as an event.
  A packet that cannot be decoded is reported and skipped, never guessed at.
- **Unknown values are kept, not guessed.** When Dhan adds an order status, a segment or a
  message type, you receive it as an unknown value instead of a parse failure.
- **You own your telemetry.** dhani reads no environment variables and installs nothing
  globally. Plug in your own `tracing` subscriber and metrics recorder, or none at all.
- **Take only what you need.** The REST client, the feeds and the decoders are separate
  features. A decoder-only build has no Tokio or network dependency.

## Installation

```toml
[dependencies]
dhani = "0.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
futures-util = "0.3" # for StreamExt, to read the feeds' event streams
```

`rest` and `feed` are on by default. To depend on less, turn off the defaults and pick what
you use:

```toml
dhani = { version = "0.1", default-features = false, features = ["rest"] }
```

dhani needs **Rust 1.88** or newer. The REST client and the feeds run on the Tokio runtime you
already use, multi-threaded or not. TLS is rustls with the aws-lc-rs provider, which needs a C
compiler and, on some targets, CMake.

## Getting started

### Before you start

You need a Dhan trading account with DhanHQ API access. Dhan's Trading APIs are free for every
Dhan user; its Data APIs, such as market quotes, historical data and the market feed, carry
additional charges. Without access, Dhan rejects a call with `DH-902`, or with Data API error
806 when the Data APIs are not subscribed. Two more things are worth knowing up front:

- **Placing orders needs a static IP.** Dhan requires every account that places, modifies or
  cancels orders through the API to whitelist a static IP address ([Dhan's authentication
  guide](https://docs.dhanhq.co/api/v2/guides/authentication#setup-static-ip)). Reading order
  and trade details works without one.
- **The sandbox is separate.** A sandbox token comes from
  [developer.dhanhq.co](https://developer.dhanhq.co/), and the sandbox serves only some calls:
  see [step 2](#2-call-the-rest-api).

### 1. Get an access token

Dhan authenticates every call with your **client ID** and an **access token**. A token usually
stays valid for about 24 hours; a generated one tells you exactly when it expires in
`expiry_time`. You can generate the token on [web.dhan.co](https://web.dhan.co/) (My Profile, then
Access DhanHQ APIs) or, once TOTP is enabled for your Dhan account, have dhani generate one
from your PIN and the current TOTP code:

```rust,no_run
use dhani::credentials::{Pin, Totp};
use dhani::{ClientId, DhanClient};

#[tokio::main]
async fn main() -> dhani::Result<()> {
    // Token generation needs no credentials, so the client starts without them.
    let client = DhanClient::builder().build()?;
    let token = client
        .auth()
        .generate_access_token(
            &ClientId::new("1000000009")?,
            &Pin::new("123456")?,
            &Totp::new("654321")?, // the current code from your authenticator app
        )
        .await?;

    // The same client, now with credentials for every other call.
    let client = client.with_credentials(token.credentials());
    // Storing the token is up to you: dhani keeps nothing on disk.
    println!("token expires at {:?}", token.expiry_time);
    let _ = client;
    Ok(())
}
```

dhani never computes TOTP codes, because that would mean holding your TOTP seed. Dhan issues
at most one token every two minutes, and dhani enforces that locally too.

### 2. Call the REST API

Give the client your credentials, then reach each part of the API through it:

```rust,no_run
use dhani::{AccessToken, ClientId, Credentials, DhanClient};

#[tokio::main]
async fn main() -> dhani::Result<()> {
    // Load these however you like: dhani never reads the environment.
    let credentials = Credentials::new(
        ClientId::new("1000000009")?,
        AccessToken::new("<access token>")?,
    );
    let client = DhanClient::builder().credentials(credentials).build()?;

    let funds = client.funds().limits().await?;
    println!("available balance: {:?}", funds.available_balance);

    let holdings = client.portfolio().holdings().await?;
    let positions = client.portfolio().positions().await?;
    println!("{} holdings, {} positions", holdings.len(), positions.len());
    Ok(())
}
```

A `DhanClient` is cheap to clone. Clones share one connection pool and one set of rate limits,
so create it once and hand clones to the tasks that need it.

To try orders without touching your account, point the client at Dhan's sandbox and give it a
sandbox token:

```rust,no_run
use dhani::{Credentials, DhanClient, Environment};

fn sandbox_client(sandbox_credentials: Credentials) -> dhani::Result<DhanClient> {
    DhanClient::builder()
        .environment(Environment::Sandbox)
        .credentials(sandbox_credentials)
        .build()
}
```

Only the REST base URL changes. Of the calls in this release, Dhan's sandbox serves: placing,
slicing, modifying and cancelling orders; the order book, orders by ID or correlation ID, the trade
book and an order's trades; holdings, positions and position conversion; the fund limit and
single-instrument margin; the ledger and trade history; and daily and intraday candles.
Market quotes, option chains, multi-instrument margin, exiting all positions, the profile,
token calls and the feeds are not in the sandbox. Dhan's sandbox also serves the kill switch
and EDIS, which a later release of dhani will expose.

### 3. Place an order, carefully

Orders are built from a request type that is checked before anything is sent. Tag each order
with a correlation ID: if the response is lost, it is how you find the order again.

```rust,no_run
use dhani::DhanClient;
use dhani::rest::PlaceOrderRequest;
use dhani::types::{
    CorrelationId, ExchangeSegment, OrderType, ProductType, SecurityId, TransactionType,
    Validity,
};

async fn buy(client: &DhanClient) -> dhani::Result<()> {
    let tag = CorrelationId::new("rebalance-42")?;
    let order = PlaceOrderRequest::new(
        ExchangeSegment::NseEq,
        SecurityId::new("1333")?,
        TransactionType::Buy,
        1,
        OrderType::Limit,
        ProductType::Cnc,
        Validity::Day,
    )
    .with_price(1642.5)
    .with_correlation_id(tag.clone());

    match client.orders().place(&order).await {
        Ok(ack) => println!("accepted as {}", ack.order_id),
        Err(err) if err.may_have_reached_server() => {
            // The request left the process, so the order may exist. Look it up before
            // deciding to place it again.
            match client.orders().get_by_correlation_id(&tag).await {
                Ok(placed) => println!("it was placed: {}", placed.order_id),
                Err(_) => println!("not found yet: check the order book before retrying"),
            }
        }
        Err(err) => return Err(err),
    }
    Ok(())
}
```

An acknowledgement means Dhan accepted the order, not that it filled. Follow its progress with
`orders().get(..)`, or with the order-update feed below.

Dhan identifies an instrument by its numeric **security ID** together with its **exchange
segment**, not by its trading symbol: HDFC Bank on NSE is security ID 1333 in `NSE_EQ`. To
look one up, use the scrip master, Dhan's CSV list of every instrument: with the `instruments`
feature, `client.instruments().scrip_master(ScripMasterKind::Compact)` returns each one's
security ID, trading symbol and segment.

### 4. Stream live market data

A feed is a single background task that owns the WebSocket connection. You talk to it through
a handle and read everything it receives from one stream:

```rust,no_run
use dhani::feed::{FeedEvent, Instrument, MarketFeed, Mode};
use dhani::types::{ExchangeSegment, SecurityId};
use dhani::{AccessToken, ClientId, Credentials};
use futures_util::StreamExt; // futures-util = "0.3"

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let credentials = Credentials::new(
        ClientId::new("1000000009")?,
        AccessToken::new("<access token>")?,
    );
    let (handle, mut events, task) = MarketFeed::builder(credentials).spawn()?;
    let instrument = Instrument::new(ExchangeSegment::NseEq, SecurityId::new("1333")?)?;
    handle.subscribe([instrument], Mode::Ticker).await?;

    while let Some(event) = events.next().await {
        match event? {
            FeedEvent::Data(d) => println!("{:?}", d.value),
            FeedEvent::Lifecycle(change) => println!("{change:?}"),
            FeedEvent::DecodeError(e) => eprintln!("skipped a packet: {e:?}"),
            _ => {} // FeedEvent is non-exhaustive: later versions may add events.
        }
    }
    println!("feed ended: {:?}", task.join().await);
    Ok(())
}
```

A few things are worth knowing:

- **Subscriptions survive reconnects.** The feed remembers what you asked for and sends it
  again on every new connection, reporting each step as a `Lifecycle` event.
- **The stream tells you when it ends and why.** It ends with `None` only after you call
  `handle.shutdown()`. Any other ending, such as a rejected token or reconnect attempts
  running out, yields one error first.
- **Read promptly.** Delivery is bounded. If your consumer falls too far behind, the feed
  stops with an explicit error rather than dropping data you never saw. If you would rather
  skip ahead, `OverflowPolicy::DropOldest` drops the oldest data and tells you how much with a
  `Lifecycle::Lagged` event.
- **`Active` is not freshness.** It means your subscriptions were written to the connection.
  Whether a price is recent enough for your purpose is your call.
- **Keep a handle.** Dropping every `FeedHandle` stops the feed: the stream yields one
  `FeedError(TerminalReason::HandlesDropped)`, then ends. If you spawn the feed in a helper,
  return the handle along with the stream.
- **Mind the connection limit.** Dhan allows at most five WebSocket connections per user.
  Whether the limit is shared across feed types is undocumented, so budget as if it is. A
  sixth connection evicts the oldest one with disconnect code 805, which dhani treats as final
  rather than reconnecting into a loop.

`handle.status()` gives you a snapshot at any time: the connection state, how many
subscriptions you want and whether the latest change has been sent (`desired_revision` and
`sent_revision`), the queue's depth, how long ago the last frame arrived,
recent failures and, once the feed has stopped, why.

### 5. Follow your orders

The order-update feed, `OrderUpdateFeed`, works the same way for your account's order updates:

```rust,no_run
use dhani::Credentials;
use dhani::feed::{FeedEvent, OrderUpdateEvent, OrderUpdateFeed};
use futures_util::StreamExt;

async fn follow(credentials: Credentials) -> Result<(), Box<dyn std::error::Error>> {
    let (handle, mut events, task) = OrderUpdateFeed::builder(credentials).spawn()?;
    while let Some(event) = events.next().await {
        if let FeedEvent::Data(d) = event? {
            if let OrderUpdateEvent::Order(order) = d.value {
                println!("{:?} is now {:?}", order.order_no, order.status);
            }
        }
    }
    drop(handle);
    println!("feed ended: {:?}", task.join().await);
    Ok(())
}
```

Message types dhani does not recognise arrive as `OrderUpdateEvent::Other` with their raw JSON.
A message it cannot parse arrives as `FeedEvent::DecodeError`, so nothing is skipped without
you being told.

### 6. Decode captured frames

The market-feed decoder works on plain bytes and needs no runtime, so it is just as useful for
replaying captured data. With `default-features = false, features = ["decoder"]` it builds
with no network stack at all:

```rust
use dhani::decoder::split_market;

fn decode(frame: &[u8]) {
    for packet in split_market(frame) {
        match packet {
            Ok(packet) => println!("{packet:?}"),
            Err(e) => eprintln!("the rest of the frame is unreadable: {e:?}"),
        }
    }
}
```

Prices arrive exactly as Dhan sends them, as `f32`. Trade times are exposed raw, because
their epoch is not documented; `ltt_unix()` reads them as Unix seconds without converting.

## Handling errors

Every failed call is a `dhani::Error`. Its **kind** tells you what happened, and its **stage**
tells you how far the request got:

```rust,no_run
use dhani::{DhanClient, ErrorKind};

async fn holdings(client: &DhanClient) {
    match client.portfolio().holdings().await {
        Ok(holdings) => println!("{} holdings", holdings.len()),
        Err(e) if e.kind() == ErrorKind::Auth => {
            // The access token expired or was revoked: get a new one.
        }
        Err(e) => eprintln!("{e} (may have reached Dhan: {})", e.may_have_reached_server()),
    }
}
```

| `ErrorKind` | What it means |
|---|---|
| `Validation` | your request was invalid; nothing was sent |
| `Config` | the client could not be built, or has no credentials for this call; nothing was sent |
| `Credential` | a client ID, token, PIN or TOTP was malformed; nothing was sent |
| `RateLimited` | a local limit refused the request (nothing was sent), or Dhan answered 429 or `DH-904`; `rate_limit()` says which |
| `Timeout` | an attempt or the whole operation ran out of time |
| `Transport` | the connection, TLS or network failed, or a body was cut short |
| `Auth` | Dhan rejected the credentials |
| `Api` | Dhan returned an error, available through `api()` with its `DH-9xx` or data-API code and its message |
| `HttpStatus` | an error status without Dhan's error body |
| `Decode` | a response arrived but did not have the expected shape |

The stage is `NotSent` only when dhani knows for certain that the request never left your
machine. Anything later means Dhan may have acted on it, and `may_have_reached_server()` says
so in one call. An error never contains a URL, a header, a request body, a credential or a
value from the response; Dhan's own error message is kept, sanitised and bounded.

## Cancellation and timeouts

Every call is an ordinary future, so `tokio::select!` and `tokio::time::timeout` work as you'd
expect, with one thing to keep in mind. A future does nothing until it is first polled, and
dropping it before the request is sent cancels only local work. Dropping it after the request
was sent does not cancel anything at Dhan: the order may still be placed. Treat a cancelled
order call like a lost response, and look the order up.

Feed commands are the same. Once `subscribe` has handed a command to the feed, dropping its
future does not withdraw it; `desired_revision` in `handle.status()` shows what the feed is
working towards.

## Rate limits, retries and deadlines

You don't need to configure anything to stay within Dhan's published limits:

| Class | Limits | Applies to |
|---|---|---|
| Order | 10 per second, 250 per minute, 1000 per hour, 7000 per day | order placement, modification and cancellation |
| Data | 5 per second, 100 000 per day | historical data, option chain, expiry list |
| Quote | 1 per second | LTP (last traded price), OHLC (open, high, low, close) and full quote |
| Non-trading | 20 per second | every other REST call, including order and trade books |
| Token generation | 1 per 2 minutes | access token from PIN and TOTP |
| No local limit | none | scrip master downloads (Dhan's servers may still limit them) |

Two more rules apply on top. The option chain allows one call per 3 seconds for the same
underlying and expiry, and Dhan caps each order at 25 modifications. dhani enforces that cap
with its own counter: it counts each modify attempt it sends, and resets the count at midnight
IST (India Standard Time). Days are IST days, and a daily ceiling refuses the request straight
away rather than waiting. The numbers follow the
rate-limit table on Dhan's API overview.

By default a request waits up to 5 seconds for capacity, and each operation has 30 seconds to
finish. A read makes up to three attempts with jittered backoff between them. It retries a
timeout or a transport failure before any response arrives, a 502, 503 or 504 answer, and one
remote rate limit (429, `DH-904` or data error 805) after at least a second. A failure while
reading a response body is returned without a retry. Calls that can change your account, and
token calls, always make exactly one attempt.

When your application needs different bounds, set them once on the builder:
[`timeouts`](https://docs.rs/dhani/latest/dhani/rest/struct.DhanClientBuilder.html#method.timeouts),
`retry`, `limits` (body sizes), `rate_limiter`, `http_client` (your own `reqwest` client) and
`user_agent_suffix`. Every value is checked against a documented range:

```rust,no_run
use std::time::Duration;
use dhani::DhanClient;
use dhani::rest::{AdmissionLimits, QuotaProfile, RateLimiter, RetryPolicy, Timeouts};

fn client() -> Result<DhanClient, Box<dyn std::error::Error>> {
    let limiter = RateLimiter::new(
        QuotaProfile::dhan_v2(),
        AdmissionLimits::new(Duration::from_secs(2), 256)?, // wait at most 2 s for capacity
    );
    Ok(DhanClient::builder()
        .timeouts(Timeouts::new(
            Duration::from_secs(5),  // connect
            Duration::from_secs(10), // one attempt
            Duration::from_secs(20), // the whole operation
        )?)
        .retry(RetryPolicy::new(2, Duration::from_millis(250), Duration::from_secs(2))?)
        .rate_limiter(limiter)
        .build()?)
}
```

Dhan applies its limits per account, so everything using the same account should share one
limiter. Clones of a client already do. For clients built separately, pass the same one to
each: `.rate_limiter(shared.clone())`.

## Observability

dhani emits `tracing` spans and events for every request and feed, and, with the `metrics`
feature, metrics through the `metrics` facade. Nothing is installed for you. To see INFO
lifecycle lines plus warnings and errors (with
`tracing-subscriber = { version = "0.3", features = ["env-filter"] }`):

```rust,no_run
tracing_subscriber::fmt()
    .with_env_filter("info,dhani=info,dhani::decode=warn")
    .init();
```

To export metrics, enable the feature (`dhani = { version = "0.1", features = ["metrics"] }`)
and install any `metrics` recorder, for example `metrics-exporter-prometheus`:

```rust,ignore
metrics_exporter_prometheus::PrometheusBuilder::new().install()?;
```

REST calls open a `dhani.http.request` span with one `dhani.http.attempt` child per attempt,
and each feed opens a `dhani.ws.session` span with one `dhani.ws.connection` child per
connection, so any `tracing-opentelemetry` layer shows them as traces.
Metric labels come from small, closed sets, so they won't blow up your metrics backend.

One caution: the market-feed URL and the order-update login message carry your access token.
`tungstenite`, the WebSocket library underneath, logs both at TRACE through the `log` crate,
which `tracing_subscriber::fmt().init()` forwards. Never enable TRACE for the `tungstenite`
target; `RUST_LOG=trace,tungstenite=debug` keeps everything else at TRACE.

## What dhani does not do

Knowing the edges saves surprises:

- It does not store, refresh or revoke credentials on its own, and it does not compute TOTP
  codes.
- It does not decide whether market data is fresh enough, or whether an order filled. It tells
  you what Dhan said and when.
- It does not persist anything: no caches, no files, no databases.
- It never retries a request that could change your account.
- It does not count WebSocket connections across feeds for you.
- It does not yet cover super, forever or conditional orders, trader's control, EDIS, Global
  Stocks (REST and feed), static IP management, the depth feeds, rolling options or the
  consent flows. The REST facades for the first six exist with no calls yet; all of these
  arrive in later 0.x releases.

## How dhani is tested

Trust has to be earned, so here is how dhani tries to earn it:

- **Upstream fixtures.** Contract tests run against the response fixtures that ship with
  [DhanHQ-py](https://github.com/dhan-oss/DhanHQ-py), Dhan's Python SDK, pinned to a commit,
  plus fixtures written from Dhan's documentation and OpenAPI spec. All 82 are listed with
  their origin and SHA-256. Expected values are written out by hand, and request bodies are
  compared exactly.
- **Every endpoint has a contract test.** A test fails the build if any shipped endpoint lacks
  one, and another pins every public method's signature.
- **Hostile networks.** A raw-TCP fault harness drops responses, stalls bodies, truncates
  them, refuses connections and serves redirects, and checks that mutations are never resent and
  reads retry only what is safe.
- **Hostile inputs.** Seeded property campaigns feed the binary decoders tens of thousands of
  generated, concatenated, truncated and mutated frames, checking that decoding never panics,
  never loops and round-trips exactly.
- **No leaked secrets.** A sentinel sweep plants a client ID, token, PIN and TOTP, drives
  successes, broker errors and feed failures, and checks that none of them appears in any
  span, event, metric label, error or `Debug` output.
- **Nothing leaves your machine.** Every test runs against loopback servers. A separate live
  lane, for a Dhan account and the sandbox, compiles in CI and runs only on demand with your
  own credentials.
- **Every claim has a source.** Behaviour that follows Dhan's documentation cites the exact
  lines, and a test checks that every citation resolves.
- **Every configuration is tested.** CI runs the tests in six feature configurations on Rust
  1.88 and on stable, builds the packaged crate in each, and checks the dependency graph: a
  decoder-only build pulls in no Tokio, HTTP or WebSocket stack, and `rest` and `feed` each
  leave out the other's.

## Examples

Each example runs against a local mock server, so you can try everything without an account.
They are in the repository, not the published crate:

| Example | What it shows |
|---|---|
| `rest_basic` | placing an order and listing the order book |
| `totp_login` | a token from PIN and TOTP, then switching the client to it |
| `market_feed` | subscribing and reading market-feed events, then a clean shutdown |
| `order_updates` | receiving an order update |
| `observability` | the logging setup above, with a retry warning |

```text
cargo run --example rest_basic
cargo run --example market_feed
```

## Supported DhanHQ APIs

- **Orders**: place, place sliced, modify and cancel; the order book, an order by ID or
  correlation ID, the trade book, and an order's trades
- **Portfolio**: holdings, positions, position conversion, and exiting all positions
- **Funds**: fund limit, and margin for one instrument or many
- **Statements**: ledger and trade history
- **Market quote**: LTP, OHLC and full quote with market depth, typed or as raw JSON
- **Historical data**: daily and intraday candles, with open interest
- **Option chain**: the chain with greeks, and the expiry list
- **Instruments**: the compact and detailed scrip master CSVs (feature `instruments`)
- **Auth and account**: access token from PIN and TOTP, token renewal, and the profile
- **WebSocket**: the Live Market Feed in ticker, quote and full modes, and the Live Order
  Update feed for individual and partner accounts
- **Environments**: live, and the REST sandbox

## Feature flags

| Feature | What it adds | Default |
|---|---|---|
| `rest` | `DhanClient` and every REST call | yes |
| `feed` | the WebSocket feeds (implies `decoder`) | yes |
| `decoder` | pure decoders for binary feed packets and order-update JSON, with no runtime or network code | no |
| `instruments` | scrip master CSV download and parsing (implies `rest`) | no |
| `metrics` | metric emission through the `metrics` facade | no |
| `decimal` | `dhani::types::to_decimal()`, converting an `f64` price to `Option<rust_decimal::Decimal>` (`None` out of range) | no |
| `live-tests` | compiles the crate's own live test lane (repository checkouts only); adds no library code | no |

Credentials, configuration, errors, the shared types and the telemetry schema are always
available.

## Documentation

- [API reference on docs.rs](https://docs.rs/dhani)
- [`CHANGELOG.md`](CHANGELOG.md): what changed in each release.
- [`docs/dhan-sources.toml`](https://github.com/saxena-dev/dhani/blob/main/docs/dhan-sources.toml):
  the Dhan documentation every `DOC:` citation in the API reference refers to.

## Status and disclaimer

dhani is pre-1.0 and still changing. Breaking changes ship only in minor-version bumps, each
listed in the [changelog](CHANGELOG.md).

dhani is an independent open-source project. It is not affiliated with Dhan in any way, and
Dhan neither makes, endorses nor supports it. For questions and bug reports, please
[open an issue](https://github.com/saxena-dev/dhani/issues).

The software is provided "as is", without warranty of any kind. The author and contributors
take no responsibility for any financial losses, damages or other issues arising from its use.
Nothing in this crate is a statement that data is fresh, complete or fit for trading. Test
against your own requirements, and in the sandbox, before trading real money.

## License

[MIT](LICENSE)
