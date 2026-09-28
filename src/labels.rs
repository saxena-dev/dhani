//! Closed label enums shared by the error model, observability and the REST layer:
//! `EndpointId`, `Method`, `RateClass`, `RetryClass` and `FeedKind`.
//!
//! These are always compiled and name no REST-only type, so errors, spans and metrics can carry
//! them in every feature combination. Their `as_str` values are span and metric label values:
//! closed, stable, and free of any identifier or secret.

/// An HTTP method.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    /// `GET`.
    Get,
    /// `POST`.
    Post,
    /// `PUT`.
    Put,
    /// `DELETE`.
    Delete,
}

impl Method {
    /// The label value: `get`, `post`, `put` or `delete`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Post => "post",
            Self::Put => "put",
            Self::Delete => "delete",
        }
    }
}

/// How a failed request may be retried.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RetryClass {
    /// A `GET` read.
    Read,
    /// A read-only `POST` (for example a quote or margin query).
    Query,
    /// A state-changing request: exactly one attempt.
    Mutation,
    /// An auth or token request: exactly one attempt.
    Session,
}

impl RetryClass {
    /// The label value: `read`, `query`, `mutation` or `session`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Query => "query",
            Self::Mutation => "mutation",
            Self::Session => "session",
        }
    }
}

/// The rate-limit class a request is admitted under.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RateClass {
    /// Order placement, modification and cancellation.
    Order,
    /// Data APIs.
    Data,
    /// Market quote APIs.
    Quote,
    /// Other non-trading APIs.
    NonTrading,
    /// Access-token generation.
    TokenGeneration,
    /// Not rate limited locally.
    Unmetered,
}

impl RateClass {
    /// The label value, in snake_case.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Order => "order",
            Self::Data => "data",
            Self::Quote => "quote",
            Self::NonTrading => "non_trading",
            Self::TokenGeneration => "token_generation",
            Self::Unmetered => "unmetered",
        }
    }
}

/// A WebSocket feed.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FeedKind {
    /// The Live Market Feed.
    Market,
    /// 20-level Full Market Depth.
    Depth20,
    /// 200-level Full Market Depth.
    Depth200,
    /// The Live Order Update feed.
    OrderUpdate,
    /// The Global Stocks Live Feed.
    Global,
}

impl FeedKind {
    /// The label value: `market`, `depth20`, `depth200`, `order_update` or `global`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Market => "market",
            Self::Depth20 => "depth20",
            Self::Depth200 => "depth200",
            Self::OrderUpdate => "order_update",
            Self::Global => "global",
        }
    }
}

/// Declares `EndpointId` from `(variant => label)` pairs, with `ALL` and `as_str`.
macro_rules! endpoint_ids {
    ($($variant:ident => $label:literal,)+) => {
        /// One REST endpoint. Its label is `<group>.<verb>`.
        #[non_exhaustive]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum EndpointId {
            $(
                #[doc = concat!("`", $label, "`.")]
                $variant,
            )+
        }

        impl EndpointId {
            /// Every endpoint, in declaration order.
            pub const ALL: &'static [EndpointId] = &[$(Self::$variant),+];

            /// The label value, `<group>.<verb>`; stable, used in spans and metrics.
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $label,)+
                }
            }
        }
    };
}

