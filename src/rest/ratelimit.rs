//! `RateLimiter`, `QuotaProfile`, `Window`, `WindowPeriod`, `AdmissionLimits`, `Grant` and
//! `WallClock`.
//!
//! Admission is a sliding-window log per `(RateClass, window)` plus IST-day ceilings, a keyed
//! option-chain window and a per-order modification cap, all behind one `std::sync::Mutex` that
//! is never held across an `.await`. Capacity is charged when a [`Grant`] is issued and returned
//! if the grant is dropped before it is dispatched.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::time::Instant;

use crate::error::{ConfigError, Error, ErrorKind, RateLimitInfo, RateLimitSource, Stage};
use crate::labels::RateClass;
use crate::rest::endpoint::Endpoint;
use crate::rest::transport::OptionChainKey;
use crate::types::OrderId;

/// Seconds east of UTC for India Standard Time.
const IST_OFFSET_SECONDS: i64 = 19_800;
const SECONDS_PER_DAY: i64 = 86_400;
/// Longest rolling window a profile accepts; longer budgets use [`WindowPeriod::IstDay`].
const MAX_ROLLING_PERIOD: Duration = Duration::from_secs(86_400);

/// A source of the current Unix time, from which the IST day is derived. Replace it in tests to
/// cross an IST midnight without waiting.
pub trait WallClock: Send + Sync + 'static {
    /// Seconds since the Unix epoch.
    fn unix_seconds(&self) -> i64;
}

/// The system clock.
struct SystemClock;

impl WallClock for SystemClock {
    fn unix_seconds(&self) -> i64 {
        match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
            Err(e) => -i64::try_from(e.duration().as_secs()).unwrap_or(i64::MAX),
        }
    }
}

/// The IST calendar day number of a Unix time.
fn ist_day(unix_seconds: i64) -> i64 {
    unix_seconds
        .saturating_add(IST_OFFSET_SECONDS)
        .div_euclid(SECONDS_PER_DAY)
}

/// How long a window counts requests for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowPeriod {
    /// A sliding window of this length; a full window makes callers wait.
    Rolling(Duration),
    /// The IST calendar day; a full window is a hard ceiling until IST midnight.
    IstDay,
}

/// At most `limit` requests per `period`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    /// Requests allowed in one period.
    pub limit: u32,
    /// The period.
    pub period: WindowPeriod,
}

impl Window {
    const fn rolling(limit: u32, secs: u64) -> Self {
        Window {
            limit,
            period: WindowPeriod::Rolling(Duration::from_secs(secs)),
        }
    }

    const fn ist_day(limit: u32) -> Self {
        Window {
            limit,
            period: WindowPeriod::IstDay,
        }
    }
}

/// The request budget per rate class, plus the keyed option-chain window and the modification
/// cap.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct QuotaProfile {
    /// Identifies the source of the numbers.
    pub version: &'static str,
    windows: BTreeMap<RateClass, Vec<Window>>,
    option_chain_key_period: Option<Duration>,
    modification_cap_per_order_per_day: Option<u32>,
}

impl QuotaProfile {
    /// Dhan's documented limits.
    ///
    /// - Order: 10/s, 250/min, 1000/h, 7000/day; Data: 5/s, 100 000/day; Quote: 1/s;
    ///   non-trading: 20/s (DOC:476-487; the quote rate is restated at DOC:2890).
    /// - Access-token generation: once every 2 minutes (DOC:8836).
    /// - Option chain: one request per unique underlying and expiry every 3 s (DOC:3236).
    /// - 25 modifications per order (DOC:485).
    ///
    /// Which endpoints fall into which class is the SDK's own mapping, and so is the reset of the
    /// modification count at IST midnight.
    pub fn dhan_v2() -> Self {
        let mut windows = BTreeMap::new();
        windows.insert(
            RateClass::Order,
            vec![
                Window::rolling(10, 1),
                Window::rolling(250, 60),
                Window::rolling(1000, 3600),
                Window::ist_day(7000),
            ],
        );
        windows.insert(
            RateClass::Data,
            vec![Window::rolling(5, 1), Window::ist_day(100_000)],
        );
        windows.insert(RateClass::Quote, vec![Window::rolling(1, 1)]);
        windows.insert(RateClass::NonTrading, vec![Window::rolling(20, 1)]);
        windows.insert(RateClass::TokenGeneration, vec![Window::rolling(1, 120)]);
        windows.insert(RateClass::Unmetered, Vec::new());
        QuotaProfile {
            version: "dhanhq-v2/DOC@2026-07-16",
            windows,
            option_chain_key_period: Some(Duration::from_secs(3)),
            modification_cap_per_order_per_day: Some(25),
        }
    }

