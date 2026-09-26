use std::sync::atomic::AtomicI64;

use super::*;
use crate::labels::EndpointId;
use crate::rest::endpoint::by_id;
use crate::types::ExchangeSegment;

const SEC: Duration = Duration::from_secs(1);
/// 2024-09-22 00:00:00 IST, an IST midnight: 20 000 UTC days minus 5 h 30 min.
const MIDNIGHT_IST: i64 = 1_727_980_200;

struct FakeClock(AtomicI64);

impl FakeClock {
    fn at(unix: i64) -> Arc<Self> {
        Arc::new(FakeClock(AtomicI64::new(unix)))
    }

    fn set(&self, unix: i64) {
        self.0.store(unix, Ordering::SeqCst);
    }
}

impl WallClock for FakeClock {
    fn unix_seconds(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn limiter_with(
    profile: QuotaProfile,
    limits: AdmissionLimits,
    clock: Arc<FakeClock>,
) -> RateLimiter {
    RateLimiter::with_clock(profile, limits, clock)
}

fn limiter() -> RateLimiter {
    limiter_with(
        QuotaProfile::dhan_v2(),
        AdmissionLimits::default(),
        FakeClock::at(MIDNIGHT_IST + 3600),
    )
}

fn patient() -> AdmissionLimits {
    AdmissionLimits::new(Duration::from_secs(60), 256).unwrap()
}

fn far() -> Instant {
    Instant::now() + Duration::from_secs(1_000_000)
}

async fn take(l: &RateLimiter, id: EndpointId) -> Result<Grant, Error> {
    l.acquire(by_id(id), None, None, far()).await
}

/// Acquires and dispatches, returning when the grant was issued.
async fn send(l: &RateLimiter, id: EndpointId) -> Instant {
    let mut grant = take(l, id).await.unwrap();
    grant.dispatch();
    Instant::now()
}

async fn modify(l: &RateLimiter, order: &OrderId) -> Result<Grant, Error> {
    l.acquire(by_id(EndpointId::OrdersModify), None, Some(order), far())
        .await
}

fn source(result: Result<Grant, Error>) -> (RateLimitSource, Duration) {
    let err = result.err().expect("admission was refused");
    assert_eq!(
        (err.kind(), err.stage()),
        (ErrorKind::RateLimited, Stage::NotSent)
    );
    let info = err.rate_limit().expect("rate-limit info");
    (info.source, info.waited)
}

fn chain_key(scrip: u32, day: u32) -> OptionChainKey {
    OptionChainKey {
        scrip,
        segment: ExchangeSegment::IdxI,
        expiry: chrono::NaiveDate::from_ymd_opt(2026, 10, day).unwrap(),
    }
}

#[test]
fn the_dhan_profile_matches_the_documented_table() {
    let p = QuotaProfile::dhan_v2();
    assert_eq!(p.version, "dhanhq-v2/DOC@2026-07-16");
    let r = |limit, secs| Window {
        limit,
        period: WindowPeriod::Rolling(Duration::from_secs(secs)),
    };
    let d = |limit| Window {
        limit,
        period: WindowPeriod::IstDay,
    };
    assert_eq!(
        p.windows(RateClass::Order),
        [r(10, 1), r(250, 60), r(1000, 3600), d(7000)]
    );
    assert_eq!(p.windows(RateClass::Data), [r(5, 1), d(100_000)]);
    assert_eq!(p.windows(RateClass::Quote), [r(1, 1)]);
    assert_eq!(p.windows(RateClass::NonTrading), [r(20, 1)]);
    assert_eq!(p.windows(RateClass::TokenGeneration), [r(1, 120)]);
    assert!(p.windows(RateClass::Unmetered).is_empty());
    assert_eq!(p.option_chain_key_period, Some(Duration::from_secs(3)));
    assert_eq!(p.modification_cap_per_order_per_day, Some(25));
}

#[test]
fn profile_and_limit_values_are_range_checked() {
    let zero = Window {
        limit: 0,
        period: WindowPeriod::IstDay,
    };
    let err = QuotaProfile::dhan_v2()
        .with_windows(RateClass::Data, vec![zero])
        .unwrap_err();
    assert_eq!(
        (err.field, err.reason),
        ("windows", "limit must be at least 1")
    );
    let instant = Window {
        limit: 1,
        period: WindowPeriod::Rolling(Duration::ZERO),
    };
    let err = QuotaProfile::dhan_v2()
        .with_windows(RateClass::Data, vec![instant])
        .unwrap_err();
    assert_eq!(
        (err.field, err.reason),
        ("windows", "rolling period must be between 1 ns and 24 h")
    );
    let endless = Window {
        limit: 1,
        period: WindowPeriod::Rolling(Duration::MAX),
    };
    assert!(
        QuotaProfile::dhan_v2()
            .with_windows(RateClass::Data, vec![endless])
            .is_err()
    );
    let day = Window {
        limit: 1,
        period: WindowPeriod::Rolling(Duration::from_secs(86_400)),
    };
    assert!(
        QuotaProfile::dhan_v2()
            .with_windows(RateClass::Data, vec![day])
            .is_ok()
    );

    assert_eq!(
        AdmissionLimits::default(),
        AdmissionLimits::new(5 * SEC, 256).unwrap()
    );
    assert!(AdmissionLimits::new(Duration::ZERO, 1).is_ok());
    assert!(AdmissionLimits::new(60 * SEC, 10_000).is_ok());
    let err = AdmissionLimits::new(61 * SEC, 1).unwrap_err();
    assert_eq!(
        (err.field, err.reason),
        ("max_wait", "must be between 0 and 60 s")
    );
    let err = AdmissionLimits::new(SEC, 0).unwrap_err();
    assert_eq!(
        (err.field, err.reason),
        ("max_waiters", "must be between 1 and 10000")
    );
    assert!(AdmissionLimits::new(SEC, 10_001).is_err());

    // Fields are public, so a caller can set them out of range; the limiter clamps them.
    let wild = AdmissionLimits {
        max_wait: 600 * SEC,
        max_waiters: 0,
    };
    let l = RateLimiter::new(QuotaProfile::dhan_v2(), wild);
    assert_eq!(l.limits(), AdmissionLimits::new(60 * SEC, 1).unwrap());
}

#[test]
fn clones_share_one_limiter() {
    let a = RateLimiter::default();
    let b = a.clone();
    assert!(a.ptr_eq(&b));
    assert!(!a.ptr_eq(&RateLimiter::disabled()));
    assert_eq!(
        format!("{a:?}"),
        "RateLimiter { profile: \"dhanhq-v2/DOC@2026-07-16\", limits: AdmissionLimits { max_wait: 5s, max_waiters: 256 }, .. }"
    );
}

#[test]
fn ist_days_turn_at_ist_midnight() {
    assert_eq!(ist_day(MIDNIGHT_IST), 20_000);
    assert_eq!(ist_day(MIDNIGHT_IST - 1), 19_999);
    assert_eq!(ist_day(MIDNIGHT_IST + 86_399), 20_000);
    assert_eq!(ist_day(-19_801), -1);
    assert_eq!(ist_day(-19_800), 0);
}

#[tokio::test(start_paused = true)]
async fn the_eleventh_order_in_a_second_waits_a_second() {
    let l = limiter();
    let start = Instant::now();
    for _ in 0..10 {
        assert_eq!(send(&l, EndpointId::OrdersPlace).await, start);
    }
    assert_eq!(send(&l, EndpointId::OrdersPlace).await, start + SEC);
}

#[tokio::test(start_paused = true)]
async fn the_order_minute_and_hour_windows_hold() {
    let l = limiter_with(
        QuotaProfile::dhan_v2(),
        patient(),
        FakeClock::at(MIDNIGHT_IST + 3600),
    );
    let start = Instant::now();
    // Ten per second: the 250th goes at 24 s, and the minute window then holds the 251st
    // until the first entry is 60 s old.
    for _ in 0..250 {
        send(&l, EndpointId::OrdersPlace).await;
    }
    assert_eq!(Instant::now(), start + 24 * SEC);
    assert_eq!(send(&l, EndpointId::OrdersPlace).await, start + 60 * SEC);
    // Fill the hour: batches of 250 start at 60, 120 and 180 s.
    for _ in 251..1000 {
        send(&l, EndpointId::OrdersPlace).await;
    }
    assert_eq!(Instant::now(), start + 204 * SEC);
    // The 1001st would wait until 3600 s, far past the 60 s wait bound.
    assert_eq!(
        source(take(&l, EndpointId::OrdersPlace).await),
        (RateLimitSource::LocalWaitExceeded, Duration::ZERO)
    );
}

#[tokio::test(start_paused = true)]
async fn the_second_quote_waits_a_second_and_non_trading_allows_twenty() {
    let l = limiter();
    let start = Instant::now();
    send(&l, EndpointId::MarketQuoteLtp).await;
    // Classes are independent: other classes are not held up by the quote window.
    for _ in 0..20 {
        assert_eq!(send(&l, EndpointId::OrdersList).await, start);
    }
    assert_eq!(send(&l, EndpointId::MarketQuoteOhlc).await, start + SEC);
    assert_eq!(send(&l, EndpointId::OrdersList).await, start + SEC);
}

#[tokio::test(start_paused = true)]
async fn token_generation_is_once_per_two_minutes_and_unmetered_is_free() {
    let l = limiter_with(
        QuotaProfile::dhan_v2(),
        patient(),
        FakeClock::at(MIDNIGHT_IST + 3600),
    );
    send(&l, EndpointId::AuthGenerateAccessToken).await;
    // 120 s is past the 60 s wait bound.
    assert_eq!(
        source(take(&l, EndpointId::AuthGenerateAccessToken).await).0,
        RateLimitSource::LocalWaitExceeded
    );
    let start = Instant::now();
    for _ in 0..1000 {
        assert_eq!(send(&l, EndpointId::AuthGenerateConsent).await, start);
    }
}

#[tokio::test(start_paused = true)]
async fn the_data_day_ceiling_is_a_hard_error() {
    let l = limiter();
    for _ in 0..100_000 {
        send(&l, EndpointId::HistoricalDaily).await;
    }
    let before = Instant::now();
    assert_eq!(
        source(take(&l, EndpointId::HistoricalIntraday).await),
        (RateLimitSource::LocalCeiling, Duration::ZERO)
    );
    assert_eq!(Instant::now(), before);
}

#[tokio::test(start_paused = true)]
async fn day_counters_reset_at_ist_midnight() {
    let one_a_day = QuotaProfile::dhan_v2()
        .with_windows(
            RateClass::Data,
            vec![Window {
                limit: 1,
                period: WindowPeriod::IstDay,
            }],
        )
        .unwrap();
    let clock = FakeClock::at(MIDNIGHT_IST - 1); // 23:59:59 IST
    let l = limiter_with(one_a_day, AdmissionLimits::default(), clock.clone());
    send(&l, EndpointId::HistoricalDaily).await;
    assert_eq!(
        source(take(&l, EndpointId::HistoricalDaily).await).0,
        RateLimitSource::LocalCeiling
    );
    clock.set(MIDNIGHT_IST + 1); // 00:00:01 IST
    send(&l, EndpointId::HistoricalDaily).await;
    assert_eq!(
        source(take(&l, EndpointId::HistoricalDaily).await).0,
        RateLimitSource::LocalCeiling
    );
}

#[tokio::test(start_paused = true)]
async fn the_option_chain_allows_one_request_per_key_every_three_seconds() {
    let l = limiter();
    let chain = by_id(EndpointId::OptionChainChain);
    let start = Instant::now();
    let (a, b) = (chain_key(13, 7), chain_key(13, 14));
    l.acquire(chain, Some(a), None, far())
        .await
        .unwrap()
        .dispatch();
    // Another expiry of the same underlying is a different key.
    l.acquire(chain, Some(b), None, far())
        .await
        .unwrap()
        .dispatch();
    assert_eq!(Instant::now(), start);
    l.acquire(chain, Some(a), None, far())
        .await
        .unwrap()
        .dispatch();
    assert_eq!(Instant::now(), start + 3 * SEC);
    // The key only applies to the option-chain endpoint itself.
    let expiries = by_id(EndpointId::OptionChainExpiries);
    l.acquire(expiries, Some(a), None, far())
        .await
        .unwrap()
        .dispatch();
    assert_eq!(Instant::now(), start + 3 * SEC);
}

#[tokio::test(start_paused = true)]
async fn the_twenty_sixth_modification_of_an_order_is_refused() {
    let clock = FakeClock::at(MIDNIGHT_IST + 3600);
    let l = limiter_with(
        QuotaProfile::dhan_v2(),
        AdmissionLimits::default(),
        clock.clone(),
    );
    let (a, b) = (
        OrderId::new("112111182198").unwrap(),
        OrderId::new("112111182199").unwrap(),
    );
    for _ in 0..25 {
        // Dispatched and then failed: still counted.
        let mut grant = modify(&l, &a).await.unwrap();
        grant.dispatch();
        drop(grant);
    }
    let before = Instant::now();
    assert_eq!(
        source(modify(&l, &a).await),
        (RateLimitSource::LocalCeiling, Duration::ZERO)
    );
    assert_eq!(Instant::now(), before);
    // Other orders and other order endpoints are unaffected.
    modify(&l, &b).await.unwrap().dispatch();
    let cancel = by_id(EndpointId::OrdersCancel);
    l.acquire(cancel, None, Some(&a), far())
        .await
        .unwrap()
        .dispatch();
    // The count resets at IST midnight.
    clock.set(MIDNIGHT_IST + 86_400);
    modify(&l, &a).await.unwrap().dispatch();
}

#[tokio::test(start_paused = true)]
async fn undispatched_modifications_reserve_the_cap_until_dropped() {
    let l = limiter_with(
        QuotaProfile::dhan_v2(),
        patient(),
        FakeClock::at(MIDNIGHT_IST + 3600),
    );
    let a = OrderId::new("112111182198").unwrap();
    for _ in 0..24 {
        modify(&l, &a).await.unwrap().dispatch();
    }
    let held = modify(&l, &a).await.unwrap();
    assert_eq!(
        source(modify(&l, &a).await).0,
        RateLimitSource::LocalCeiling
    );
    drop(held);
    let mut last = modify(&l, &a).await.unwrap();
    last.dispatch();
    assert_eq!(
        source(modify(&l, &a).await).0,
        RateLimitSource::LocalCeiling
    );
}

#[tokio::test(start_paused = true)]
async fn dropping_an_undispatched_grant_refunds_every_charge() {
    let one_a_day = QuotaProfile::dhan_v2()
        .with_windows(
            RateClass::Data,
            vec![Window::rolling(1, 1), Window::ist_day(1)],
        )
        .unwrap();
    let l = limiter_with(
        one_a_day,
        AdmissionLimits::default(),
        FakeClock::at(MIDNIGHT_IST + 3600),
    );
    let chain = by_id(EndpointId::OptionChainChain);
    let key = chain_key(13, 7);
    let start = Instant::now();
    let grant = l.acquire(chain, Some(key), None, far()).await.unwrap();
    assert_eq!(l.__outstanding(), 1);
    drop(grant);
    assert_eq!(l.__outstanding(), 0);
    // Rolling, daily and keyed charges were all returned: admitted again at once.
    let mut grant = l.acquire(chain, Some(key), None, far()).await.unwrap();
    assert_eq!(Instant::now(), start);
    grant.dispatch();
    assert_eq!(l.__outstanding(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_dispatched_grant_is_not_refunded() {
    let l = limiter();
    let start = Instant::now();
    let mut grant = take(&l, EndpointId::MarketQuoteLtp).await.unwrap();
    grant.dispatch();
    // The attempt then times out or fails: dropping the grant returns nothing.
    drop(grant);
    assert_eq!(send(&l, EndpointId::MarketQuoteLtp).await, start + SEC);
}

#[tokio::test(start_paused = true)]
async fn a_zero_wait_bound_refuses_instead_of_waiting() {
    let l = limiter_with(
        QuotaProfile::dhan_v2(),
        AdmissionLimits::new(Duration::ZERO, 256).unwrap(),
        FakeClock::at(MIDNIGHT_IST + 3600),
    );
    send(&l, EndpointId::MarketQuoteLtp).await;
    assert_eq!(
        source(take(&l, EndpointId::MarketQuoteLtp).await),
        (RateLimitSource::LocalWaitExceeded, Duration::ZERO)
    );
}

#[tokio::test(start_paused = true)]
async fn a_wait_past_the_operation_deadline_is_refused() {
    let l = limiter();
    send(&l, EndpointId::MarketQuoteLtp).await;
    let deadline = Instant::now() + Duration::from_millis(500);
    let result = l
        .acquire(by_id(EndpointId::MarketQuoteLtp), None, None, deadline)
        .await;
    assert_eq!(source(result).0, RateLimitSource::LocalWaitExceeded);
}

#[tokio::test(start_paused = true)]
async fn waiters_beyond_the_bound_fail_immediately() {
    let l = limiter_with(
        QuotaProfile::dhan_v2(),
        AdmissionLimits::new(5 * SEC, 1).unwrap(),
        FakeClock::at(MIDNIGHT_IST + 3600),
    );
    let start = Instant::now();
    send(&l, EndpointId::MarketQuoteLtp).await;
    let first = tokio::spawn({
        let l = l.clone();
        async move { send(&l, EndpointId::MarketQuoteLtp).await }
    });
    tokio::task::yield_now().await;
    assert_eq!(l.0.waiters.load(Ordering::SeqCst), 1);
    assert_eq!(
        source(take(&l, EndpointId::MarketQuoteLtp).await).0,
        RateLimitSource::LocalWaitExceeded
    );
    assert_eq!(first.await.unwrap(), start + SEC);
    assert_eq!(l.0.waiters.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn a_waiter_cancelled_before_admission_takes_nothing() {
    let l = limiter();
    let start = Instant::now();
    send(&l, EndpointId::MarketQuoteLtp).await;
    let waiter = tokio::spawn({
        let l = l.clone();
        async move { send(&l, EndpointId::MarketQuoteLtp).await }
    });
    tokio::task::yield_now().await;
    assert_eq!(l.0.waiters.load(Ordering::SeqCst), 1);
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    assert_eq!(
        (l.__outstanding(), l.0.waiters.load(Ordering::SeqCst)),
        (0, 0)
    );
    // The cancelled waiter charged nothing: the next call goes as soon as the window frees.
    assert_eq!(send(&l, EndpointId::MarketQuoteLtp).await, start + SEC);
}

#[tokio::test(start_paused = true)]
async fn a_disabled_limiter_admits_everything_and_still_counts() {
    let l = RateLimiter::disabled();
    let start = Instant::now();
    let a = OrderId::new("112111182198").unwrap();
    for _ in 0..100 {
        send(&l, EndpointId::MarketQuoteLtp).await;
        modify(&l, &a).await.unwrap().dispatch();
    }
    assert_eq!(Instant::now(), start);
    let grant = take(&l, EndpointId::OrdersPlace).await.unwrap();
    assert_eq!(l.__outstanding(), 1);
    drop(grant);
    assert_eq!(l.__outstanding(), 0);
}

#[tokio::test(start_paused = true)]
async fn the_sixth_data_call_in_a_second_waits_a_second() {
    let l = limiter();
    let start = Instant::now();
    for _ in 0..5 {
        assert_eq!(send(&l, EndpointId::HistoricalDaily).await, start);
    }
    assert_eq!(send(&l, EndpointId::HistoricalIntraday).await, start + SEC);
}

#[tokio::test(start_paused = true)]
async fn the_order_day_ceiling_is_a_hard_error() {
    // Only the IST-day window, so 7000 orders need no waiting.
    let day_only = QuotaProfile::dhan_v2()
        .with_windows(RateClass::Order, vec![Window::ist_day(7000)])
        .unwrap();
    let l = limiter_with(
        day_only,
        AdmissionLimits::default(),
        FakeClock::at(MIDNIGHT_IST + 3600),
    );
    for _ in 0..7000 {
        send(&l, EndpointId::OrdersPlace).await;
    }
    assert_eq!(
        source(take(&l, EndpointId::OrdersCancel).await),
        (RateLimitSource::LocalCeiling, Duration::ZERO)
    );
}

#[tokio::test(start_paused = true)]
async fn a_modification_dispatched_after_midnight_counts_against_the_new_day() {
    let tight = QuotaProfile {
        modification_cap_per_order_per_day: Some(1),
        ..QuotaProfile::dhan_v2()
    };
    let clock = FakeClock::at(MIDNIGHT_IST - 1);
    let l = limiter_with(tight, AdmissionLimits::default(), clock.clone());
    let a = OrderId::new("112111182198").unwrap();
    let mut reserved = modify(&l, &a).await.unwrap();
    clock.set(MIDNIGHT_IST + 1);
    // Another order's acquire rolls the day before the first modify is dispatched.
    modify(&l, &OrderId::new("112111182199").unwrap())
        .await
        .unwrap()
        .dispatch();
    reserved.dispatch();
    assert_eq!(
        source(modify(&l, &a).await).0,
        RateLimitSource::LocalCeiling
    );
}

#[tokio::test(start_paused = true)]
async fn a_dispatch_after_midnight_rolls_the_day_itself() {
    let tight = QuotaProfile {
        modification_cap_per_order_per_day: Some(1),
        ..QuotaProfile::dhan_v2()
    };
    let clock = FakeClock::at(MIDNIGHT_IST - 1);
    let l = limiter_with(tight, AdmissionLimits::default(), clock.clone());
    let a = OrderId::new("112111182198").unwrap();
    let mut reserved = modify(&l, &a).await.unwrap();
    clock.set(MIDNIGHT_IST + 1);
    // Nothing else ran since midnight: the dispatch still counts against the new day.
    reserved.dispatch();
    assert_eq!(
        source(modify(&l, &a).await).0,
        RateLimitSource::LocalCeiling
    );
}
