//! Public-surface assertions: every public item is reachable at its documented path, and the
//! client types have the documented auto traits.

mod support;

#[cfg(feature = "rest")]
const _: () = {
    fn assert<T: Send + Sync + Clone + 'static>() {}
    let _ = assert::<dhani::DhanClient>;
};

#[cfg(feature = "rest")]
#[test]
fn the_client_is_send_sync_and_clone() {
    fn assert<T: Send + Sync + Clone + 'static>() {}
    assert::<dhani::DhanClient>();
    assert::<dhani::rest::DhanClient>();
}

#[cfg(feature = "feed")]
const _: () = {
    fn handle<T: Send + Sync + Clone + 'static>() {}
    fn events<T: Send + Unpin + 'static>() {}
    let _ = handle::<dhani::feed::FeedHandle<dhani::feed::MarketSub>>;
    let _ = handle::<dhani::feed::FeedHandle<()>>;
    let _ = events::<dhani::feed::FeedEvents<dhani::feed::MarketEvent>>;
    let _ = events::<dhani::feed::FeedEvents<dhani::feed::OrderUpdateEvent>>;
};

/// Every public feed item at its documented path.
#[cfg(feature = "feed")]
#[test]
fn the_feed_items_are_reachable_at_their_documented_paths() {
    #[allow(unused_imports)]
    use dhani::feed::{
        CommandError, Delivery, DisconnectReason, FailureRecord, FeedBuilder, FeedError, FeedEvent,
        FeedEvents, FeedHandle, FeedLimits, FeedSpawnError, FeedState, FeedStatus, FeedTask,
        FeedTypes, FrameKind, Instrument, Lifecycle, MarketEvent, MarketFeed, MarketProtocol,
        MarketSub, Mode, OrderUpdateEvent, OrderUpdateFeed, OrderUpdateProtocol, OverflowPolicy,
        RawFrame, ReconnectPolicy, Revision, SubscriptionCommand, SubscriptionError, TaskOutcome,
        TerminalReason,
    };
    let credentials = dhani::Credentials::new(
        dhani::ClientId::new("1000000009").unwrap(),
        dhani::AccessToken::new("token").unwrap(),
    );
    let _: FeedBuilder<MarketProtocol> = MarketFeed::builder(credentials.clone());
    let _: FeedBuilder<OrderUpdateProtocol> = OrderUpdateFeed::builder(credentials);
    fn types<P: FeedTypes>() {}
    types::<MarketProtocol>();
    types::<OrderUpdateProtocol>();
    assert_eq!(OverflowPolicy::default(), OverflowPolicy::Fail);
    assert_eq!(Revision(3).0, 3);
}

