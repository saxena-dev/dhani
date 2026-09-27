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
