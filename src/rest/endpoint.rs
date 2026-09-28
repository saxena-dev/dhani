//! `Endpoint`, `Host`, `AuthMode`, `BodyPolicy`, `ResponseShape` and the `const` endpoint
//! descriptors.
//!
//! One descriptor per REST row of the endpoint matrix. Method and path are source facts (each
//! descriptor's `doc` cites where); the rate class of non-order endpoints is local policy
//! (`NonTrading`, OQ-3). Path templates use `{name}` placeholders filled positionally by the
//! transport; query parameters are not part of the path. Scrip-master CSV rows have an empty
//! path: their absolute URL comes from the matching `Urls` field.
#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the transport and facades that read these descriptors land later"
    )
)]

use crate::labels::{EndpointId, Method, RateClass, RetryClass};

/// The base URL an endpoint is resolved against.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Host {
    /// `Urls::rest`.
    Rest,
    /// `Urls::auth`.
    Auth,
    /// The endpoint's own scrip-master CSV URL in `Urls`.
    ScripMaster,
}

/// How a request is authenticated.
#[derive(Clone, Copy, Debug)]
pub(crate) enum AuthMode {
    /// `access-token` and `client-id` headers from the client's credentials.
    AccessToken,
    /// As `AccessToken`, plus a `dhanClientId` header (RenewToken, profile).
    AccessTokenAndClientIdHeader,
    /// `app_id` and `app_secret` headers supplied by the call.
    AppCredentials,
    /// `partner_id` and `partner_secret` headers supplied by the call.
    PartnerCredentials,
    /// No credential headers (credentials, if any, travel in the query).
    None,
}

/// What the request body carries.
#[derive(Clone, Copy, Debug)]
pub(crate) enum BodyPolicy {
    /// No body.
    None,
    /// The JSON body as built by the facade.
    #[allow(
        dead_code,
        reason = "every documented JSON body on the REST host carries the client id (Appendix A D44)"
    )]
    Json,
    /// The JSON body with `"dhanClientId"` inserted at the top level (Appendix A D44).
    JsonWithClientId,
}

/// The success body the endpoint returns.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ResponseShape {
    /// A JSON document decoded into the response type.
    Json,
    /// No meaningful body (empty, whitespace or any JSON value is accepted).
    Empty,
    /// A JSON document, or nothing (empty body, `{}` or `null`).
    JsonOrEmpty,
    /// UTF-8 CSV text.
    Csv,
}

/// A REST endpoint descriptor.
#[derive(Debug)]
pub(crate) struct Endpoint {
    pub id: EndpointId,
    pub method: Method,
    pub host: Host,
    /// Template such as `/orders/{order_id}`; placeholders are filled positionally.
    pub path: &'static str,
    pub retry: RetryClass,
    pub rate: RateClass,
    /// Additionally takes the per-key 1-per-3-seconds option-chain window.
    pub keyed_option_chain: bool,
    /// Counts toward the 25-modifications-per-order cap.
    pub modification_cap: bool,
    pub auth: AuthMode,
    pub body: BodyPolicy,
    pub response: ResponseShape,
    /// Enabled in the sandbox (DOC:3872-3895).
    pub sandbox: bool,
    /// Source citation of the endpoint's method and path.
    pub doc: &'static str,
}

/// Descriptor flags, listed in brackets in the table below.
#[derive(Clone, Copy)]
enum Flag {
    /// Enabled in the sandbox.
    Sandbox,
    /// Takes the keyed option-chain window.
    Keyed,
    /// Counts toward the modification cap.
    Cap,
}

const fn has(flags: &[Flag], flag: Flag) -> bool {
    let mut i = 0;
    while i < flags.len() {
        if flags[i] as u8 == flag as u8 {
            return true;
        }
        i += 1;
    }
    false
}