    /// A profile with no limits at all.
    fn unlimited() -> Self {
        QuotaProfile {
            version: "unlimited",
            windows: BTreeMap::new(),
            option_chain_key_period: None,
            modification_cap_per_order_per_day: None,
        }
    }

    /// The same profile with `class` limited by `windows` instead (an empty list removes every
    /// limit on the class). Every window needs a limit of at least 1, and a rolling period between
    /// 1 ns and 24 h.
    pub fn with_windows(
        mut self,
        class: RateClass,
        windows: Vec<Window>,
    ) -> Result<Self, ConfigError> {
        for w in &windows {
            if w.limit == 0 {
                return Err(ConfigError::new("windows", "limit must be at least 1"));
            }
            if let WindowPeriod::Rolling(period) = w.period
                && (period.is_zero() || period > MAX_ROLLING_PERIOD)
            {
                return Err(ConfigError::new(
                    "windows",
                    "rolling period must be between 1 ns and 24 h",
                ));
            }
        }
        self.windows.insert(class, windows);
        Ok(self)
    }

    /// The windows of `class`.
    pub fn windows(&self, class: RateClass) -> &[Window] {
        self.windows.get(&class).map_or(&[], Vec::as_slice)
    }
}

/// Bounds on waiting for admission (SDK policy).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmissionLimits {
    /// Longest wait for capacity: 0..=60 s (default 5 s). Zero never waits.
    pub max_wait: Duration,
    /// Most callers waiting at once: 1..=10 000 (default 256). A caller that would exceed it
    /// fails immediately.
    pub max_waiters: usize,
}

impl AdmissionLimits {
    const MAX_WAIT: Duration = Duration::from_secs(60);
    const MAX_WAITERS: usize = 10_000;

    /// Range-checked limits.
    pub fn new(max_wait: Duration, max_waiters: usize) -> Result<Self, ConfigError> {
        if max_wait > Self::MAX_WAIT {
            return Err(ConfigError::new("max_wait", "must be between 0 and 60 s"));
        }
        if !(1..=Self::MAX_WAITERS).contains(&max_waiters) {
            return Err(ConfigError::new(
                "max_waiters",
                "must be between 1 and 10000",
            ));
        }
        Ok(AdmissionLimits {
            max_wait,
            max_waiters,
        })
    }

    /// The limits forced into range (fields are public and may have been set directly).
    fn clamped(self) -> Self {
        AdmissionLimits {
            max_wait: self.max_wait.min(Self::MAX_WAIT),
            max_waiters: self.max_waiters.clamp(1, Self::MAX_WAITERS),
        }
    }
}

impl Default for AdmissionLimits {
    fn default() -> Self {
        AdmissionLimits {
            max_wait: Duration::from_secs(5),
            max_waiters: 256,
        }
    }
}

/// Client-side admission control shared by every client built with it.
///
/// Clone it (cheaply) and pass it to several clients of the same account so that they share one
/// budget.
#[derive(Clone)]
pub struct RateLimiter(Arc<LimiterInner>);

