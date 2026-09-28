//! Orders facade contract tests (§9 rows O1–O9): each call matches the method, exact path,
//! credential headers and hand-written body, is served its upstream fixture and decodes to
//! hand-written values, including the Swagger sentinels (`"string"`, `-2147483648`,
//! `-3.402823669209385e+38`). Invalid requests are refused with zero requests sent.

mod support;

use dhani::error::{ValidationError, ValidationReason};
use dhani::rest::{ModifyOrderRequest, Order, PlaceOrderRequest, Trade};
use dhani::types::{
    AmoTime, CorrelationId, ExchangeSegment, Inbound, LegName, OptionType, OrderId, OrderStatus,
    OrderType, ProductType, SecurityId, TransactionType, Validity,
};
use dhani::{DhanClient, ErrorKind};
use serde_json::json;
use support::fixtures::upstream_payload;
use support::mock::{CLIENT_ID, body_json_eq, client_for, expect_headers};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture(name: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_bytes(upstream_payload(name))
}

/// A server with one mock: `verb path`, the credential headers, and `body` when given.
async fn serve(verb: &str, at: &str, body: Option<serde_json::Value>, reply: &str) -> MockServer {
    let server = MockServer::start().await;
    let mock = Mock::given(method(verb))
        .and(path(at))
        .and(expect_headers());
    let mock = match body {
        Some(expected) => mock
            .and(header("content-type", "application/json"))
            .and(body_json_eq(expected)),
        None => mock,
    };
    mock.respond_with(fixture(reply))
        .expect(1)
        .mount(&server)
        .await;
    server
}

/// The number of requests received; none of these endpoints takes a query.
async fn requests(server: &MockServer) -> usize {
    let received = server.received_requests().await.unwrap();
    assert!(received.iter().all(|r| r.url.query().is_none()));
    received.len()
}

fn limit_buy() -> PlaceOrderRequest {
    PlaceOrderRequest::new(
        ExchangeSegment::NseEq,
        SecurityId::new("1333").unwrap(),
        TransactionType::Buy,
        10,
        OrderType::Limit,
        ProductType::Cnc,
        Validity::Day,
    )
    .with_price(1642.5)
    .with_correlation_id(CorrelationId::new("run-7").unwrap())
}

fn limit_buy_body() -> serde_json::Value {
    json!({
        "dhanClientId": CLIENT_ID,
        "correlationId": "run-7",
        "transactionType": "BUY",
        "exchangeSegment": "NSE_EQ",
        "productType": "CNC",
        "orderType": "LIMIT",
        "validity": "DAY",
        "securityId": "1333",
        "quantity": 10,
        "price": 1642.5
    })
}

/// The minimum sentinels of the by-ID and by-correlation fixtures.
const MIN_QTY: i64 = -2_147_483_648;
const MIN_PRICE: f64 = -3.402823669209385e+38;

/// The values every order fixture carries; `qty` and `price` are the fixture's numeric
/// placeholders (the list fixture uses zeros, the lookups the minimum sentinels).
fn assert_sentinel_order(order: &Order, qty: i64, price: f64) {
    assert_eq!(order.order_id.as_ref(), "string");
    assert_eq!(order.exchange_order_id.as_deref(), Some("string"));
    assert_eq!(order.correlation_id.as_deref(), Some("string"));
    assert_eq!(
        order.order_status,
        Some(Inbound::Known(OrderStatus::Transit))
    );
    assert_eq!(
        order.transaction_type,
        Some(Inbound::Known(TransactionType::Buy))
    );
    assert_eq!(
        order.exchange_segment,
        Some(Inbound::Known(ExchangeSegment::NseEq))
    );
    assert_eq!(order.product_type, Some(Inbound::Known(ProductType::Cnc)));
    assert_eq!(order.order_type, Some(Inbound::Known(OrderType::Limit)));
    assert_eq!(order.validity, Some(Inbound::Known(Validity::Day)));
    assert_eq!(
        order.security_id.as_ref().map(|s| s.as_ref()),
        Some("string")
    );
    assert_eq!(order.quantity, Some(qty));
    assert_eq!(order.disclosed_quantity, Some(qty));
    assert_eq!(order.price, Some(price));
    assert_eq!(order.trigger_price, Some(price));
    assert_eq!(order.after_market_order, Some(true));
    assert_eq!(order.leg_name, Some(Inbound::Known(LegName::EntryLeg)));
    assert_eq!(
        order.create_time.as_ref().map(|t| t.as_str()),
        Some("string")
    );
    assert_eq!(
        order.drv_option_type,
        Some(Inbound::Known(OptionType::Call))
    );
    assert_eq!(order.average_traded_price, Some(price));
    assert_eq!(
        (order.remaining_quantity, order.filled_qty),
        (Some(qty), Some(qty))
    );
}