/// Declares one `pub(crate) const` descriptor per table row, and `ALL` in table order. Columns:
/// id, method, host, path, retry, rate, auth, body, response, flags, doc.
macro_rules! endpoints {
    ($(
        $name:ident = $id:ident, $method:ident, $host:ident, $path:literal, $retry:ident, $rate:ident,
        $auth:ident, $body:ident, $response:ident, [$($flag:ident)*], doc: $doc:literal;
    )+) => {
        $(
            pub(crate) const $name: Endpoint = Endpoint {
                id: EndpointId::$id,
                method: Method::$method,
                host: Host::$host,
                path: $path,
                retry: RetryClass::$retry,
                rate: RateClass::$rate,
                keyed_option_chain: has(&[$(Flag::$flag),*], Flag::Keyed),
                modification_cap: has(&[$(Flag::$flag),*], Flag::Cap),
                auth: AuthMode::$auth,
                body: BodyPolicy::$body,
                response: ResponseShape::$response,
                sandbox: has(&[$(Flag::$flag),*], Flag::Sandbox),
                doc: $doc,
            };
        )+

        /// Every descriptor, in endpoint-matrix order (unit-test support).
        #[cfg(test)]
        pub(crate) static ALL: &[&Endpoint] = &[$(&$name),+];
    };
}

