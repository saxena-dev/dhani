//! The retry decision for REST calls; backoff delays come from the crate `backoff` module.
//!
//! Only `Read` and `Query` endpoints are retried; `Mutation` and `Session` endpoints get exactly
//! one attempt for every cause. Remote rate limits are retried at most `rate_limit_retries` times
//! per operation with a delay of at least one second.

use std::time::{Duration, Instant, SystemTime};

use reqwest::header::HeaderValue;

use crate::backoff::{SplitMix64, full_jitter};
use crate::labels::RetryClass;
use crate::rest::endpoint::Endpoint;

/// Minimum delay before retrying after a remote rate limit (DOC:5328).
const RATE_LIMIT_FLOOR: Duration = Duration::from_secs(1);

/// A failure that may be retried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cause {
    /// HTTP 502 without a parsable broker error.
    Status502,
    /// HTTP 503 without a parsable broker error.
    Status503,
    /// HTTP 504 without a parsable broker error.
    Status504,
    /// The attempt deadline passed.
    Timeout,
    /// A transport error with no response.
    Transport,
    /// HTTP 429, `DH-904` or data code 805, with the server's `Retry-After`, if any.
    RemoteRateLimit {
        /// The parsed `Retry-After` header.
        retry_after: Option<Duration>,
    },
}

impl Cause {
    /// The cause label of the `http.retry.scheduled` event.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Status502 => "status_502",
            Self::Status503 => "status_503",
            Self::Status504 => "status_504",
            Self::Timeout => "timeout",
            Self::Transport => "transport",
            Self::RemoteRateLimit { .. } => "rate_limited",
        }
    }
}

/// The retry settings `decide` needs. Built from the client's `RetryPolicy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RetryLimits {
    /// Total attempts allowed per operation, including the first.
    pub max_attempts: u32,
    /// Base of the full-jitter backoff.
    pub initial_backoff: Duration,
    /// Cap of the full-jitter backoff.
    pub max_backoff: Duration,
    /// Extra attempts allowed for a remote rate limit, counted inside `max_attempts`.
    pub rate_limit_retries: u32,
    /// Base of the rate-limit backoff.
    pub rate_limit_initial_backoff: Duration,
}

/// What to do after a failed attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    /// Sleep for `delay`, then make another attempt.
    Retry {
        /// How long to wait before the next attempt.
        delay: Duration,
    },
    /// Give up and report the failure.
    Stop,
}

/// Decides whether to retry after attempt number `attempt` (1-based) failed with `cause`.
///
/// Retries only `Read` and `Query` endpoints, only while `attempt < max_attempts`, and only when
/// `now + delay` is still before `deadline`. Ordinary causes use full-jitter backoff; a remote
/// rate limit waits `max(Retry-After, rate_limit_initial_backoff · 2^(k−1), 1 s)`, where `k` is
/// the number of rate-limit retries including this one, and is retried at most
/// `rate_limit_retries` times.
#[allow(
    clippy::too_many_arguments,
    reason = "the inputs of step 9 of the pipeline, passed explicitly so the decision stays pure"
)]
pub(crate) fn decide(
    ep: &Endpoint,
    cause: Cause,
    attempt: u32,
    rate_limit_retries_used: u32,
    now: Instant,
    deadline: Instant,
    limits: &RetryLimits,
    rng: &mut SplitMix64,
) -> Decision {
    if !matches!(ep.retry, RetryClass::Read | RetryClass::Query) || attempt >= limits.max_attempts {
        return Decision::Stop;
    }
    let delay = match cause {
        Cause::RemoteRateLimit { retry_after } => {
            if rate_limit_retries_used >= limits.rate_limit_retries {
                return Decision::Stop;
            }
            let k = rate_limit_retries_used + 1;
            let exponential = match 1u32.checked_shl(k - 1) {
                Some(factor) => limits.rate_limit_initial_backoff.saturating_mul(factor),
                None => Duration::MAX,
            };
            exponential
                .max(retry_after.unwrap_or(Duration::ZERO))
                .max(RATE_LIMIT_FLOOR)
        }
        _ => full_jitter(limits.initial_backoff, limits.max_backoff, attempt, rng),
    };
    match now.checked_add(delay) {
        Some(wake) if wake < deadline => Decision::Retry { delay },
        _ => Decision::Stop,
    }
}