/// The values of both trade fixtures.
fn assert_sentinel_trade(trade: &Trade) {
    assert_eq!(trade.order_id.as_ref().map(|o| o.as_ref()), Some("string"));
    assert_eq!(trade.exchange_trade_id.as_deref(), Some("string"));
    assert_eq!(
        trade.transaction_type,
        Some(Inbound::Known(TransactionType::Buy))
    );
    assert_eq!(trade.product_type, Some(Inbound::Known(ProductType::Cnc)));
    assert_eq!(trade.custom_symbol.as_deref(), Some("string"));
    // Integer zeros in the fixture decode into the float and integer fields.
    assert_eq!(
        (trade.traded_quantity, trade.traded_price),
        (Some(0), Some(0.0))
    );
    assert_eq!(trade.drv_strike_price, Some(0.0));
    assert_eq!(
        trade.drv_option_type,
        Some(Inbound::Known(OptionType::Call))
    );
    assert_eq!(
        trade.exchange_time.as_ref().map(|t| t.as_str()),
        Some("string")
    );
}

// ---- O1–O9 contract tests ------------------------------------------------------------------

// row: O1
#[tokio::test]
async fn o1_place() {
    let server = serve(
        "POST",
        "/v2/orders",
        Some(limit_buy_body()),
        "place_order.json",
    )
    .await;
    let ack = client_for(&server)
        .orders()
        .place(&limit_buy())
        .await
        .unwrap();
    assert_eq!(ack.order_id.as_ref(), "string");
    assert_eq!(ack.order_status, Some(Inbound::Known(OrderStatus::Transit)));
    assert_eq!(requests(&server).await, 1);
}

// row: O2
#[tokio::test]
async fn o2_place_sliced_decodes_an_array() {
    let server = serve(
        "POST",
        "/v2/orders/slicing",
        Some(limit_buy_body()),
        "place_slice_order.json",
    )
    .await;
    let acks = client_for(&server)
        .orders()
        .place_sliced(&limit_buy())
        .await
        .unwrap();
    let decoded: Vec<(&str, Option<Inbound<OrderStatus>>)> = acks
        .iter()
        .map(|a| (a.order_id.as_ref(), a.order_status.clone()))
        .collect();
    let transit = Some(Inbound::Known(OrderStatus::Transit));
    // The fixture is a one-element array (DOC shows an object; Appendix A D30).
    assert_eq!(decoded, [("string", transit)]);
    assert_eq!(requests(&server).await, 1);
}

// row: O3
#[tokio::test]
async fn o3_modify_sends_only_the_set_fields() {
    let body = json!({
        "dhanClientId": CLIENT_ID,
        "orderId": "112111182198",
        "orderType": "LIMIT",
        "validity": "DAY",
        "price": 1.0
    });
    let server = serve(
        "PUT",
        "/v2/orders/112111182198",
        Some(body),
        "modify_pending_order.json",
    )
    .await;
    let req = ModifyOrderRequest::new(
        OrderId::new("112111182198").unwrap(),
        OrderType::Limit,
        Validity::Day,
    )
    .with_price(1.0);
    let ack = client_for(&server).orders().modify(&req).await.unwrap();
    assert_eq!(ack.order_id.as_ref(), "string");
    assert_eq!(ack.order_status, Some(Inbound::Known(OrderStatus::Transit)));
    assert_eq!(requests(&server).await, 1);
}

