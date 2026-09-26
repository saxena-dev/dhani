//! `RateLimiter`, `QuotaProfile`, `Window`, `WindowPeriod`, `AdmissionLimits`, `Grant` and
//! `WallClock`.
//!
//! This is the admission interface the transport uses. The current limiter admits every request
//! immediately; the sliding windows, daily ceilings, keyed option-chain window and modification
//! cap replace its internals without changing this interface.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::time::Instant;

use crate::error::Error;
use crate::rest::endpoint::Endpoint;
use crate::rest::transport::OptionChainKey;
use crate::types::OrderId;

/// Client-side admission control shared by every client built with it.
///
/// Clone it (cheaply) and pass it to several clients of the same account so that they share one
/// budget.
#[derive(Clone, Default)]
pub struct RateLimiter(Arc<LimiterInner>);

#[derive(Default)]
struct LimiterInner {
    /// Grants issued and neither dispatched nor refunded yet.
    outstanding: AtomicUsize,
}

impl fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RateLimiter").finish_non_exhaustive()
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "reachable once the rest module re-exports RateLimiter with the client"
    )
)]
impl RateLimiter {
    /// A limiter that admits every request immediately.
    pub fn disabled() -> Self {
        Self::default()
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

    /// Admits one attempt at `ep`, waiting at most until `deadline`.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "called by the transport pipeline")
    )]
    pub(crate) async fn acquire(
        &self,
        _ep: &'static Endpoint,
        _key: Option<OptionChainKey>,
        _order_id: Option<&OrderId>,
        _deadline: Instant,
    ) -> Result<Grant, Error> {
        self.0.outstanding.fetch_add(1, Ordering::SeqCst);
        Ok(Grant {
            limiter: Arc::clone(&self.0),
            dispatched: false,
        })
    }
}

/// Permission for one attempt. Dropping it before [`dispatch`](Grant::dispatch) refunds the
/// capacity it reserved; once dispatched, nothing is refunded because the request may have
/// reached the broker.
pub(crate) struct Grant {
    limiter: Arc<LimiterInner>,
    dispatched: bool,
}

impl Grant {
    /// Marks the grant as used; call it immediately before the request is handed to the HTTP
    /// client.
    pub(crate) fn dispatch(&mut self) {
        if !self.dispatched {
            self.dispatched = true;
            self.limiter.outstanding.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl Drop for Grant {
    fn drop(&mut self) {
        if !self.dispatched {
            // Never sent: refund the reservation.
            self.limiter.outstanding.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::labels::EndpointId;
    use crate::rest::endpoint::by_id;

    fn far() -> Instant {
        Instant::now() + std::time::Duration::from_secs(60)
    }

    #[tokio::test]
    async fn grants_are_refunded_unless_dispatched() {
        let limiter = RateLimiter::disabled();
        let ep = by_id(EndpointId::OrdersList);
        let grant = limiter.acquire(ep, None, None, far()).await.unwrap();
        assert_eq!(limiter.__outstanding(), 1);
        drop(grant);
        assert_eq!(limiter.__outstanding(), 0);

        let mut grant = limiter.acquire(ep, None, None, far()).await.unwrap();
        grant.dispatch();
        grant.dispatch();
        assert_eq!(limiter.__outstanding(), 0);
        drop(grant);
        assert_eq!(limiter.__outstanding(), 0);
    }

    #[test]
    fn clones_share_one_limiter() {
        let a = RateLimiter::default();
        let b = a.clone();
        assert!(a.ptr_eq(&b));
        assert!(!a.ptr_eq(&RateLimiter::disabled()));
        assert_eq!(format!("{a:?}"), "RateLimiter { .. }");
    }
}
