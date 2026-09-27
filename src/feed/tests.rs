use super::*;

#[test]
fn defaults_are_the_documented_policy_values() {
    let l = FeedLimits::default();
    assert_eq!(
        (
            l.queue_capacity,
            l.lifecycle_capacity,
            l.mailbox,
            l.max_frame_bytes
        ),
        (4096, 256, 64, 1024 * 1024)
    );
    assert_eq!(
        (
            l.delivery_wait,
            l.handshake_timeout,
            l.liveness_timeout,
            l.shutdown_timeout
        ),
        (SEC, 10 * SEC, 30 * SEC, 5 * SEC)
    );
    let r = ReconnectPolicy::default();
    assert_eq!(
        (
            r.max_attempts,
            r.outage_deadline,
            r.initial_backoff,
            r.max_backoff,
            r.jitter_seed
        ),
        (10, 300 * SEC, 500 * MS, 30 * SEC, None)
    );
    assert!(l.validate().is_ok());
    assert_eq!(OverflowPolicy::default(), OverflowPolicy::Fail);
}

/// A change to the default limits and the error it must produce.
type Case = (fn(&mut FeedLimits), &'static str, &'static str);

#[test]
fn every_limit_is_range_checked() {
    let cases: Vec<Case> = vec![
        (
            |l| l.queue_capacity = 10,
            "queue_capacity",
            "must be between 64 and 1000000",
        ),
        (
            |l| l.queue_capacity = 63,
            "queue_capacity",
            "must be between 64 and 1000000",
        ),
        (
            |l| l.queue_capacity = 1_000_001,
            "queue_capacity",
            "must be between 64 and 1000000",
        ),
        (
            |l| l.lifecycle_capacity = 15,
            "lifecycle_capacity",
            "must be between 16 and 4096",
        ),
        (|l| l.mailbox = 0, "mailbox", "must be between 1 and 1024"),
        (
            |l| l.mailbox = 1025,
            "mailbox",
            "must be between 1 and 1024",
        ),
        (
            |l| l.delivery_wait = 9 * MS,
            "delivery_wait",
            "must be between 10 ms and 10 s",
        ),
        (
            |l| l.handshake_timeout = 61 * SEC,
            "handshake_timeout",
            "must be between 1 s and 60 s",
        ),
        (
            |l| l.liveness_timeout = 4 * SEC,
            "liveness_timeout",
            "must be between 5 s and 120 s",
        ),
        (
            |l| l.shutdown_timeout = Duration::ZERO,
            "shutdown_timeout",
            "must be between 1 s and 60 s",
        ),
        (
            |l| l.max_frame_bytes = 4095,
            "max_frame_bytes",
            "must be between 4 KiB and 16 MiB",
        ),
        (
            |l| l.reconnect.max_attempts = 0,
            "reconnect.max_attempts",
            "must be between 1 and 100",
        ),
        (
            |l| l.reconnect.outage_deadline = 9 * SEC,
            "reconnect.outage_deadline",
            "must be between 10 s and 1 h",
        ),
        (
            |l| l.reconnect.initial_backoff = 99 * MS,
            "reconnect.initial_backoff",
            "must be between 100 ms and 10 s",
        ),
        (
            |l| l.reconnect.max_backoff = 400 * MS,
            "reconnect.max_backoff",
            "must be between the initial backoff and 5 min",
        ),
    ];
    for (mutate, field, reason) in cases {
        let mut l = FeedLimits::default();
        mutate(&mut l);
        let err = l.validate().unwrap_err();
        assert_eq!((err.field, err.reason), (field, reason));
    }
    let mut edges = FeedLimits {
        queue_capacity: 64,
        mailbox: 1024,
        max_frame_bytes: 16 * 1024 * 1024,
        ..FeedLimits::default()
    };
    edges.reconnect.max_backoff = edges.reconnect.initial_backoff;
    assert!(edges.validate().is_ok());
}

#[test]
fn raw_frames_print_their_length_only() {
    let raw = RawFrame {
        epoch: 1,
        seq: 2,
        received_at: SystemTime::UNIX_EPOCH,
        kind: FrameKind::Binary,
        bytes: vec![0xAB; 162],
    };
    let debug = format!("{raw:?}");
    assert!(debug.contains("len: 162"), "{debug}");
    assert!(!debug.contains("171"), "{debug}");
}

#[test]
fn feed_errors_display_their_reason() {
    assert_eq!(
        FeedError(TerminalReason::ServerDisconnect { code: 805 }).to_string(),
        "feed ended: server disconnected with code 805"
    );
    assert_eq!(
        FeedError(TerminalReason::AuthRejected { http_status: 401 }).to_string(),
        "feed ended: authentication rejected (HTTP 401)"
    );
    assert_eq!(
        FeedSpawnError::UnsupportedEnvironment.to_string(),
        "unsupported environment"
    );
}