// row: O4
#[tokio::test]
async fn o4_cancel() {
    let server = serve(
        "DELETE",
        "/v2/orders/112111182198",
        None,
        "cancel_given_order.json",
    )
    .await;
    let order_id = OrderId::new("112111182198").unwrap();
    let ack = client_for(&server)
        .orders()
        .cancel(&order_id)
        .await
        .unwrap();
    assert_eq!(ack.order_id.as_ref(), "string");
    assert_eq!(ack.order_status, Some(Inbound::Known(OrderStatus::Transit)));
    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    // A bodyless call sends no body and no content type.
    assert!(received[0].body.is_empty());
    assert!(received[0].headers.get("content-type").is_none());
}

// row: O5
#[tokio::test]
async fn o5_list() {
    let server = serve("GET", "/v2/orders", None, "get-current-orders-list.json").await;
    let orders = client_for(&server).orders().list().await.unwrap();
    assert_eq!(orders.len(), 1);
    // The list fixture's numbers are integer zeros, which decode into i64 and f64 fields.
    assert_sentinel_order(&orders[0], 0, 0.0);
    assert_eq!(requests(&server).await, 1);
}

// row: O6
#[tokio::test]
async fn o6_get_decodes_the_sentinels() {
    let server = serve(
        "GET",
        "/v2/orders/112111182198",
        None,
        "get-order-by-id.json",
    )
    .await;
    let order = client_for(&server)
        .orders()
        .get(&OrderId::new("112111182198").unwrap())
        .await
        .unwrap();
    assert_sentinel_order(&order, MIN_QTY, MIN_PRICE);
    assert_eq!(order.quantity, Some(-2147483648));
    assert_eq!(order.price, Some(-3.402823669209385e+38));
    assert_eq!(
        order.create_time.as_ref().map(|t| t.as_str()),
        Some("string")
    );
    assert_eq!(requests(&server).await, 1);
}

// row: O7
#[tokio::test]
async fn o7_get_by_correlation_id() {
    let server = serve(
        "GET",
        "/v2/orders/external/run-7",
        None,
        "get-order-by-correlation-id.json",
    )
    .await;
    let order = client_for(&server)
        .orders()
        .get_by_correlation_id(&CorrelationId::new("run-7").unwrap())
        .await
        .unwrap();
    assert_sentinel_order(&order, MIN_QTY, MIN_PRICE);
    assert_eq!(requests(&server).await, 1);
}

// row: O8
#[tokio::test]
async fn o8_trades_has_no_trailing_slash() {
    // Exactly /v2/trades (Appendix A D32); PY requests /trades/.
    let server = serve("GET", "/v2/trades", None, "get_all_trades.json").await;
    let trades = client_for(&server).orders().trades().await.unwrap();
    assert_eq!(trades.len(), 1);
    assert_sentinel_trade(&trades[0]);
    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].url.path(), "/v2/trades");
}

// row: O9
#[tokio::test]
async fn o9_trades_for_order() {
    let server = serve(
        "GET",
        "/v2/trades/112111182198",
        None,
        "get_trade_book_by_orderid.json",
    )
    .await;
    let trades = client_for(&server)
        .orders()
        .trades_for_order(&OrderId::new("112111182198").unwrap())
        .await
        .unwrap();
    assert_eq!(trades.len(), 1);
    assert_sentinel_trade(&trades[0]);
    assert_eq!(requests(&server).await, 1);
}

// ---- Validation: refused locally, nothing sent ---------------------------------------------