struct LimiterInner {
    profile: QuotaProfile,
    limits: AdmissionLimits,
    clock: Arc<dyn WallClock>,
    state: Mutex<State>,
    /// Callers currently sleeping for capacity.
    waiters: AtomicUsize,
    /// Grants issued and neither dispatched nor refunded yet.
    outstanding: AtomicUsize,
}

#[derive(Default)]
struct State {
    next_grant: u64,
    /// The IST day the day counters belong to.
    day: i64,
    /// Rolling logs of `(grant, issued at)` per `(class, window index)`.
    rolling: HashMap<(RateClass, usize), VecDeque<(u64, Instant)>>,
    /// Requests charged today per `(class, window index)` of the IST-day windows.
    daily: HashMap<(RateClass, usize), u32>,
    /// Rolling logs per option-chain key.
    keyed: HashMap<OptionChainKey, VecDeque<(u64, Instant)>>,
    /// Modifications per order today.
    modifications: HashMap<OrderId, ModCount>,
}

#[derive(Default, Clone, Copy)]
struct ModCount {
    /// Dispatched modifications; never refunded.
    committed: u32,
    /// Granted but not yet dispatched.
    reserved: u32,
}

impl State {
    fn roll_day(&mut self, today: i64) {
        if self.day != today {
            self.day = today;
            self.daily.clear();
            self.modifications.clear();
        }
    }
}

/// The outcome of one admission attempt under the lock.
enum Attempt {
    Granted(Grant),
    /// Capacity frees at this instant.
    Wait(Instant),
    /// A daily ceiling or the modification cap is exhausted.
    Ceiling,
}

impl fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RateLimiter")
            .field("profile", &self.0.profile.version)
            .field("limits", &self.0.limits)
            .finish_non_exhaustive()
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(QuotaProfile::dhan_v2(), AdmissionLimits::default())
    }
}

impl RateLimiter {
    /// A limiter enforcing `profile`, waiting within `limits` (out-of-range limits are clamped).
    pub fn new(profile: QuotaProfile, limits: AdmissionLimits) -> Self {
        Self::with_clock(profile, limits, Arc::new(SystemClock))
    }

    /// A limiter that reads the IST day from `clock`.
    pub fn with_clock(
        profile: QuotaProfile,
        limits: AdmissionLimits,
        clock: Arc<dyn WallClock>,
    ) -> Self {
        let day = ist_day(clock.unix_seconds());
        RateLimiter(Arc::new(LimiterInner {
            profile,
            limits: limits.clamped(),
            clock,
            state: Mutex::new(State {
                day,
                ..State::default()
            }),
            waiters: AtomicUsize::new(0),
            outstanding: AtomicUsize::new(0),
        }))
    }

    /// A limiter that admits every request immediately (it still counts grants).
    pub fn disabled() -> Self {
        Self::new(QuotaProfile::unlimited(), AdmissionLimits::default())
    }

    /// The profile in force.
    pub fn profile(&self) -> &QuotaProfile {
        &self.0.profile
    }

    /// The admission bounds in force.
    pub fn limits(&self) -> AdmissionLimits {
        self.0.limits
    }

    /// Whether `self` and `other` are the same limiter (clones of one another).
    #[doc(hidden)]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// The number of grants issued and neither dispatched nor refunded. Test support.
    #[doc(hidden)]
    pub fn __outstanding(&self) -> usize {
        self.0.outstanding.load(Ordering::SeqCst)
    }