// Row ids refer to the endpoint matrix. Scrip-master rows have an empty path.
endpoints! {
    // A1
    AUTH_GENERATE_CONSENT = AuthGenerateConsent, Post, Auth, "/app/generate-consent", Session, Unmetered, AppCredentials, None, Json, [], doc: "DOC:4367-4412";
    // A2
    AUTH_CONSUME_CONSENT = AuthConsumeConsent, Get, Auth, "/app/consumeApp-consent", Session, Unmetered, AppCredentials, None, Json, [], doc: "DOC:4321-4366";
    // A3
    AUTH_PARTNER_GENERATE_CONSENT = AuthPartnerGenerateConsent, Post, Auth, "/partner/generate-consent", Session, Unmetered, PartnerCredentials, None, Json, [], doc: "DOC:4569-4603";
    // A4
    AUTH_PARTNER_CONSUME_CONSENT = AuthPartnerConsumeConsent, Get, Auth, "/partner/consume-consent", Session, Unmetered, PartnerCredentials, None, Json, [], doc: "DOC:4523-4568";
    // A5
    AUTH_GENERATE_ACCESS_TOKEN = AuthGenerateAccessToken, Post, Auth, "/app/generateAccessToken", Session, TokenGeneration, None, None, Json, [], doc: "DOC:4413-4451";
    // A6
    ACCOUNT_RENEW_TOKEN = AccountRenewToken, Get, Rest, "/RenewToken", Session, NonTrading, AccessTokenAndClientIdHeader, None, Json, [], doc: "DOC:4604-4643";
    // A7
    ACCOUNT_PROFILE = AccountProfile, Get, Rest, "/profile", Read, NonTrading, AccessTokenAndClientIdHeader, None, Json, [], doc: "DOC:4870-4879";
    // A8
    ACCOUNT_SET_IP = AccountSetIp, Post, Rest, "/ip/setIP", Mutation, NonTrading, AccessToken, JsonWithClientId, Json, [], doc: "DOC:4644-4685";
    // A9
    ACCOUNT_MODIFY_IP = AccountModifyIp, Put, Rest, "/ip/modifyIP", Mutation, NonTrading, AccessToken, JsonWithClientId, Json, [], doc: "DOC:4481-4522";
    // A10
    ACCOUNT_GET_IP = AccountGetIp, Get, Rest, "/ip/getIP", Read, NonTrading, AccessToken, None, Json, [], doc: "DOC:4452-4480";
    // O1
    ORDERS_PLACE = OrdersPlace, Post, Rest, "/orders", Mutation, Order, AccessToken, JsonWithClientId, Json, [Sandbox], doc: "DOC:3712-3770";
    // O2
    ORDERS_PLACE_SLICED = OrdersPlaceSliced, Post, Rest, "/orders/slicing", Mutation, Order, AccessToken, JsonWithClientId, Json, [Sandbox], doc: "DOC:3898-3954";
    // O3
    ORDERS_MODIFY = OrdersModify, Put, Rest, "/orders/{order_id}", Mutation, Order, AccessToken, JsonWithClientId, Json, [Sandbox Cap], doc: "DOC:3109-3166";
    // O4
    ORDERS_CANCEL = OrdersCancel, Delete, Rest, "/orders/{order_id}", Mutation, Order, AccessToken, None, Json, [Sandbox], doc: "DOC:131-170";
    // O5
    ORDERS_LIST = OrdersList, Get, Rest, "/orders", Read, NonTrading, AccessToken, None, Json, [Sandbox], doc: "DOC:1375-1437";
    // O6
    ORDERS_GET = OrdersGet, Get, Rest, "/orders/{order_id}", Read, NonTrading, AccessToken, None, Json, [Sandbox], doc: "DOC:1305-1374";
    // O7
    ORDERS_GET_BY_CORRELATION = OrdersGetByCorrelation, Get, Rest, "/orders/external/{correlation_id}", Read, NonTrading, AccessToken, None, Json, [Sandbox], doc: "DOC:1236-1304";
    // O8
    TRADES_LIST = TradesList, Get, Rest, "/trades", Read, NonTrading, AccessToken, None, Json, [Sandbox], doc: "DOC:1751-1800";
    // O9
    TRADES_FOR_ORDER = TradesForOrder, Get, Rest, "/trades/{order_id}", Read, NonTrading, AccessToken, None, Json, [Sandbox], doc: "DOC:1695-1750";
    // S1
    SUPER_ORDERS_PLACE = SuperOrdersPlace, Post, Rest, "/super/orders", Mutation, Order, AccessToken, JsonWithClientId, Json, [], doc: "DOC:3771-3826";
    // S2
    SUPER_ORDERS_MODIFY = SuperOrdersModify, Put, Rest, "/super/orders/{order_id}", Mutation, Order, AccessToken, JsonWithClientId, Json, [], doc: "DOC:3167-3226";
    // S3
    SUPER_ORDERS_CANCEL_LEG = SuperOrdersCancelLeg, Delete, Rest, "/super/orders/{order_id}/{order_leg}", Mutation, Order, AccessToken, None, JsonOrEmpty, [], doc: "DOC:171-213";
    // S4
    SUPER_ORDERS_LIST = SuperOrdersList, Get, Rest, "/super/orders", Read, NonTrading, AccessToken, None, Json, [], doc: "DOC:1570-1627";
    // F1
    FOREVER_ORDERS_PLACE = ForeverOrdersPlace, Post, Rest, "/forever/orders", Mutation, Order, AccessToken, JsonWithClientId, Json, [], doc: "LEGACY:forever";
    // F2
    FOREVER_ORDERS_MODIFY = ForeverOrdersModify, Put, Rest, "/forever/orders/{order_id}", Mutation, Order, AccessToken, JsonWithClientId, Json, [], doc: "LEGACY:forever";
    // F3
    FOREVER_ORDERS_CANCEL = ForeverOrdersCancel, Delete, Rest, "/forever/orders/{order_id}", Mutation, Order, AccessToken, None, Json, [], doc: "LEGACY:forever";
    // F4
    FOREVER_ORDERS_LIST = ForeverOrdersList, Get, Rest, "/forever/orders", Read, NonTrading, AccessToken, None, Json, [], doc: "LEGACY:forever";
    // C1
    CONDITIONAL_PLACE = ConditionalPlace, Post, Rest, "/alerts/orders", Mutation, Order, AccessToken, JsonWithClientId, Json, [], doc: "DOC:3262-3463";
    // C2
    CONDITIONAL_MODIFY = ConditionalModify, Put, Rest, "/alerts/orders/{alert_id}", Mutation, Order, AccessToken, JsonWithClientId, Json, [], doc: "DOC:2894-3108";
    // C3
    CONDITIONAL_DELETE = ConditionalDelete, Delete, Rest, "/alerts/orders/{alert_id}", Mutation, Order, AccessToken, None, Json, [], doc: "DOC:369-409";
    // C4
    CONDITIONAL_GET = ConditionalGet, Get, Rest, "/alerts/orders/{alert_id}", Read, NonTrading, AccessToken, None, Json, [], doc: "DOC:709-741";
    // C5
    CONDITIONAL_LIST = ConditionalList, Get, Rest, "/alerts/orders", Read, NonTrading, AccessToken, None, Json, [], doc: "DOC:683-708";
    // C6
    CONDITIONAL_PLACE_MULTI = ConditionalPlaceMulti, Post, Rest, "/alerts/multi/orders", Mutation, Order, AccessToken, JsonWithClientId, Json, [], doc: "DOC:3464-3711";
    // P1
    PORTFOLIO_HOLDINGS = PortfolioHoldings, Get, Rest, "/holdings", Read, NonTrading, AccessToken, None, Json, [Sandbox], doc: "DOC:938-981";
    // P2
    PORTFOLIO_POSITIONS = PortfolioPositions, Get, Rest, "/positions", Read, NonTrading, AccessToken, None, Json, [Sandbox], doc: "DOC:1470-1528";
    // P3
    PORTFOLIO_CONVERT_POSITION = PortfolioConvertPosition, Post, Rest, "/positions/convert", Mutation, NonTrading, AccessToken, JsonWithClientId, Empty, [Sandbox], doc: "DOC:281-325";
    // P4
    PORTFOLIO_EXIT_ALL = PortfolioExitAll, Delete, Rest, "/positions", Mutation, NonTrading, AccessToken, None, Empty, [], doc: "DOC:548-578";
    // M1
    FUNDS_LIMITS = FundsLimits, Get, Rest, "/fundlimit", Read, NonTrading, AccessToken, None, Json, [Sandbox], doc: "DOC:899-937";
    // M2
    FUNDS_MARGIN = FundsMargin, Post, Rest, "/margincalculator", Query, NonTrading, AccessToken, JsonWithClientId, Json, [Sandbox], doc: "DOC:10-72";
    // M3
    FUNDS_MARGIN_MULTI = FundsMarginMulti, Post, Rest, "/margincalculator/multi", Query, NonTrading, AccessToken, JsonWithClientId, Json, [], doc: "DOC:73-130";
    // T1
    STATEMENTS_LEDGER = StatementsLedger, Get, Rest, "/ledger", Read, NonTrading, AccessToken, None, Json, [Sandbox], doc: "DOC:1068-1116";
    // T2
    STATEMENTS_TRADE_HISTORY = StatementsTradeHistory, Get, Rest, "/trades/{from_date}/{to_date}/{page_number}", Read, NonTrading, AccessToken, None, Json, [Sandbox], doc: "DOC:1628-1694";
    // K1
    TRADER_CONTROL_SET_KILL_SWITCH = TraderControlSetKillSwitch, Post, Rest, "/killswitch", Mutation, NonTrading, AccessToken, None, Json, [Sandbox], doc: "DOC:2832-2877";
    // K2
    TRADER_CONTROL_KILL_SWITCH_STATUS = TraderControlKillSwitchStatus, Get, Rest, "/killswitch", Read, NonTrading, AccessToken, None, Json, [], doc: "DOC:1034-1067";
    // K3
    TRADER_CONTROL_SET_PNL_EXIT = TraderControlSetPnlExit, Post, Rest, "/pnlExit", Mutation, NonTrading, AccessToken, JsonWithClientId, Json, [], doc: "DOC:234-280";
    // K4
    TRADER_CONTROL_PNL_EXIT = TraderControlPnlExit, Get, Rest, "/pnlExit", Read, NonTrading, AccessToken, None, Json, [], doc: "DOC:1438-1469";
    // K5
    TRADER_CONTROL_STOP_PNL_EXIT = TraderControlStopPnlExit, Delete, Rest, "/pnlExit", Mutation, NonTrading, AccessToken, None, Json, [], doc: "DOC:3968-3999";
    // E1: a GET with a side effect (it issues a T-PIN), kept a Mutation so it gets exactly one
    // attempt, as the endpoint matrix marks it; JsonOrEmpty accepts both the documented empty
    // 202 body and a JSON ack.
    EDIS_GENERATE_TPIN = EdisGenerateTpin, Get, Rest, "/edis/tpin", Mutation, NonTrading, AccessToken, None, JsonOrEmpty, [Sandbox], doc: "DOC:648-682";
    // E2
    EDIS_FORM = EdisForm, Post, Rest, "/edis/form", Mutation, NonTrading, AccessToken, JsonWithClientId, Json, [], doc: "DOC:5145-5179";
    // E3
    EDIS_BULK_FORM = EdisBulkForm, Post, Rest, "/edis/bulkform", Mutation, NonTrading, AccessToken, JsonWithClientId, Json, [Sandbox], doc: "DOC:603-647";
    // E4
    EDIS_INQUIRE = EdisInquire, Get, Rest, "/edis/inquire/{isin}", Read, NonTrading, AccessToken, None, Json, [Sandbox], doc: "DOC:489-535";
    // Q1
    MARKET_QUOTE_LTP = MarketQuoteLtp, Post, Rest, "/marketfeed/ltp", Query, Quote, AccessToken, JsonWithClientId, Json, [], doc: "DOC:1117-1158";
    // Q2
    MARKET_QUOTE_OHLC = MarketQuoteOhlc, Post, Rest, "/marketfeed/ohlc", Query, Quote, AccessToken, JsonWithClientId, Json, [], doc: "DOC:1159-1199";
    // Q3
    MARKET_QUOTE_QUOTE = MarketQuoteQuote, Post, Rest, "/marketfeed/quote", Query, Quote, AccessToken, JsonWithClientId, Json, [], doc: "DOC:1529-1569";
    // H1
    HISTORICAL_DAILY = HistoricalDaily, Post, Rest, "/charts/historical", Query, Data, AccessToken, JsonWithClientId, Json, [Sandbox], doc: "DOC:742-793";
    // H2
    HISTORICAL_INTRADAY = HistoricalIntraday, Post, Rest, "/charts/intraday", Query, Data, AccessToken, JsonWithClientId, Json, [Sandbox], doc: "DOC:982-1033";
    // H3
    HISTORICAL_ROLLING_OPTIONS = HistoricalRollingOptions, Post, Rest, "/charts/rollingoption", Query, Data, AccessToken, JsonWithClientId, Json, [], doc: "DOC:794-856";
    // X1
    OPTION_CHAIN_CHAIN = OptionChainChain, Post, Rest, "/optionchain", Query, Data, AccessToken, JsonWithClientId, Json, [Keyed], doc: "DOC:1200-1234";
    // X2
    OPTION_CHAIN_EXPIRIES = OptionChainExpiries, Post, Rest, "/optionchain/expirylist", Query, Data, AccessToken, JsonWithClientId, Json, [], doc: "DOC:857-897";
    // I1
    INSTRUMENTS_SCRIP_MASTER_COMPACT = InstrumentsScripMasterCompact, Get, ScripMaster, "", Read, Unmetered, None, None, Csv, [], doc: "DOC:5729";
    // I2
    INSTRUMENTS_SCRIP_MASTER_DETAILED = InstrumentsScripMasterDetailed, Get, ScripMaster, "", Read, Unmetered, None, None, Csv, [], doc: "DOC:5735";
    // I3
    INSTRUMENTS_SEGMENT = InstrumentsSegment, Get, Rest, "/instrument/{exchange_segment}", Read, NonTrading, AccessToken, None, Csv, [], doc: "DOC:5742-5751";
    // I4
    INSTRUMENTS_GLOBAL_SCRIP_MASTER = InstrumentsGlobalScripMaster, Get, ScripMaster, "", Read, Unmetered, None, None, Csv, [], doc: "DOC:5806";
    // G1
    GLOBAL_MARKET_STATUS = GlobalMarketStatus, Get, Rest, "/globalstocks/marketstatus", Read, NonTrading, AccessToken, None, Json, [], doc: "DOC:2274-2308";
    // G2
    GLOBAL_FUND_LIMIT = GlobalFundLimit, Get, Rest, "/globalstocks/fundlimit", Read, NonTrading, AccessToken, None, Json, [], doc: "DOC:2191-2228";
    // G3
    GLOBAL_HOLDINGS = GlobalHoldings, Get, Rest, "/globalstocks/holdings", Read, NonTrading, AccessToken, None, Json, [], doc: "DOC:2229-2273";
    // G4
    GLOBAL_ORDERS = GlobalOrders, Get, Rest, "/globalstocks/orders", Read, NonTrading, AccessToken, None, Json, [], doc: "DOC:2309-2369";
    // G5
    GLOBAL_ORDER = GlobalOrder, Get, Rest, "/globalstocks/orders/{order_id}", Read, NonTrading, AccessToken, None, Json, [], doc: "DOC:2370-2437";
    // G6
    GLOBAL_TRADES = GlobalTrades, Get, Rest, "/globalstocks/trades", Read, NonTrading, AccessToken, None, Json, [], doc: "DOC:2494-2542";
    // G7
    GLOBAL_TRADES_FOR_SECURITY = GlobalTradesForSecurity, Get, Rest, "/globalstocks/trades/{security_id}", Read, NonTrading, AccessToken, None, Json, [], doc: "DOC:2438-2493";
    // G8
    GLOBAL_MARGIN = GlobalMargin, Post, Rest, "/globalstocks/margincalculator", Query, NonTrading, AccessToken, JsonWithClientId, Json, [], doc: "DOC:2543-2595";
    // G9
    GLOBAL_ESTIMATE = GlobalEstimate, Post, Rest, "/globalstocks/transEstimate", Query, NonTrading, AccessToken, JsonWithClientId, Json, [], doc: "DOC:2657-2710";
    // G10
    GLOBAL_PLACE = GlobalPlace, Post, Rest, "/globalstocks/orders", Mutation, Order, AccessToken, JsonWithClientId, Json, [], doc: "DOC:2727-2785";
    // G11
    GLOBAL_MODIFY = GlobalModify, Put, Rest, "/globalstocks/orders/{order_id}", Mutation, Order, AccessToken, JsonWithClientId, Json, [], doc: "DOC:2596-2656";
    // G12
    GLOBAL_CANCEL = GlobalCancel, Delete, Rest, "/globalstocks/orders/{order_id}", Mutation, Order, AccessToken, None, Json, [], doc: "DOC:2127-2166";
}

