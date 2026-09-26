# dhani

`dhani` is an asynchronous Rust client library for the DhanHQ v2 developer API. It covers the
REST trading APIs (orders, super orders, forever orders, conditional and multi orders, portfolio,
funds and margin, statements, trader's control and EDIS), the REST data APIs (market quote,
historical data, option chain and expiry list, and the instrument master), the auth and account
APIs, the Global Stocks REST APIs, the streaming feeds (the binary Live Market Feed, 20- and
200-level Full Market Depth, the JSON Live Order Update feed and the Global Stocks Live Feed),
the DhanHQ sandbox, and first-class observability through `tracing` spans and events with
redaction and optional metrics.

## Unverified against a live account