    /// Admits one attempt at `ep`, waiting at most until the earlier of `max_wait` from now and
    /// `deadline`. `key` is the option-chain key of an option-chain request; `order_id` is the
    /// order a modification counts against.
    pub(crate) async fn acquire(
        &self,
        ep: &'static Endpoint,
        key: Option<OptionChainKey>,
        order_id: Option<&OrderId>,
        deadline: Instant,
    ) -> Result<Grant, Error> {
        let start = Instant::now();
        let wait_until = (start + self.0.limits.max_wait).min(deadline);
        let modify = order_id.filter(|_| ep.modification_cap);
        let mut waiting: Option<WaiterGuard<'_>> = None;
        loop {
            match self.try_grant(ep, key, modify) {
                Attempt::Granted(grant) => return Ok(grant),
                Attempt::Ceiling => {
                    return Err(refused(ep, RateLimitSource::LocalCeiling, start));
                }
                Attempt::Wait(ready_at) => {
                    if ready_at > wait_until {
                        return Err(refused(ep, RateLimitSource::LocalWaitExceeded, start));
                    }
                    if waiting.is_none() {
                        let guard = WaiterGuard::enter(&self.0.waiters);
                        if guard.count > self.0.limits.max_waiters {
                            return Err(refused(ep, RateLimitSource::LocalWaitExceeded, start));
                        }
                        waiting = Some(guard);
                    }
                    tokio::time::sleep_until(ready_at).await;
                }
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn try_grant(
        &self,
        ep: &'static Endpoint,
        key: Option<OptionChainKey>,
        modify: Option<&OrderId>,
    ) -> Attempt {
        let now = Instant::now();
        let today = ist_day(self.0.clock.unix_seconds());
        let class = ep.rate;
        let windows = self.0.profile.windows(class);
        let key = key.filter(|_| ep.keyed_option_chain);
        let key_period = key.and(self.0.profile.option_chain_key_period);
        let cap = modify.and(self.0.profile.modification_cap_per_order_per_day);
        let mut state = self.lock();
        state.roll_day(today);

        // Hard ceilings first: they never wait.
        for (i, w) in windows.iter().enumerate() {
            if w.period == WindowPeriod::IstDay
                && state.daily.get(&(class, i)).copied().unwrap_or(0) >= w.limit
            {
                return Attempt::Ceiling;
            }
        }
        if let (Some(cap), Some(order)) = (cap, modify) {
            let used = state.modifications.get(order).copied().unwrap_or_default();
            if used.committed.saturating_add(used.reserved) >= cap {
                return Attempt::Ceiling;
            }
        }

        let mut ready_at: Option<Instant> = None;
        // A period is at most 24 h, but an instant that cannot be represented still means "not
        // within any wait", never a panic.
        let far = now + MAX_ROLLING_PERIOD;
        let mut later = |t: Instant, period: Duration| {
            let t = t.checked_add(period).unwrap_or(far).max(now);
            ready_at = Some(ready_at.map_or(t, |r| r.max(t)));
        };
        for (i, w) in windows.iter().enumerate() {
            let (WindowPeriod::Rolling(period), limit) = (w.period, w.limit) else {
                continue;
            };
            let log = state.rolling.entry((class, i)).or_default();
            prune(log, now, period);
            if log.len() >= limit as usize {
                later(log[log.len() - limit as usize].1, period);
            }
        }
        if let (Some(period), Some(k)) = (key_period, key) {
            // Forget idle keys so the map stays bounded by recent traffic.
            state.keyed.retain(|_, log| {
                prune(log, now, period);
                !log.is_empty()
            });
            if let Some(front) = state.keyed.get(&k).and_then(|log| log.front()) {
                later(front.1, period);
            }
        }
        if let Some(t) = ready_at {
            return Attempt::Wait(t);
        }

        state.next_grant += 1;
        let id = state.next_grant;
        for (i, w) in windows.iter().enumerate() {
            match w.period {
                WindowPeriod::Rolling(_) => {
                    state
                        .rolling
                        .entry((class, i))
                        .or_default()
                        .push_back((id, now));
                }
                WindowPeriod::IstDay => *state.daily.entry((class, i)).or_default() += 1,
            }
        }
        let key = key_period.and(key);
        if let Some(k) = key {
            state.keyed.entry(k).or_default().push_back((id, now));
        }
        let modify = cap.and(modify).cloned();
        if let Some(order) = &modify {
            state
                .modifications
                .entry(order.clone())
                .or_default()
                .reserved += 1;
        }
        drop(state);
        self.0.outstanding.fetch_add(1, Ordering::SeqCst);
        Attempt::Granted(Grant {
            limiter: Arc::clone(&self.0),
            id,
            class,
            key,
            modify,
            day: today,
            dispatched: false,
        })
    }
}

/// Drops log entries older than `period`.
fn prune(log: &mut VecDeque<(u64, Instant)>, now: Instant, period: Duration) {
    while log
        .front()
        .is_some_and(|(_, t)| now.saturating_duration_since(*t) >= period)
    {
        log.pop_front();
    }
}

fn refused(ep: &Endpoint, source: RateLimitSource, start: Instant) -> Error {
    Error::new(ErrorKind::RateLimited, Stage::NotSent)
        .with_endpoint(ep.id)
        .with_rate_limit(RateLimitInfo {
            source,
            class: ep.rate,
            waited: start.elapsed(),
        })
}

/// Counts one caller as waiting until dropped.
struct WaiterGuard<'a> {
    waiters: &'a AtomicUsize,
    /// The waiter count including this caller.
    count: usize,
}

impl<'a> WaiterGuard<'a> {
    fn enter(waiters: &'a AtomicUsize) -> Self {
        let count = waiters.fetch_add(1, Ordering::SeqCst) + 1;
        WaiterGuard { waiters, count }
    }
}

impl Drop for WaiterGuard<'_> {
    fn drop(&mut self) {
        self.waiters.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Permission for one attempt. Dropping it before [`dispatch`](Grant::dispatch) refunds the
/// capacity it reserved; once dispatched, nothing is refunded because the request may have
/// reached the broker.
pub(crate) struct Grant {
    limiter: Arc<LimiterInner>,
    id: u64,
    class: RateClass,
    key: Option<OptionChainKey>,
    /// The order whose modification cap this grant reserved.
    modify: Option<OrderId>,
    /// The IST day the grant was charged to.
    day: i64,
    dispatched: bool,
}

impl Grant {
    /// Marks the grant as used and charges the modification cap where it applies; call it
    /// immediately before the request is handed to the HTTP client.
    pub(crate) fn dispatch(&mut self) {
        if self.dispatched {
            return;
        }
        self.dispatched = true;
        if let Some(order) = &self.modify {
            let now_day = ist_day(self.limiter.clock.unix_seconds());
            let mut state = self.limiter.state.lock().unwrap_or_else(|e| e.into_inner());
            state.roll_day(now_day);
            let today = state.day == self.day;
            let count = state.modifications.entry(order.clone()).or_default();
            // Reserved before an IST midnight that has since rolled the counters: the
            // reservation is gone, and the dispatch counts against the new day.
            if today {
                count.reserved = count.reserved.saturating_sub(1);
            }
            count.committed = count.committed.saturating_add(1);
        }
        self.limiter.outstanding.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Drop for Grant {
    fn drop(&mut self) {
        if self.dispatched {
            return;
        }
        // Never sent: remove every charge this grant made.
        let mut state = self.limiter.state.lock().unwrap_or_else(|e| e.into_inner());
        let id = self.id;
        for ((class, _), log) in state.rolling.iter_mut() {
            if *class == self.class {
                log.retain(|(g, _)| *g != id);
            }
        }
        if let Some(log) = self.key.and_then(|k| state.keyed.get_mut(&k)) {
            log.retain(|(g, _)| *g != id);
        }
        if state.day == self.day {
            let windows = self.limiter.profile.windows(self.class);
            for (i, w) in windows.iter().enumerate() {
                if w.period == WindowPeriod::IstDay
                    && let Some(n) = state.daily.get_mut(&(self.class, i))
                {
                    *n = n.saturating_sub(1);
                }
            }
            if let Some(count) = self
                .modify
                .as_ref()
                .and_then(|o| state.modifications.get_mut(o))
            {
                count.reserved = count.reserved.saturating_sub(1);
            }
        }
        drop(state);
        self.limiter.outstanding.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
#[path = "ratelimit_tests.rs"]
mod tests;