/// The descriptor for `id` (unit-test support).
#[cfg(test)]
pub(crate) fn by_id(id: EndpointId) -> &'static Endpoint {
    ALL.iter()
        .copied()
        .find(|e| e.id == id)
        .expect("every EndpointId has a descriptor (checked by the unit tests)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn rows(ids: &[EndpointId]) -> HashSet<EndpointId> {
        ids.iter().copied().collect()
    }

    #[test]
    fn one_descriptor_per_endpoint_id() {
        assert_eq!(ALL.len(), 75);
        let ids: Vec<EndpointId> = ALL.iter().map(|e| e.id).collect();
        assert_eq!(rows(&ids).len(), 75, "ids are unique");
        assert_eq!(rows(&ids), rows(EndpointId::ALL));
        for id in EndpointId::ALL {
            assert_eq!(by_id(*id).id, *id);
        }
    }

    #[test]
    fn mutations_and_methods_agree() {
        for e in ALL {
            if matches!(e.retry, RetryClass::Mutation) && e.method == Method::Get {
                // The single documented exception: generating a T-PIN is a GET with a side effect,
                // so it gets exactly one attempt.
                assert_eq!(
                    e.id,
                    EndpointId::EdisGenerateTpin,
                    "{:?} is a GET mutation",
                    e.id
                );
            }
        }
        for e in ALL {
            if e.method != Method::Get && e.host == Host::Rest {
                assert!(
                    matches!(
                        e.retry,
                        RetryClass::Mutation | RetryClass::Query | RetryClass::Session
                    ),
                    "{:?}",
                    e.id
                );
            }
        }
        assert!(matches!(
            by_id(EndpointId::EdisGenerateTpin).retry,
            RetryClass::Mutation
        ));
    }

    #[test]
    fn table_wide_method_invariants() {
        for e in ALL {
            if e.method == Method::Get && e.id != EndpointId::EdisGenerateTpin {
                assert!(
                    matches!(e.retry, RetryClass::Read | RetryClass::Session),
                    "{:?}",
                    e.id
                );
            }
            if e.rate == RateClass::Order {
                assert_ne!(e.method, Method::Get, "{:?}", e.id);
                assert!(matches!(e.retry, RetryClass::Mutation), "{:?}", e.id);
            }
            if matches!(e.response, ResponseShape::Csv) {
                assert_eq!(e.method, Method::Get, "{:?}", e.id);
            }
        }
    }

    #[test]
    fn doc_strings_are_single_citations() {
        for e in ALL {
            let doc = e.doc;
            let ok = if let Some(rest) = doc.strip_prefix("DOC:") {
                let mut parts = rest.splitn(2, '-');
                let a = parts.next().unwrap_or("");
                let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
                digits(a) && parts.next().is_none_or(digits)
            } else {
                doc == "LEGACY:forever"
            };
            assert!(ok, "{:?}: {doc}", e.id);
        }
    }

    #[test]
    fn sandbox_rows_match_the_documented_list() {
        use EndpointId::*;
        let expected = rows(&[
            OrdersPlace,
            OrdersPlaceSliced,
            OrdersModify,
            OrdersCancel,
            OrdersList,
            OrdersGet,
            OrdersGetByCorrelation,
            TradesList,
            TradesForOrder,
            PortfolioHoldings,
            PortfolioPositions,
            PortfolioConvertPosition,
            FundsLimits,
            FundsMargin,
            StatementsLedger,
            StatementsTradeHistory,
            TraderControlSetKillSwitch,
            EdisGenerateTpin,
            EdisBulkForm,
            EdisInquire,
            HistoricalDaily,
            HistoricalIntraday,
        ]);
        let actual: HashSet<EndpointId> = ALL.iter().filter(|e| e.sandbox).map(|e| e.id).collect();
        assert_eq!(actual.len(), 22);
        assert_eq!(actual, expected);
    }

    #[test]
    fn keyed_window_and_modification_cap_rows() {
        let keyed: Vec<EndpointId> = ALL
            .iter()
            .filter(|e| e.keyed_option_chain)
            .map(|e| e.id)
            .collect();
        assert_eq!(keyed, [EndpointId::OptionChainChain]);
        let capped: Vec<EndpointId> = ALL
            .iter()
            .filter(|e| e.modification_cap)
            .map(|e| e.id)
            .collect();
        assert_eq!(capped, [EndpointId::OrdersModify]);
    }

    #[test]
    fn selected_rows_match_the_matrix() {
        let place = by_id(EndpointId::OrdersPlace);
        assert_eq!((place.method, place.path), (Method::Post, "/orders"));
        assert!(matches!(place.retry, RetryClass::Mutation) && place.rate == RateClass::Order);
        assert!(matches!(place.body, BodyPolicy::JsonWithClientId));
        let leg = by_id(EndpointId::SuperOrdersCancelLeg);
        assert_eq!(
            (leg.method, leg.path),
            (Method::Delete, "/super/orders/{order_id}/{order_leg}")
        );
        assert!(matches!(leg.response, ResponseShape::JsonOrEmpty));
        assert!(matches!(
            by_id(EndpointId::PortfolioConvertPosition).response,
            ResponseShape::Empty
        ));
        assert!(matches!(
            by_id(EndpointId::PortfolioExitAll).response,
            ResponseShape::Empty
        ));
        let token = by_id(EndpointId::AuthGenerateAccessToken);
        assert_eq!(
            (token.host, token.rate),
            (Host::Auth, RateClass::TokenGeneration)
        );
        assert!(matches!(token.auth, AuthMode::None) && matches!(token.retry, RetryClass::Session));
        assert_eq!(by_id(EndpointId::AuthConsumeConsent).method, Method::Get);
        assert_eq!(
            by_id(EndpointId::AuthPartnerConsumeConsent).method,
            Method::Get
        );
        assert_eq!(
            by_id(EndpointId::AuthPartnerGenerateConsent).method,
            Method::Post
        );
        assert!(matches!(
            by_id(EndpointId::AccountRenewToken).auth,
            AuthMode::AccessTokenAndClientIdHeader
        ));
        assert!(matches!(
            by_id(EndpointId::TraderControlSetKillSwitch).body,
            BodyPolicy::None
        ));
        assert_eq!(by_id(EndpointId::StatementsLedger).path, "/ledger");
        let csv = by_id(EndpointId::InstrumentsScripMasterCompact);
        assert_eq!((csv.host, csv.path), (Host::ScripMaster, ""));
        assert!(matches!(csv.response, ResponseShape::Csv) && csv.rate == RateClass::Unmetered);
        assert_eq!(by_id(EndpointId::OrdersList).rate, RateClass::NonTrading);
        assert_eq!(by_id(EndpointId::MarketQuoteLtp).rate, RateClass::Quote);
        assert_eq!(by_id(EndpointId::HistoricalDaily).rate, RateClass::Data);
        assert!(!by_id(EndpointId::ConditionalPlaceMulti).sandbox);
        for e in ALL {
            // Every JSON body on the REST host carries the client id (Appendix A D44).
            if e.host != Host::Rest {
                assert!(matches!(e.body, BodyPolicy::None), "{:?}", e.id);
            }
            assert!(!matches!(e.body, BodyPolicy::Json), "{:?}", e.id);
        }
    }
}