endpoint_ids! {
    AuthGenerateConsent => "auth.generate_consent",
    AuthConsumeConsent => "auth.consume_consent",
    AuthPartnerGenerateConsent => "auth.partner_generate_consent",
    AuthPartnerConsumeConsent => "auth.partner_consume_consent",
    AuthGenerateAccessToken => "auth.generate_access_token",
    AccountRenewToken => "account.renew_token",
    AccountProfile => "account.profile",
    AccountSetIp => "account.set_ip",
    AccountModifyIp => "account.modify_ip",
    AccountGetIp => "account.get_ip",
    OrdersPlace => "orders.place",
    OrdersPlaceSliced => "orders.place_sliced",
    OrdersModify => "orders.modify",
    OrdersCancel => "orders.cancel",
    OrdersList => "orders.list",
    OrdersGet => "orders.get",
    OrdersGetByCorrelation => "orders.get_by_correlation",
    TradesList => "trades.list",
    TradesForOrder => "trades.for_order",
    SuperOrdersPlace => "super_orders.place",
    SuperOrdersModify => "super_orders.modify",
    SuperOrdersCancelLeg => "super_orders.cancel_leg",
    SuperOrdersList => "super_orders.list",
    ForeverOrdersPlace => "forever_orders.place",
    ForeverOrdersModify => "forever_orders.modify",
    ForeverOrdersCancel => "forever_orders.cancel",
    ForeverOrdersList => "forever_orders.list",
    ConditionalPlace => "conditional.place",
    ConditionalModify => "conditional.modify",
    ConditionalDelete => "conditional.delete",
    ConditionalGet => "conditional.get",
    ConditionalList => "conditional.list",
    ConditionalPlaceMulti => "conditional.place_multi",
    PortfolioHoldings => "portfolio.holdings",
    PortfolioPositions => "portfolio.positions",
    PortfolioConvertPosition => "portfolio.convert_position",
    PortfolioExitAll => "portfolio.exit_all",
    FundsLimits => "funds.limits",
    FundsMargin => "funds.margin",
    FundsMarginMulti => "funds.margin_multi",
    StatementsLedger => "statements.ledger",
    StatementsTradeHistory => "statements.trade_history",
    TraderControlSetKillSwitch => "trader_control.set_kill_switch",
    TraderControlKillSwitchStatus => "trader_control.kill_switch_status",
    TraderControlSetPnlExit => "trader_control.set_pnl_exit",
    TraderControlPnlExit => "trader_control.pnl_exit",
    TraderControlStopPnlExit => "trader_control.stop_pnl_exit",
    EdisGenerateTpin => "edis.generate_tpin",
    EdisForm => "edis.form",
    EdisBulkForm => "edis.bulk_form",
    EdisInquire => "edis.inquire",
    MarketQuoteLtp => "market_quote.ltp",
    MarketQuoteOhlc => "market_quote.ohlc",
    MarketQuoteQuote => "market_quote.quote",
    HistoricalDaily => "historical.daily",
    HistoricalIntraday => "historical.intraday",
    HistoricalRollingOptions => "historical.rolling_options",
    OptionChainChain => "option_chain.chain",
    OptionChainExpiries => "option_chain.expiries",
    InstrumentsScripMasterCompact => "instruments.scrip_master_compact",
    InstrumentsScripMasterDetailed => "instruments.scrip_master_detailed",
    InstrumentsSegment => "instruments.segment",
    InstrumentsGlobalScripMaster => "instruments.global_scrip_master",
    GlobalMarketStatus => "global.market_status",
    GlobalFundLimit => "global.fund_limit",
    GlobalHoldings => "global.holdings",
    GlobalOrders => "global.orders",
    GlobalOrder => "global.order",
    GlobalTrades => "global.trades",
    GlobalTradesForSecurity => "global.trades_for_security",
    GlobalMargin => "global.margin",
    GlobalEstimate => "global.estimate",
    GlobalPlace => "global.place",
    GlobalModify => "global.modify",
    GlobalCancel => "global.cancel",
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn is_snake_label(s: &str) -> bool {
        !s.is_empty()
            && !s.starts_with('_')
            && !s.ends_with('_')
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    }

    #[test]
    fn endpoint_ids_are_complete_unique_and_well_formed() {
        assert_eq!(EndpointId::ALL.len(), 75);
        let labels: HashSet<&str> = EndpointId::ALL.iter().map(|e| e.as_str()).collect();
        assert_eq!(labels.len(), 75);
        for id in EndpointId::ALL {
            let label = id.as_str();
            let (group, verb) = label.split_once('.').expect("label has a dot");
            assert!(!verb.contains('.'), "{label}");
            assert!(is_snake_label(group) && is_snake_label(verb), "{label}");
        }
        assert_eq!(
            EndpointId::OrdersGetByCorrelation.as_str(),
            "orders.get_by_correlation"
        );
        assert_eq!(
            EndpointId::InstrumentsScripMasterCompact.as_str(),
            "instruments.scrip_master_compact"
        );
        assert_eq!(
            EndpointId::AuthGenerateConsent.as_str(),
            "auth.generate_consent"
        );
        assert_eq!(EndpointId::GlobalCancel.as_str(), "global.cancel");
        assert_eq!(EndpointId::ALL[0], EndpointId::AuthGenerateConsent);
        assert_eq!(EndpointId::ALL[74], EndpointId::GlobalCancel);
    }

    #[test]
    fn endpoint_labels_are_stable() {
        // Every endpoint label, in declaration order.
        let expected = [
            "auth.generate_consent",
            "auth.consume_consent",
            "auth.partner_generate_consent",
            "auth.partner_consume_consent",
            "auth.generate_access_token",
            "account.renew_token",
            "account.profile",
            "account.set_ip",
            "account.modify_ip",
            "account.get_ip",
            "orders.place",
            "orders.place_sliced",
            "orders.modify",
            "orders.cancel",
            "orders.list",
            "orders.get",
            "orders.get_by_correlation",
            "trades.list",
            "trades.for_order",
            "super_orders.place",
            "super_orders.modify",
            "super_orders.cancel_leg",
            "super_orders.list",
            "forever_orders.place",
            "forever_orders.modify",
            "forever_orders.cancel",
            "forever_orders.list",
            "conditional.place",
            "conditional.modify",
            "conditional.delete",
            "conditional.get",
            "conditional.list",
            "conditional.place_multi",
            "portfolio.holdings",
            "portfolio.positions",
            "portfolio.convert_position",
            "portfolio.exit_all",
            "funds.limits",
            "funds.margin",
            "funds.margin_multi",
            "statements.ledger",
            "statements.trade_history",
            "trader_control.set_kill_switch",
            "trader_control.kill_switch_status",
            "trader_control.set_pnl_exit",
            "trader_control.pnl_exit",
            "trader_control.stop_pnl_exit",
            "edis.generate_tpin",
            "edis.form",
            "edis.bulk_form",
            "edis.inquire",
            "market_quote.ltp",
            "market_quote.ohlc",
            "market_quote.quote",
            "historical.daily",
            "historical.intraday",
            "historical.rolling_options",
            "option_chain.chain",
            "option_chain.expiries",
            "instruments.scrip_master_compact",
            "instruments.scrip_master_detailed",
            "instruments.segment",
            "instruments.global_scrip_master",
            "global.market_status",
            "global.fund_limit",
            "global.holdings",
            "global.orders",
            "global.order",
            "global.trades",
            "global.trades_for_security",
            "global.margin",
            "global.estimate",
            "global.place",
            "global.modify",
            "global.cancel",
        ];
        let actual: Vec<&str> = EndpointId::ALL.iter().map(|e| e.as_str()).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn feed_kind_labels() {
        let all = [
            (FeedKind::Market, "market"),
            (FeedKind::Depth20, "depth20"),
            (FeedKind::Depth200, "depth200"),
            (FeedKind::OrderUpdate, "order_update"),
            (FeedKind::Global, "global"),
        ];
        for (kind, label) in all {
            assert_eq!(kind.as_str(), label);
        }
    }

    #[test]
    fn method_retry_and_rate_labels() {
        assert_eq!(
            [Method::Get, Method::Post, Method::Put, Method::Delete].map(Method::as_str),
            ["get", "post", "put", "delete"]
        );
        assert_eq!(
            [
                RetryClass::Read,
                RetryClass::Query,
                RetryClass::Mutation,
                RetryClass::Session
            ]
            .map(RetryClass::as_str),
            ["read", "query", "mutation", "session"]
        );
        assert_eq!(
            [
                RateClass::Order,
                RateClass::Data,
                RateClass::Quote,
                RateClass::NonTrading,
                RateClass::TokenGeneration,
                RateClass::Unmetered,
            ]
            .map(RateClass::as_str),
            [
                "order",
                "data",
                "quote",
                "non_trading",
                "token_generation",
                "unmetered"
            ]
        );
        assert!(RateClass::Order < RateClass::Unmetered);
        assert!(
            RateClass::Order < RateClass::Data && RateClass::TokenGeneration < RateClass::Unmetered
        );
    }
}
