# Changelog

All notable changes to this project are documented in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## 0.1.0 - Unreleased

The first release: an asynchronous client for the DhanHQ v2 API, built and tested without access
to a Dhan account (see [Unverified against a live account](#unverified-against-a-live-account)).

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
  25-modification cap. Reads and queries are retried on transient failures and remote rate
  limits; mutations and token calls are never retried.
- Typed errors: `Error::kind()`, the stage a failure happened at, broker error codes, and
  `may_have_reached_server()` for failed mutations.
- Request validation before sending. Tolerant decoding: unknown enum values are preserved as
  `Inbound::Unknown`, unknown fields are ignored, and fields that the sources type
  inconsistently accept both forms.
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

### Unverified against a live account

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
| Intraday interval (OQ-4) | The 25-minute interval, as the API spec, the guide and the Python SDK give it. Some endpoint pages say 30; there is no 30-minute option. |
| Market-feed mode change (OQ-17) | It is undocumented whether subscribing in a new mode replaces the old one. dhani unsubscribes the old mode, then subscribes the new one. |
| MARKET orders (OQ-21) | `price` is omitted, as the documentation allows. The Python SDK sends `0`. |
| Sandbox feeds (OQ-22) | No sandbox WebSocket endpoints are documented. The feed builders take no environment and connect to the live endpoints unless given `.url(..)`; Dhan, not dhani, rejects a sandbox token on a live feed. |
| Multi-instrument margin (OQ-24) | Between 1 and 50 instruments per call. The upper bound is dhani's own. |
| Modification cap (OQ-28) | The 25-modification cap is applied to order modifications only, counted per attempt and reset at midnight IST. |
| Correlation IDs (OQ-29) | Up to 30 characters from `A-Z a-z 0-9 _ -`. A `.` is refused locally, since it is unclear whether the documentation allows it, and so is a space, which the API spec allows but the order documentation does not list. |
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
