//! Subscribes two instruments on the market feed, prints ten events and shuts down, against a
//! loopback WebSocket server.
//!
//! Run with `cargo run --example market_feed --features feed`. To use Dhan's feed, drop the
//! `.url(..)` override and pass your own credentials.

#[path = "support/mod.rs"]
mod support;

use std::time::Duration;

use dhani::feed::{FeedEvent, Instrument, MarketFeed, Mode};
use dhani::types::{ExchangeSegment, SecurityId};
use futures_util::StreamExt;
use support::ws::{Send, ticker};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The server waits for the subscription, then streams ticks for both instruments.
    let mut script = vec![Send::AwaitText];
    for i in 0..8u32 {
        script.push(Send::Pause(Duration::from_millis(50)));
        let id = if i % 2 == 0 { 1333 } else { 11536 };
        script.push(Send::Binary(ticker(
            id,
            1642.5 + i as f32,
            1_756_698_300 + i,
        )));
    }
    let (url, received) = support::ws::ws_server(script).await;

    let (handle, mut events, task) = MarketFeed::builder(support::credentials())
        .url(url)
        .spawn()?;
    let instruments = [
        Instrument::new(ExchangeSegment::NseEq, SecurityId::new("1333")?)?,
        Instrument::new(ExchangeSegment::NseEq, SecurityId::new("11536")?)?,
    ];
    let revision = handle.subscribe(instruments, Mode::Ticker).await?;
    println!("subscribed (revision {revision:?})");

    let mut printed = 0;
    while let Some(event) = events.next().await {
        match event? {
            FeedEvent::Data(d) => println!("data  seq {:>2}: {:?}", d.seq, d.value),
            FeedEvent::Lifecycle(l) => println!("life  {l:?}"),
            // Non-terminal: the packet is skipped and the feed carries on.
            FeedEvent::DecodeError(e) => println!("bad   {e:?}"),
            other => println!("other {other:?}"),
        }
        printed += 1;
        if printed == 10 {
            break;
        }
    }

    handle.shutdown().await?;
    println!("task ended: {:?}", task.join().await);
    println!("the server received: {:?}", received.lock().unwrap());
    Ok(())
}
