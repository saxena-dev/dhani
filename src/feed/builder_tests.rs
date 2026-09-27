use std::collections::BTreeSet;
use std::time::Duration;

use secrecy::SecretString;

use super::super::protocol::{Decoded, FeedProtocol, Message, sealed};
use super::*;
use crate::feed::{CommandError, SubscriptionCommand};
use crate::labels::FeedKind;

/// A protocol with no wire behaviour, for driving the owner.
struct TestProtocol {
    url: SecretString,
}

impl sealed::Sealed for TestProtocol {}

impl FeedProtocol for TestProtocol {
    type Sub = u32;
    type Data = ();
    const FEED: FeedKind = FeedKind::Market;

    fn url(&self) -> &SecretString {
        &self.url
    }

    fn on_open(&self) -> Vec<Message> {
        Vec::new()
    }

    fn capacity(&self) -> usize {
        10
    }

    fn reconcile(&self, _sent: &BTreeSet<u32>, _desired: &BTreeSet<u32>) -> Vec<String> {
        Vec::new()
    }

    fn decode(&mut self, _frame: &Message, _out: &mut Vec<Decoded<()>>) {}

    fn disconnect_message(&self) -> Option<String> {
        None
    }

    fn client_ping_interval(&self) -> Option<Duration> {
        None
    }
}

fn builder(endpoint_available: bool) -> FeedBuilder<TestProtocol> {
    FeedBuilder::new(
        TestProtocol {
            url: SecretString::from("wss://feed.example.invalid/"),
        },
        endpoint_available,
    )
}

#[test]
fn spawning_outside_a_runtime_is_refused() {
    assert!(matches!(
        builder(true).spawn(),
        Err(FeedSpawnError::NoRuntime)
    ));
}

#[tokio::test]
async fn a_non_websocket_url_is_refused() {
    let url = url::Url::parse("ftp://x").unwrap();
    assert!(matches!(
        builder(true).url(url).spawn(),
        Err(FeedSpawnError::InvalidUrl)
    ));
}

#[tokio::test]
async fn an_environment_without_an_endpoint_needs_a_url() {
    assert!(matches!(
        builder(false).spawn(),
        Err(FeedSpawnError::UnsupportedEnvironment)
    ));
}

#[tokio::test]
async fn invalid_limits_are_refused() {
    let limits = FeedLimits {
        mailbox: 0,
        ..FeedLimits::default()
    };
    assert!(matches!(
        builder(true).limits(limits).spawn(),
        Err(FeedSpawnError::Config(_))
    ));
}

/// A TCP listener that never answers the WebSocket handshake.
async fn silent_server() -> (tokio::net::TcpListener, url::Url) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = url::Url::parse(&format!("ws://{}/", listener.local_addr().unwrap())).unwrap();
    (listener, url)
}

// real-time: the handshake stalls on a loopback socket (a paused clock would time it out at
// once); the test ends by shutdown long before the 10 s handshake timeout.
#[tokio::test]
async fn a_full_mailbox_refuses_at_once_and_status_never_waits() {
    let (_listener, url) = silent_server().await;
    let limits = FeedLimits {
        mailbox: 1,
        ..FeedLimits::default()
    };
    let (handle, _events, task) = builder(true).url(url).limits(limits).spawn().unwrap();
    // The owner is stuck in the handshake and does not read the mailbox.
    let status = handle.status();
    assert_eq!(status.terminal, None);
    let first = tokio::spawn({
        let handle = handle.clone();
        async move {
            handle
                .command(SubscriptionCommand::Subscribe(vec![1]))
                .await
        }
    });
    tokio::task::yield_now().await;
    assert_eq!(
        handle
            .command(SubscriptionCommand::Subscribe(vec![2]))
            .await,
        Err(CommandError::MailboxFull)
    );
    handle.shutdown().await.unwrap();
    assert_eq!(task.join().await, TaskOutcome::Clean);
    // The queued command was never applied; its reply reports the ended feed.
    assert_eq!(
        first.await.unwrap(),
        Err(CommandError::Terminated(TerminalReason::Shutdown))
    );
}

// real-time: the 1 s handshake timeout runs against a silent loopback socket.
#[tokio::test]
async fn dropping_every_handle_ends_the_feed() {
    let (_listener, url) = silent_server().await;
    let limits = FeedLimits {
        handshake_timeout: Duration::from_secs(1),
        ..FeedLimits::default()
    };
    let (handle, _events, task) = builder(true).url(url).limits(limits).spawn().unwrap();
    drop(handle);
    // The handshake times out, the feed backs off and notices the handles are gone.
    let outcome = tokio::time::timeout(Duration::from_secs(10), task.join())
        .await
        .unwrap();
    assert_eq!(
        outcome,
        TaskOutcome::Terminal(TerminalReason::HandlesDropped)
    );
}
