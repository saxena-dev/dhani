//! The sealed, crate-private `FeedProtocol` trait.
//!
//! Each feed (market, depth, global, order update) implements it; the single owner task drives
//! any implementation through it.

use std::collections::BTreeSet;
use std::time::Duration;

use secrecy::SecretString;

pub(crate) use tokio_tungstenite::tungstenite::Message;

use crate::decoder::DecodeError;
use crate::labels::FeedKind;

#[allow(dead_code, reason = "implemented by the feed protocols")]
pub(crate) mod sealed {
    /// Only this crate implements [`super::FeedProtocol`].
    pub trait Sealed {}
}

/// One item decoded from a frame.
#[allow(
    dead_code,
    reason = "produced by the feed protocols and consumed by the owner task"
)]
pub(crate) enum Decoded<T> {
    /// A data item.
    Data(T),
    /// A frame or packet that could not be decoded (non-terminal).
    Error(DecodeError),
    /// The server announced a disconnect; `ambiguous` holds both candidate codes when a depth
    /// disconnect packet carries two different plausible ones.
    ServerDisconnect {
        code: Option<u16>,
        ambiguous: Option<(u16, u32)>,
    },
}

/// What a feed contributes to the shared owner loop.
#[allow(
    dead_code,
    reason = "implemented by the feed protocols and driven by the owner task"
)]
pub(crate) trait FeedProtocol: sealed::Sealed + Send + 'static {
    /// The desired-state key, e.g. `(Instrument, Mode)`.
    type Sub: Clone + Ord + Send + std::fmt::Debug + 'static;
    /// The decoded item.
    type Data: Send + 'static;
    /// The label for spans and metrics.
    const FEED: FeedKind;
    /// The connection URL; it carries credentials, so it stays secret.
    fn url(&self) -> &SecretString;
    /// Messages to send right after the handshake (the order-update login); empty otherwise.
    fn on_open(&self) -> Vec<Message>;
    /// The most subscriptions one connection may hold.
    fn capacity(&self) -> usize;
    /// The JSON text frames that turn `sent` into `desired`.
    fn reconcile(&self, sent: &BTreeSet<Self::Sub>, desired: &BTreeSet<Self::Sub>) -> Vec<String>;
    /// Decodes one frame into `out`.
    fn decode(&mut self, frame: &Message, out: &mut Vec<Decoded<Self::Data>>);
    /// The message sent before a clean close, if the feed has one.
    fn disconnect_message(&self) -> Option<String>;
    /// How often the client pings the server, for feeds whose server does not ping.
    fn client_ping_interval(&self) -> Option<Duration>;
    /// The key under which subscriptions count as the same instrument (entries with equal keys
    /// are one subscription in different modes); the identity by default.
    fn key(sub: &Self::Sub) -> Self::Sub {
        sub.clone()
    }
}
