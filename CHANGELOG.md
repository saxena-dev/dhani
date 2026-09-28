# Changelog

All notable changes to this project are documented in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## 0.1.0 - 2026-09-28

The first release: an asynchronous client for the DhanHQ v2 API.

### Added

- `DhanClient`, with a builder for the environment (live or the REST sandbox), credentials,
  base URLs, timeouts, retry policy, body limits, rate limiter, HTTP client and user-agent
  suffix.
  `with_credentials` switches credentials and keeps the transport and rate limiter.
- REST facades:
  - orders: place, place sliced, modify, cancel, order book, order by ID or correlation ID,
    trade book, trades of an order;
  - portfolio: holdings, positions, convert position, exit all;
  - funds: fund limit, margin, multi-instrument margin;
  - statements: ledger, trade history;
  - market quote: LTP, OHLC and full quote, typed or raw;
  - historical: daily and intraday candles;
  - option chain: chain and expiry list;
  - auth and account: access token from PIN and TOTP, token renewal, profile;
  - instruments (feature `instruments`): the compact and detailed scrip master CSVs, parsed off
    the async runtime.
- REST facades for super orders, forever orders, conditional orders, trader's control, EDIS and
  Global Stocks, with no calls yet.
- Local rate limiting of Dhan's published limits, the per-key option-chain window and the
  25-modification cap. Reads and queries retry timeouts and transport failures before a
  response arrives, 502, 503 and 504 answers, and one remote rate limit; a failure while
  reading a response body is not retried. Calls that can change your account, and token
  calls, are never retried.
- Typed errors: `Error::kind()`, the stage a failure happened at, broker error codes, and
  `may_have_reached_server()` for failed mutations.
- Request validation before sending. Tolerant decoding: unknown enum values are preserved as
  `Inbound::Unknown`, unknown fields are ignored, and fields that Dhan's documentation types
  inconsistently (a number in one place, a string in another) accept both.
- Feeds (feature `feed`):
  - `MarketFeed` (ticker, quote and full modes) and `OrderUpdateFeed` (individual and partner);
  - reconnection with jittered backoff and subscription restore;
  - lifecycle events, bounded delivery with a `Fail` or `DropOldest` overflow policy, a
    liveness timeout and optional raw frame capture.
- Pure decoders (feature `decoder`) for market-feed binary packets and order-update JSON.
- `tracing` spans and events for every request and feed, with credentials, tokens and client
  IDs redacted. Optional metrics through the `metrics` facade (feature `metrics`).
- `decimal` feature: `dhani::types::to_decimal()` converts an `f64` price to an
  `Option<rust_decimal::Decimal>` (`None` out of range).
- Examples that run against local mock servers: `rest_basic`, `totp_login`, `market_feed`,
  `order_updates` and `observability`.
