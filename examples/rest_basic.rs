//! Places an order and lists the order book, against a local mock server.
//!
//! Run with `cargo run --example rest_basic --features rest`. Nothing leaves the machine; to
//! talk to Dhan, drop the `.urls(..)` and `.http_client(..)` lines and pass your own
//! credentials.

#[path = "support/mod.rs"]
mod support;

use dhani::DhanClient;
use dhani::rest::PlaceOrderRequest;
use dhani::types::{
    ExchangeSegment, OrderType, ProductType, SecurityId, TransactionType, Validity,
};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::main]
async fn main() -> dhani::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/orders"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"orderId": "112111182198", "orderStatus": "PENDING"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/orders"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "orderId": "112111182198",
            "orderStatus": "PENDING",
            "transactionType": "BUY",
            "tradingSymbol": "HDFCBANK",
            "securityId": "1333",
            "quantity": 10,
            "price": 1642.5
        }])))
        .mount(&server)
        .await;

    let client = DhanClient::builder()
        .urls(support::urls_for(&server))
        .http_client(support::local_http_client())
        .credentials(support::credentials())
        .build()?;

    let order = PlaceOrderRequest::new(
        ExchangeSegment::NseEq,
        SecurityId::new("1333")?,
        TransactionType::Buy,
        10,
        OrderType::Limit,
        ProductType::Cnc,
        Validity::Day,
    )
    .with_price(1642.5);
    let ack = client.orders().place(&order).await?;
    println!("placed order {} ({:?})", ack.order_id, ack.order_status);

    for o in client.orders().list().await? {
        println!(
            "{} {:?} {} x{} @ {}",
            o.order_id,
            o.order_status,
            o.trading_symbol.as_deref().unwrap_or("?"),
            o.quantity.unwrap_or_default(),
            o.price.unwrap_or_default()
        );
    }
    Ok(())
}