/// Every MVP REST method (endpoint rows A5–A7, O1–O9, P1–P4, M1–M3, T1–T2, Q1–Q3, H1–H2, X1–X2,
/// I1–I2) with its full signature: each typed async closure is checked by the compiler and never
/// run.
#[cfg(feature = "rest")]
#[test]
fn every_mvp_rest_method_has_its_documented_signature() {
    use chrono::NaiveDate;
    use dhani::credentials::{Pin, Totp};
    use dhani::rest::{
        Candles, ConvertPositionRequest, DailyRequest, FullQuote, FundLimits, HistoricalTrade,
        Holding, IntradayRequest, IssuedToken, LedgerEntry, LtpQuote, Margin, MarginRequest,
        ModifyOrderRequest, MultiMargin, MultiMarginRequest, OhlcQuote, OptionChainData,
        OptionChainRequest, Order, OrderAck, PlaceOrderRequest, Position, Profile, QuoteData,
        QuoteRequest, Trade, UnderlyingRef,
    };
    use dhani::types::{CorrelationId, OrderId, RawJson};
    use dhani::{ClientId, DhanClient, Result};

    // Auth and account.
    let _ = async |c: &DhanClient, id: &ClientId, pin: &Pin, totp: &Totp| -> Result<IssuedToken> {
        c.auth().generate_access_token(id, pin, totp).await
    }; // A5
    let _ = async |c: &DhanClient| -> Result<IssuedToken> { c.account().renew_token().await }; // A6
    let _ = async |c: &DhanClient| -> Result<Profile> { c.account().profile().await }; // A7

    // Orders and trades.
    let _ = async |c: &DhanClient, r: &PlaceOrderRequest| -> Result<OrderAck> {
        c.orders().place(r).await
    }; // O1
    let _ = async |c: &DhanClient, r: &PlaceOrderRequest| -> Result<Vec<OrderAck>> {
        c.orders().place_sliced(r).await
    }; // O2
    let _ = async |c: &DhanClient, r: &ModifyOrderRequest| -> Result<OrderAck> {
        c.orders().modify(r).await
    }; // O3
    let _ =
        async |c: &DhanClient, id: &OrderId| -> Result<OrderAck> { c.orders().cancel(id).await }; // O4
    let _ = async |c: &DhanClient| -> Result<Vec<Order>> { c.orders().list().await }; // O5
    let _ = async |c: &DhanClient, id: &OrderId| -> Result<Order> { c.orders().get(id).await }; // O6
    let _ = async |c: &DhanClient, id: &CorrelationId| -> Result<Order> {
        c.orders().get_by_correlation_id(id).await
    }; // O7
    let _ = async |c: &DhanClient| -> Result<Vec<Trade>> { c.orders().trades().await }; // O8
    let _ = async |c: &DhanClient, id: &OrderId| -> Result<Vec<Trade>> {
        c.orders().trades_for_order(id).await
    }; // O9

    // Portfolio.
    let _ = async |c: &DhanClient| -> Result<Vec<Holding>> { c.portfolio().holdings().await }; // P1
    let _ = async |c: &DhanClient| -> Result<Vec<Position>> { c.portfolio().positions().await }; // P2
    let _ = async |c: &DhanClient, r: &ConvertPositionRequest| -> Result<()> {
        c.portfolio().convert_position(r).await
    }; // P3
    let _ = async |c: &DhanClient| -> Result<()> { c.portfolio().exit_all().await }; // P4

    // Funds and margin.
    let _ = async |c: &DhanClient| -> Result<FundLimits> { c.funds().limits().await }; // M1
    let _ =
        async |c: &DhanClient, r: &MarginRequest| -> Result<Margin> { c.funds().margin(r).await }; // M2
    let _ = async |c: &DhanClient, r: &MultiMarginRequest| -> Result<MultiMargin> {
        c.funds().margin_multi(r).await
    }; // M3

    // Statements.
    let _ = async |c: &DhanClient, from: NaiveDate, to: NaiveDate| -> Result<Vec<LedgerEntry>> {
        c.statements().ledger(from, to).await
    }; // T1
    let _ = async |c: &DhanClient,
                   from: NaiveDate,
                   to: NaiveDate,
                   page: u32|
           -> Result<Vec<HistoricalTrade>> {
        c.statements().trade_history(from, to, page).await
    }; // T2

    // Market quote, typed and raw.
    let _ = async |c: &DhanClient, r: &QuoteRequest| -> Result<QuoteData<LtpQuote>> {
        c.market_quote().ltp(r).await
    }; // Q1
    let _ = async |c: &DhanClient, r: &QuoteRequest| -> Result<QuoteData<OhlcQuote>> {
        c.market_quote().ohlc(r).await
    }; // Q2
    let _ = async |c: &DhanClient, r: &QuoteRequest| -> Result<QuoteData<FullQuote>> {
        c.market_quote().quote(r).await
    }; // Q3
    let _ = async |c: &DhanClient, r: &QuoteRequest| -> Result<RawJson> {
        c.market_quote().ltp_raw(r).await
    }; // Q1 raw
    let _ = async |c: &DhanClient, r: &QuoteRequest| -> Result<RawJson> {
        c.market_quote().ohlc_raw(r).await
    }; // Q2 raw
    let _ = async |c: &DhanClient, r: &QuoteRequest| -> Result<RawJson> {
        c.market_quote().quote_raw(r).await
    }; // Q3 raw

    // Historical data.
    let _ = async |c: &DhanClient, r: &DailyRequest| -> Result<Candles> {
        c.historical().daily(r).await
    }; // H1
    let _ = async |c: &DhanClient, r: &IntradayRequest| -> Result<Candles> {
        c.historical().intraday(r).await
    }; // H2

    // Option chain.
    let _ = async |c: &DhanClient, r: &OptionChainRequest| -> Result<OptionChainData> {
        c.option_chain().chain(r).await
    }; // X1
    let _ = async |c: &DhanClient, r: &OptionChainRequest| -> Result<RawJson> {
        c.option_chain().chain_raw(r).await
    }; // X1 raw
    let _ = async |c: &DhanClient, u: &UnderlyingRef| -> Result<Vec<NaiveDate>> {
        c.option_chain().expiries(u).await
    }; // X2

    // Instruments (I1 compact, I2 detailed).
    #[cfg(feature = "instruments")]
    {
        use dhani::rest::{InstrumentRecord, ScripMasterKind};
        let _ = async |c: &DhanClient, kind: ScripMasterKind| -> Result<Vec<InstrumentRecord>> {
            c.instruments().scrip_master(kind).await
        }; // I1, I2
    }
}

/// The facade futures are `Send`, so callers can `tokio::spawn` them: one method per facade
/// group, type-checked and never called.
#[cfg(feature = "rest")]
#[allow(dead_code, reason = "compiled for its type checks only")]
fn facade_futures_are_send(
    c: &dhani::DhanClient,
    order: &dhani::rest::PlaceOrderRequest,
    quote: &dhani::rest::QuoteRequest,
    daily: &dhani::rest::DailyRequest,
    chain: &dhani::rest::OptionChainRequest,
    ids: (
        &dhani::ClientId,
        &dhani::credentials::Pin,
        &dhani::credentials::Totp,
    ),
    day: chrono::NaiveDate,
) {
    fn send<F: std::future::Future + Send>(_: F) {}
    send(c.orders().place(order));
    send(c.orders().list());
    send(c.portfolio().holdings());
    send(c.funds().limits());
    send(c.statements().ledger(day, day));
    send(c.market_quote().ltp(quote));
    send(c.historical().daily(daily));
    send(c.option_chain().chain(chain));
    send(c.auth().generate_access_token(ids.0, ids.1, ids.2));
    send(c.account().profile());
    #[cfg(feature = "instruments")]
    send(
        c.instruments()
            .scrip_master(dhani::rest::ScripMasterKind::Compact),
    );
}
