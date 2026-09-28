//! Receives live order updates, against a loopback WebSocket server: the feed logs in, the
//! server sends one order alert, and the example prints it and shuts down.
//!
//! Run with `cargo run --example order_updates --features feed`. Against Dhan: drop the
//! `.url(..)` override and pass your own credentials.

#[path = "support/mod.rs"]
mod support;

use dhani::feed::{FeedEvent, OrderUpdateEvent, OrderUpdateFeed};
use futures_util::StreamExt;
use support::ws::Send;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let alert = serde_json::json!({
        "Type": "order_alert",
        "Data": {
            "OrderNo": "1124091136546",
            "Status": "Cancelled",
            "Symbol": "IDEA",
            "Quantity": 1,
            "Price": 13
        }
    });
    let (url, received) =
        support::ws::ws_server(vec![Send::AwaitText, Send::Text(alert.to_string())]).await;

    let (handle, mut events, task) = OrderUpdateFeed::builder(support::credentials())
        .url(url)
        .spawn()?;
    while let Some(event) = events.next().await {
        match event? {
            FeedEvent::Data(d) => match d.value {
                OrderUpdateEvent::Order(order) => {
                    println!(
                        "order {} is {:?}",
                        order.order_no.as_deref().unwrap_or("?"),
                        order.status
                    );
                    break;
                }
                other => println!("other message: {other:?}"),
            },
            FeedEvent::Lifecycle(l) => println!("life  {l:?}"),
            other => println!("other {other:?}"),
        }
    }
    handle.shutdown().await?;
    println!("task ended: {:?}", task.join().await);
    // The login message carries the token; print only that one arrived.
    println!(
        "login messages received: {}",
        received.lock().unwrap().len()
    );
    Ok(())
}