/// Runs `call` against a server that would accept anything and returns the validation error;
/// asserts nothing reached the server.
async fn refused<F, Fut, T>(call: F) -> (&'static str, ValidationReason)
where
    F: FnOnce(DhanClient) -> Fut,
    Fut: std::future::Future<Output = dhani::Result<T>>,
    T: std::fmt::Debug,
{
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let err = call(client_for(&server)).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(server.received_requests().await.unwrap().is_empty());
    let v: &ValidationError = err.validation().expect("a validation error");
    (v.field, v.reason.clone())
}

fn invalid(field: &'static str, reason: ValidationReason) -> (&'static str, ValidationReason) {
    (field, reason)
}

#[tokio::test]
async fn a_limit_order_without_a_price_sends_nothing() {
    let mut req = limit_buy();
    req.price = None;
    let err = refused(|c| async move { c.orders().place(&req).await }).await;
    assert_eq!(err, invalid("price", ValidationReason::Missing));
}

#[tokio::test]
async fn a_stop_loss_without_a_trigger_sends_nothing() {
    let mut req = limit_buy();
    req.order_type = OrderType::StopLoss;
    let err = refused(|c| async move { c.orders().place(&req).await }).await;
    assert_eq!(err, invalid("trigger_price", ValidationReason::Missing));
}

#[tokio::test]
async fn an_amo_time_without_the_amo_flag_sends_nothing() {
    let mut req = limit_buy().with_amo(AmoTime::Open);
    req.after_market_order = None;
    let err = refused(|c| async move { c.orders().place(&req).await }).await;
    assert_eq!(
        err,
        invalid(
            "amo_time",
            ValidationReason::Inconsistent("requires after_market_order = true")
        )
    );
}

#[tokio::test]
async fn a_31_character_correlation_id_sends_nothing() {
    // Taken from a response, so it was never checked; the facade rechecks it.
    let long: CorrelationId = serde_json::from_value(json!("c".repeat(31))).unwrap();
    let req = limit_buy().with_correlation_id(long.clone());
    let err = refused(|c| async move { c.orders().place(&req).await }).await;
    assert_eq!(
        err,
        invalid("correlation_id", ValidationReason::TooLong { max: 30 })
    );
    let err = refused(|c| async move { c.orders().get_by_correlation_id(&long).await }).await;
    assert_eq!(
        err,
        invalid("correlation_id", ValidationReason::TooLong { max: 30 })
    );
}

#[tokio::test]
async fn a_cover_order_product_sends_nothing() {
    let mut req = limit_buy();
    req.product_type = ProductType::Co;
    let err = refused(|c| async move { c.orders().place(&req).await }).await;
    assert_eq!(
        err,
        invalid("product_type", ValidationReason::UnknownEnumValue)
    );
}

#[tokio::test]
async fn a_market_slice_without_a_price_sends_nothing() {
    let mut req = limit_buy();
    req.order_type = OrderType::Market;
    req.price = None;
    let err = refused(|c| async move { c.orders().place_sliced(&req).await }).await;
    assert_eq!(err, invalid("price", ValidationReason::Missing));
}

#[tokio::test]
async fn an_invalid_order_id_from_a_response_is_rechecked_before_reuse() {
    // A placeholder that decodes leniently but breaks the OrderId charset. (The fixtures'
    // "string" happens to satisfy it, so it cannot serve here.)
    let placeholder: OrderId = serde_json::from_value(json!("string!")).unwrap();
    let expected = invalid("order_id", ValidationReason::InvalidCharacters);
    let id = placeholder.clone();
    assert_eq!(
        refused(|c| async move { c.orders().cancel(&id).await }).await,
        expected
    );
    let id = placeholder.clone();
    assert_eq!(
        refused(|c| async move { c.orders().get(&id).await }).await,
        expected
    );
    let id = placeholder.clone();
    assert_eq!(
        refused(|c| async move { c.orders().trades_for_order(&id).await }).await,
        expected
    );
    let req = ModifyOrderRequest::new(placeholder, OrderType::Market, Validity::Day);
    assert_eq!(
        refused(|c| async move { c.orders().modify(&req).await }).await,
        expected
    );
}
