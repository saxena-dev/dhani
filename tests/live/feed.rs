//! The live feed captures (`live_feed_*`): a minute of each feed, raw frames written to
//! `target/captures/ws/<feed>-<date>.bin`.

use std::io::Write;
use std::time::{Duration, UNIX_EPOCH};

use dhani::feed::{FeedEvent, FeedEvents, Instrument, MarketFeed, Mode, OrderUpdateFeed, RawFrame};
use dhani::types::{ExchangeSegment, SecurityId};
use futures_util::StreamExt;

use crate::lane::{capture_dir, live_credentials, serial, today};

/// How long each capture runs.
const CAPTURE: Duration = Duration::from_secs(60);

/// Appends one container record for `frame`.
fn write_record(out: &mut impl Write, frame: &RawFrame) -> std::io::Result<()> {
    let nanos = frame
        .received_at
        .duration_since(UNIX_EPOCH)
        .expect("a time after the epoch")
        .as_nanos();
    let nanos = u64::try_from(nanos).expect("nanoseconds fit in u64");
    let len = u32::try_from(frame.bytes.len()).expect("a frame under 4 GiB");
    out.write_all(&nanos.to_le_bytes())?;
    out.write_all(&len.to_le_bytes())?;
    out.write_all(&frame.bytes)
}

/// Reads `events` for [`CAPTURE`], appending every raw frame to
/// `target/captures/ws/<feed>-<date>.bin`, and returns how many frames it wrote.
async fn capture<T: std::fmt::Debug>(feed: &str, mut events: FeedEvents<T>) -> usize {
    let path = capture_dir("ws").join(format!("{feed}-{}.bin", today()));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .expect("open the capture");
    let mut out = std::io::BufWriter::new(file);
    let (mut frames, mut data, mut bad) = (0usize, 0usize, 0usize);
    let deadline = tokio::time::Instant::now() + CAPTURE;
    loop {
        let event = tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            event = events.next() => event,
        };
        match event {
            Some(Ok(FeedEvent::Raw(frame))) => {
                write_record(&mut out, &frame).expect("write a record");
                frames += 1;
            }
            Some(Ok(FeedEvent::Data(_))) => data += 1,
            Some(Ok(FeedEvent::DecodeError(e))) => {
                bad += 1;
                println!("decode error: {e:?}");
            }
            Some(Ok(FeedEvent::Lifecycle(l))) => println!("lifecycle: {l:?}"),
            Some(Ok(other)) => println!("event: {other:?}"),
            Some(Err(e)) => panic!("the feed ended: {e}"),
            None => panic!("the feed ended early"),
        }
    }
    out.flush().expect("flush the capture");
    println!(
        "{feed}: {frames} frames, {data} data items, {bad} decode errors -> {}",
        path.display()
    );
    frames
}

/// Subscribes HDFC Bank (`NSE_EQ:1333`) and NIFTY (`IDX_I:13`) in `mode` and captures.
async fn market(mode: Mode, feed: &str) {
    let Some(credentials) = live_credentials() else {
        return;
    };
    let _serial = serial().await;
    let (handle, events, task) = MarketFeed::builder(credentials)
        .capture_raw(true)
        .spawn()
        .expect("spawn the market feed");
    let instruments = [
        Instrument::new(ExchangeSegment::NseEq, SecurityId::new("1333").unwrap()).unwrap(),
        Instrument::new(ExchangeSegment::IdxI, SecurityId::new("13").unwrap()).unwrap(),
    ];
    handle
        .subscribe(instruments, mode)
        .await
        .expect("subscribe");
    capture(feed, events).await;
    handle.shutdown().await.expect("shut down");
    println!("task: {:?}", task.join().await);
}

#[tokio::test]
#[ignore = "needs live credentials and a Data API subscription"]
async fn live_feed_market_ticker() {
    market(Mode::Ticker, "market-ticker").await;
}

#[tokio::test]
#[ignore = "needs live credentials and a Data API subscription"]
async fn live_feed_market_quote() {
    market(Mode::Quote, "market-quote").await;
}

#[tokio::test]
#[ignore = "needs live credentials and a Data API subscription"]
async fn live_feed_market_full() {
    market(Mode::Full, "market-full").await;
}

/// The account's order updates for a minute. Without order activity only the login answer, if
/// any, is captured: to record an order alert, place and cancel an order (for example with the
/// `live_mut_*` test, in another process) while this runs.
#[tokio::test]
#[ignore = "needs live credentials"]
async fn live_feed_order_update() {
    let Some(credentials) = live_credentials() else {
        return;
    };
    let _serial = serial().await;
    let (handle, events, task) = OrderUpdateFeed::builder(credentials)
        .capture_raw(true)
        .spawn()
        .expect("spawn the order-update feed");
    capture("order-update", events).await;
    handle.shutdown().await.expect("shut down");
    println!("task: {:?}", task.join().await);
}