/// Parses a `Retry-After` header: delta-seconds or an HTTP-date. A date in the past gives zero.
pub(crate) fn parse_retry_after(value: &HeaderValue, now: SystemTime) -> Option<Duration> {
    let text = value.to_str().ok()?.trim();
    if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
        // Too large for u64: treat as "very long" so the deadline check stops the retry.
        return Some(text.parse().map_or(Duration::MAX, Duration::from_secs));
    }
    let at = chrono::DateTime::parse_from_rfc2822(text).ok()?;
    let at = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(u64::try_from(at.timestamp()).ok()?))?;
    Some(at.duration_since(now).unwrap_or(Duration::ZERO))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::labels::EndpointId;
    use crate::rest::endpoint::by_id;

    const MS: Duration = Duration::from_millis(1);

    /// The documented defaults of the retry policy: 3 attempts, 250 ms to 4 s, one rate-limit
    /// retry starting at 1 s.
    fn defaults() -> RetryLimits {
        RetryLimits {
            max_attempts: 3,
            initial_backoff: 250 * MS,
            max_backoff: 4000 * MS,
            rate_limit_retries: 1,
            rate_limit_initial_backoff: 1000 * MS,
        }
    }

    fn run(id: EndpointId, cause: Cause, attempt: u32, rl_used: u32, budget: Duration) -> Decision {
        let now = Instant::now();
        decide(
            by_id(id),
            cause,
            attempt,
            rl_used,
            now,
            now + budget,
            &defaults(),
            &mut SplitMix64::new(1),
        )
    }

    const LONG: Duration = Duration::from_secs(60);

    #[test]
    fn reads_retry_transient_causes_until_the_attempt_limit() {
        for cause in [
            Cause::Status502,
            Cause::Status503,
            Cause::Status504,
            Cause::Timeout,
            Cause::Transport,
        ] {
            let Decision::Retry { delay } = run(EndpointId::OrdersList, cause, 1, 0, LONG) else {
                panic!("{cause:?} at attempt 1 should retry")
            };
            assert!(delay <= 250 * MS, "{cause:?}: {delay:?}");
            let Decision::Retry { delay } = run(EndpointId::OrdersList, cause, 2, 0, LONG) else {
                panic!("{cause:?} at attempt 2 should retry")
            };
            assert!(delay <= 500 * MS, "{cause:?}: {delay:?}");
            assert_eq!(
                run(EndpointId::OrdersList, cause, 3, 0, LONG),
                Decision::Stop
            );
        }
        // A read-only query (POST) retries too.
        assert!(matches!(
            run(EndpointId::FundsMargin, Cause::Status503, 1, 0, LONG),
            Decision::Retry { .. }
        ));
    }

    #[test]
    fn mutations_and_session_calls_never_retry() {
        let causes = [
            Cause::Status502,
            Cause::Status503,
            Cause::Status504,
            Cause::Timeout,
            Cause::Transport,
            Cause::RemoteRateLimit { retry_after: None },
        ];
        for cause in causes {
            for id in [
                EndpointId::OrdersPlace,
                EndpointId::OrdersCancel,
                EndpointId::EdisGenerateTpin,
            ] {
                assert_eq!(
                    run(id, cause, 1, 0, LONG),
                    Decision::Stop,
                    "{id:?} {cause:?}"
                );
            }
            for id in [
                EndpointId::AuthGenerateAccessToken,
                EndpointId::AccountRenewToken,
            ] {
                assert_eq!(
                    run(id, cause, 1, 0, LONG),
                    Decision::Stop,
                    "{id:?} {cause:?}"
                );
            }
        }
    }

    #[test]
    fn remote_rate_limits_retry_once_with_at_least_one_second() {
        let rl = Cause::RemoteRateLimit { retry_after: None };
        let Decision::Retry { delay } = run(EndpointId::OrdersList, rl, 1, 0, LONG) else {
            panic!("first rate limit should retry")
        };
        assert_eq!(delay, 1000 * MS);
        assert_eq!(run(EndpointId::OrdersList, rl, 1, 1, LONG), Decision::Stop);
        let with_header = Cause::RemoteRateLimit {
            retry_after: Some(Duration::from_secs(3)),
        };
        let Decision::Retry { delay } = run(EndpointId::OrdersList, with_header, 1, 0, LONG) else {
            panic!("rate limit with Retry-After should retry")
        };
        assert_eq!(delay, Duration::from_secs(3));
        // A shorter Retry-After never lowers the delay below the backoff or the 1 s floor.
        let short = Cause::RemoteRateLimit {
            retry_after: Some(10 * MS),
        };
        assert!(
            matches!(run(EndpointId::OrdersList, short, 1, 0, LONG), Decision::Retry { delay } if delay == 1000 * MS)
        );
        // The k-th rate-limit retry doubles the base.
        let mut limits = defaults();
        limits.rate_limit_retries = 3;
        limits.max_attempts = 5;
        let now = Instant::now();
        let second = decide(
            by_id(EndpointId::OrdersList),
            rl,
            2,
            1,
            now,
            now + LONG,
            &limits,
            &mut SplitMix64::new(1),
        );
        assert_eq!(second, Decision::Retry { delay: 2000 * MS });
        // The floor applies even if the configured base is below it.
        limits.rate_limit_initial_backoff = 100 * MS;
        let floored = decide(
            by_id(EndpointId::OrdersList),
            rl,
            1,
            0,
            now,
            now + LONG,
            &limits,
            &mut SplitMix64::new(1),
        );
        assert_eq!(floored, Decision::Retry { delay: 1000 * MS });
    }

    #[test]
    fn a_delay_past_the_deadline_stops() {
        let rl = Cause::RemoteRateLimit {
            retry_after: Some(Duration::from_secs(3)),
        };
        assert_eq!(
            run(EndpointId::OrdersList, rl, 1, 0, Duration::from_secs(2)),
            Decision::Stop
        );
        assert_eq!(
            run(
                EndpointId::OrdersList,
                Cause::Status503,
                1,
                0,
                Duration::ZERO
            ),
            Decision::Stop
        );
        let huge = Cause::RemoteRateLimit {
            retry_after: Some(Duration::MAX),
        };
        assert_eq!(
            run(EndpointId::OrdersList, huge, 1, 0, LONG),
            Decision::Stop
        );
    }

    #[test]
    fn cause_labels() {
        let causes = [
            Cause::Status502,
            Cause::Status503,
            Cause::Status504,
            Cause::Timeout,
            Cause::Transport,
            Cause::RemoteRateLimit { retry_after: None },
        ];
        assert_eq!(
            causes.map(Cause::label),
            [
                "status_502",
                "status_503",
                "status_504",
                "timeout",
                "transport",
                "rate_limited"
            ]
        );
    }

    #[test]
    fn retry_after_parses_delta_seconds_and_http_dates() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(784_111_772); // Sun, 06 Nov 1994 08:49:32 GMT
        let h = |s: &str| HeaderValue::from_str(s).unwrap();
        assert_eq!(
            parse_retry_after(&h("3"), now),
            Some(Duration::from_secs(3))
        );
        assert_eq!(parse_retry_after(&h(" 0 "), now), Some(Duration::ZERO));
        assert_eq!(
            parse_retry_after(&h("Sun, 06 Nov 1994 08:49:37 GMT"), now),
            Some(Duration::from_secs(5))
        );
        assert_eq!(
            parse_retry_after(&h("Sun, 06 Nov 1994 08:49:30 GMT"), now),
            Some(Duration::ZERO)
        );
        for bad in ["", "-1", "1.5", "soon", "3s"] {
            assert_eq!(parse_retry_after(&h(bad), now), None, "{bad:?}");
        }
        assert_eq!(
            parse_retry_after(&h("99999999999999999999999"), now),
            Some(Duration::MAX)
        );
    }
}
