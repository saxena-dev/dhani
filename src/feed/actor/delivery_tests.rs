use std::time::SystemTime;

use futures_util::StreamExt;

use super::super::super::{Delivery, TerminalReason};
use super::*;

const WAIT: Duration = Duration::from_secs(1);

fn data(seq: u64) -> FeedEvent<u64> {
    FeedEvent::Data(Delivery {
        epoch: 1,
        seq,
        received_at: SystemTime::UNIX_EPOCH,
        value: seq,
    })
}

fn connected(epoch: u64) -> Lifecycle {
    Lifecycle::Connected { epoch }
}

fn seq_of(item: &FeedEvent<u64>) -> String {
    match item {
        FeedEvent::Data(d) => format!("d{}", d.value),
        FeedEvent::Lifecycle(Lifecycle::Connected { epoch }) => format!("l{epoch}"),
        FeedEvent::Lifecycle(Lifecycle::Lagged { dropped }) => format!("lagged{dropped}"),
        other => format!("{other:?}"),
    }
}

async fn next(events: &mut FeedEvents<u64>) -> String {
    seq_of(&events.next().await.unwrap().unwrap())
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn items_come_out_in_sequence_order_across_both_queues() {
    let (tx, mut events) = channel::<u64>(FeedKind::Market, 16, 16);
    // Lifecycle items are labelled by epoch, data items by value; push order is the expected
    // order.
    tx.push_lifecycle(connected(1)).unwrap();
    tx.push_data(data(2), OverflowPolicy::Fail, WAIT)
        .await
        .unwrap();
    tx.push_data(data(3), OverflowPolicy::Fail, WAIT)
        .await
        .unwrap();
    tx.push_lifecycle(connected(4)).unwrap();
    tx.push_data(data(5), OverflowPolicy::Fail, WAIT)
        .await
        .unwrap();
    tx.push_lifecycle(connected(6)).unwrap();
    assert_eq!(tx.queue_len(), 6);
    let mut got = Vec::new();
    for _ in 0..6 {
        got.push(next(&mut events).await);
    }
    assert_eq!(got, ["l1", "d2", "d3", "l4", "d5", "l6"]);
    assert_eq!(tx.queue_len(), 0);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn fail_policy_waits_for_room_then_overloads() {
    let (tx, _events) = channel::<u64>(FeedKind::Market, 2, 16);
    tx.push_data(data(1), OverflowPolicy::Fail, WAIT)
        .await
        .unwrap();
    tx.push_data(data(2), OverflowPolicy::Fail, WAIT)
        .await
        .unwrap();
    let start = Instant::now();
    let result = tx.push_data(data(3), OverflowPolicy::Fail, WAIT).await;
    assert_eq!(result, Err(PushError::Overload));
    assert_eq!(Instant::now() - start, WAIT);
    assert_eq!(tx.dropped_total(), 0);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn fail_policy_resumes_when_the_consumer_makes_room() {
    let (tx, mut events) = channel::<u64>(FeedKind::Market, 2, 16);
    tx.push_data(data(1), OverflowPolicy::Fail, WAIT)
        .await
        .unwrap();
    tx.push_data(data(2), OverflowPolicy::Fail, WAIT)
        .await
        .unwrap();
    let consumer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let first = next(&mut events).await;
        (first, events)
    });
    let start = Instant::now();
    tx.push_data(data(3), OverflowPolicy::Fail, WAIT)
        .await
        .unwrap();
    assert_eq!(Instant::now() - start, Duration::from_millis(300));
    let (first, mut events) = consumer.await.unwrap();
    assert_eq!(first, "d1");
    assert_eq!(
        (next(&mut events).await, next(&mut events).await),
        ("d2".into(), "d3".into())
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn drop_oldest_reports_the_loss_before_the_next_item() {
    let (tx, mut events) = channel::<u64>(FeedKind::Market, 2, 16);
    for seq in 1..=5 {
        tx.push_data(data(seq), OverflowPolicy::DropOldest, WAIT)
            .await
            .unwrap();
    }
    assert_eq!(tx.dropped_total(), 3);
    let got = [
        next(&mut events).await,
        next(&mut events).await,
        next(&mut events).await,
    ];
    assert_eq!(got, ["lagged3", "d4", "d5"]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_full_lifecycle_queue_is_an_overload_under_either_policy() {
    for policy in [OverflowPolicy::Fail, OverflowPolicy::DropOldest] {
        let (tx, _events) = channel::<u64>(FeedKind::Market, 2, 2);
        tx.push_lifecycle(connected(1)).unwrap();
        tx.push_lifecycle(connected(2)).unwrap();
        assert_eq!(
            tx.push_lifecycle(connected(3)),
            Err(PushError::Overload),
            "{policy:?}"
        );
        // Data pushes are unaffected by the lifecycle queue.
        tx.push_data(data(4), policy, WAIT).await.unwrap();
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_dropped_next_future_loses_nothing() {
    let (tx, mut events) = channel::<u64>(FeedKind::Market, 4, 4);
    // Poll next() while empty, then drop the future.
    assert!(
        tokio::time::timeout(Duration::from_millis(10), events.next())
            .await
            .is_err()
    );
    tx.push_data(data(1), OverflowPolicy::Fail, WAIT)
        .await
        .unwrap();
    // A future created and dropped before it is polled takes nothing either.
    drop(events.next());
    assert_eq!(next(&mut events).await, "d1");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_waiting_consumer_is_woken_by_a_push() {
    let (tx, mut events) = channel::<u64>(FeedKind::Market, 4, 4);
    let consumer = tokio::spawn(async move { next(&mut events).await });
    tokio::task::yield_now().await;
    assert!(!consumer.is_finished());
    tx.push_lifecycle(connected(7)).unwrap();
    assert_eq!(consumer.await.unwrap(), "l7");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_failed_feed_ends_with_one_error_after_the_queued_items() {
    let (tx, mut events) = channel::<u64>(FeedKind::Market, 4, 4);
    tx.push_data(data(1), OverflowPolicy::Fail, WAIT)
        .await
        .unwrap();
    tx.finish(Some(FeedError(TerminalReason::DeliveryOverload)));
    assert_eq!(next(&mut events).await, "d1");
    assert_eq!(
        events.next().await,
        Some(Err(FeedError(TerminalReason::DeliveryOverload)))
    );
    assert_eq!(events.next().await, None);
    assert_eq!(events.next().await, None);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_clean_shutdown_ends_with_none() {
    let (tx, mut events) = channel::<u64>(FeedKind::Market, 4, 4);
    tx.push_lifecycle(Lifecycle::Stopped).unwrap();
    tx.finish(None);
    assert_eq!(
        events.next().await,
        Some(Ok(FeedEvent::Lifecycle(Lifecycle::Stopped)))
    );
    assert_eq!(events.next().await, None);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_dropped_stream_is_reported_to_the_producer() {
    let (tx, events) = channel::<u64>(FeedKind::Market, 1, 4);
    tx.push_data(data(1), OverflowPolicy::Fail, WAIT)
        .await
        .unwrap();
    let pending =
        tokio::spawn(async move { tx.push_data(data(2), OverflowPolicy::Fail, WAIT).await });
    tokio::task::yield_now().await;
    drop(events);
    assert_eq!(pending.await.unwrap(), Err(PushError::ReceiverDropped));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_new_epoch_does_not_overtake_items_still_queued_from_the_old_one() {
    let (tx, mut events) = channel::<u64>(FeedKind::Market, 16, 16);
    // Epoch 1 data with a high per-epoch seq, then the reconnect's lifecycle items whose seq
    // restarts at 0, then epoch 2 data with a low seq.
    tx.push_data(data(900), OverflowPolicy::Fail, WAIT)
        .await
        .unwrap();
    tx.push_data(data(901), OverflowPolicy::Fail, WAIT)
        .await
        .unwrap();
    tx.push_lifecycle(connected(2)).unwrap();
    tx.push_data(data(0), OverflowPolicy::Fail, WAIT)
        .await
        .unwrap();
    let mut got = Vec::new();
    for _ in 0..4 {
        got.push(next(&mut events).await);
    }
    assert_eq!(got, ["d900", "d901", "l2", "d0"]);
}
