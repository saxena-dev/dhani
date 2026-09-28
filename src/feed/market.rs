//! The Live Market Feed: `MarketFeed` builder, `Instrument`, `Mode`, `MarketEvent`.
//!
//! The feed connects to `wss://api-feed.dhan.co?version=2&token=…&clientId=…&authType=2`
//! (DOC:5859-5869), subscribes with JSON requests of at most 100 instruments each and up to 5000
//! instruments per connection (DOC:5873-5893, DOC:5843), and receives binary packets decoded by
//! [`crate::decoder::split_market`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};

use super::handle::FeedHandle;
use super::protocol::{Decoded, FeedProtocol, FeedTypes, Message, sealed};
use super::{CommandError, FeedBuilder, Revision, SubscriptionCommand};
use crate::config::{Environment, Urls};
use crate::credentials::Credentials;
use crate::decoder::{DecodeError, DecodeErrorKind, MarketPacket, split_market};
use crate::error::{ValidationError, ValidationReason};
use crate::labels::FeedKind;
use crate::obs::metrics::{self, PacketKind};
use crate::types::{ExchangeSegment, SecurityId, WireEnum};

/// Instruments per subscription message (DOC:5875).
const PER_MESSAGE: usize = 100;
/// Instruments per connection (DOC:5873).
const PER_CONNECTION: usize = 5000;

/// A market-feed instrument: an exchange segment and a numeric security ID.
///
/// The ID is kept in canonical form (no leading zeros), so `"01333"` and `"1333"` are the same
/// instrument for equality, hashing and ordering alike.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Instrument {
    segment: ExchangeSegment,
    security_id: SecurityId,
    /// The numeric value of the ID.
    id: u32,
}

impl Instrument {
    /// An instrument; the feed identifies instruments by numeric security IDs.
    pub fn new(segment: ExchangeSegment, security_id: SecurityId) -> Result<Self, ValidationError> {
        let invalid = || ValidationError::new("security_id", ValidationReason::InvalidCharacters);
        let id = security_id.as_numeric().ok_or_else(invalid)?;
        let security_id = SecurityId::new(id.to_string()).map_err(|_| invalid())?;
        Ok(Instrument {
            segment,
            security_id,
            id,
        })
    }

    /// The exchange segment.
    pub fn segment(&self) -> ExchangeSegment {
        self.segment
    }

    /// The security ID, in canonical form.
    pub fn security_id(&self) -> &SecurityId {
        &self.security_id
    }
}

/// Instruments are ordered by segment wire string, then numeric security ID, so subscription
/// messages are deterministic.
impl Ord for Instrument {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.segment.as_wire(), self.id).cmp(&(other.segment.as_wire(), other.id))
    }
}

impl PartialOrd for Instrument {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// How much data a subscription receives.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mode {
    /// Last traded price and time (request codes 15/16, DOC:4191-4204).
    Ticker,
    /// Quote with volume and OHLC (codes 17/18).
    Quote,
    /// Quote, open interest and five depth levels (codes 21/22).
    Full,
}

impl Mode {
    /// Subscription modes in their wire order.
    const ALL: [Mode; 3] = [Mode::Ticker, Mode::Quote, Mode::Full];

    fn subscribe_code(self) -> u8 {
        match self {
            Mode::Ticker => 15,
            Mode::Quote => 17,
            Mode::Full => 21,
        }
    }

    fn unsubscribe_code(self) -> u8 {
        match self {
            Mode::Ticker => 16,
            Mode::Quote => 18,
            Mode::Full => 22,
        }
    }
}

/// A market-feed subscription: one instrument in one mode.
pub type MarketSub = (Instrument, Mode);

/// A decoded market-feed item.
pub type MarketEvent = MarketPacket;

/// The Live Market Feed.
///
/// Dhan allows at most five WebSocket connections per user; opening a sixth disconnects the
/// oldest with code 805 (DOC:6034). This SDK does not count connections across feeds, so keep
/// within the limit yourself; an 805 disconnect ends the feed rather than reconnecting.
#[derive(Debug)]
pub struct MarketFeed;

impl MarketFeed {
    /// A builder for the production feed with `credentials`. At most five WebSocket connections
    /// per user are allowed; a sixth disconnects the oldest with code 805 (see [`MarketFeed`]).
    ///
    /// The feed URL carries the access token: never enable TRACE logging for the `tungstenite`
    /// target (see [the logging note](crate::feed#logging)).
    pub fn builder(credentials: Credentials) -> FeedBuilder<MarketProtocol> {
        let urls = Urls::for_env(Environment::Live);
        FeedBuilder::new(MarketProtocol::new(&credentials, &urls.market_feed), true)
    }
}

/// The wire protocol of the Live Market Feed.
pub struct MarketProtocol {
    url: SecretString,
    token: SecretString,
    client_id: SecretString,
}

impl fmt::Debug for MarketProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MarketProtocol").finish_non_exhaustive()
    }
}

/// Percent-encodes a query value: everything but ASCII letters, digits and `-._~` (so a space is
/// `%20`).
fn encode_query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl MarketProtocol {
    fn new(credentials: &Credentials, base: &url::Url) -> Self {
        let token = SecretString::from(credentials.access_token().expose_secret());
        let client_id = SecretString::from(credentials.client_id().expose_secret());
        let url = Self::url_with(base, &token, &client_id);
        MarketProtocol {
            url,
            token,
            client_id,
        }
    }

    /// `base` with the version, token, client ID and auth type in the query (DOC:5859-5869).
    fn url_with(base: &url::Url, token: &SecretString, client_id: &SecretString) -> SecretString {
        // The query must precede any fragment, so a fragment is dropped.
        let mut base = base.clone();
        base.set_fragment(None);
        let base = base.as_str();
        let separator = if base.contains('?') { '&' } else { '?' };
        SecretString::from(format!(
            "{base}{separator}version=2&token={}&clientId={}&authType=2",
            encode_query_value(token.expose_secret()),
            encode_query_value(client_id.expose_secret()),
        ))
    }

    /// The connection URL (test support).
    #[cfg(test)]
    pub(crate) fn url_text(&self) -> &str {
        self.url.expose_secret()
    }

    fn message(code: u8, instruments: &[&Instrument]) -> String {
        let list: Vec<serde_json::Value> = instruments
            .iter()
            .map(|i| {
                serde_json::json!({
                    "ExchangeSegment": i.segment.as_wire(),
                    "SecurityId": i.security_id.as_ref(),
                })
            })
            .collect();
        serde_json::json!({
            "RequestCode": code,
            "InstrumentCount": list.len(),
            "InstrumentList": list,
        })
        .to_string()
    }
}

impl sealed::Sealed for MarketProtocol {}

impl FeedTypes for MarketProtocol {
    type Sub = MarketSub;
    type Data = MarketEvent;
}

impl FeedProtocol for MarketProtocol {
    const FEED: FeedKind = FeedKind::Market;

    fn url(&self) -> &SecretString {
        &self.url
    }

    fn url_for(&self, base: &url::Url) -> SecretString {
        Self::url_with(base, &self.token, &self.client_id)
    }

    fn on_open(&self) -> Vec<Message> {
        Vec::new()
    }

    fn capacity(&self) -> usize {
        PER_CONNECTION
    }

    /// Unsubscribes removed and mode-changed instruments by their old mode (16/18/22), then
    /// subscribes added and mode-changed instruments by their new mode (15, 17, 21), in chunks of
    /// 100. A mode change is therefore an unsubscribe followed by a subscribe (the documentation
    /// does not say whether subscribing in a new mode replaces the old one).
    fn reconcile(&self, sent: &BTreeSet<MarketSub>, desired: &BTreeSet<MarketSub>) -> Vec<String> {
        let sent: BTreeMap<&Instrument, Mode> = sent.iter().map(|(i, m)| (i, *m)).collect();
        let desired: BTreeMap<&Instrument, Mode> = desired.iter().map(|(i, m)| (i, *m)).collect();
        let mut out = Vec::new();
        for mode in Mode::ALL {
            let leaving: Vec<&Instrument> = sent
                .iter()
                .filter(|(i, m)| **m == mode && desired.get(*i) != Some(m))
                .map(|(i, _)| *i)
                .collect();
            for chunk in leaving.chunks(PER_MESSAGE) {
                out.push(Self::message(mode.unsubscribe_code(), chunk));
            }
        }
        for mode in Mode::ALL {
            let joining: Vec<&Instrument> = desired
                .iter()
                .filter(|(i, m)| **m == mode && sent.get(*i) != Some(m))
                .map(|(i, _)| *i)
                .collect();
            for chunk in joining.chunks(PER_MESSAGE) {
                out.push(Self::message(mode.subscribe_code(), chunk));
            }
        }
        out
    }

    fn decode(&mut self, frame: &Message, out: &mut Vec<Decoded<MarketEvent>>) {
        let bytes = match frame {
            Message::Binary(bytes) => bytes,
            _ => {
                out.push(Decoded::Error(DecodeError {
                    kind: DecodeErrorKind::UnexpectedText,
                    offset: 0,
                    packet_code: None,
                }));
                return;
            }
        };
        for item in split_market(bytes) {
            match item {
                Ok(MarketPacket::Disconnect(d)) => {
                    metrics::record_ws_packet(Self::FEED, PacketKind::Disconnect);
                    out.push(Decoded::ServerDisconnect {
                        code: Some(d.reason_code),
                        ambiguous: None,
                    });
                    // Nothing after a disconnect packet is meaningful.
                    return;
                }
                Ok(packet) => {
                    metrics::record_ws_packet(Self::FEED, packet_kind(&packet));
                    out.push(Decoded::Data(packet));
                }
                Err(e) => out.push(Decoded::Error(e)),
            }
        }
    }

    fn disconnect_message(&self) -> Option<String> {
        Some(serde_json::json!({"RequestCode": 12}).to_string())
    }

    fn client_ping_interval(&self) -> Option<Duration> {
        None
    }

    fn key(sub: &MarketSub) -> MarketSub {
        (sub.0.clone(), Mode::Ticker)
    }
}

fn packet_kind(packet: &MarketPacket) -> PacketKind {
    match packet {
        MarketPacket::Ticker(_) => PacketKind::Ticker,
        MarketPacket::PrevClose(_) => PacketKind::PrevClose,
        MarketPacket::Quote(_) => PacketKind::Quote,
        MarketPacket::OpenInterest(_) => PacketKind::OpenInterest,
        MarketPacket::Full(_) => PacketKind::Full,
        MarketPacket::Disconnect(_) => PacketKind::Disconnect,
        _ => PacketKind::Other,
    }
}

impl FeedHandle<MarketSub> {
    /// Subscribes `instruments` in `mode`; an instrument already subscribed changes mode.
    pub async fn subscribe(
        &self,
        instruments: impl IntoIterator<Item = Instrument>,
        mode: Mode,
    ) -> Result<Revision, CommandError> {
        let subs = instruments.into_iter().map(|i| (i, mode)).collect();
        self.command(SubscriptionCommand::Subscribe(subs)).await
    }

    /// Unsubscribes `instruments`, whatever their mode.
    pub async fn unsubscribe(
        &self,
        instruments: impl IntoIterator<Item = Instrument>,
    ) -> Result<Revision, CommandError> {
        let subs = instruments.into_iter().map(|i| (i, Mode::Ticker)).collect();
        self.command(SubscriptionCommand::Unsubscribe(subs)).await
    }

    /// Changes the mode of instruments already subscribed.
    pub async fn set_mode(
        &self,
        instruments: impl IntoIterator<Item = Instrument>,
        mode: Mode,
    ) -> Result<Revision, CommandError> {
        let subs = instruments.into_iter().map(|i| (i, mode)).collect();
        self.command(SubscriptionCommand::SetMode(subs)).await
    }

    /// Replaces every subscription with `pairs`.
    pub async fn replace(
        &self,
        pairs: impl IntoIterator<Item = MarketSub>,
    ) -> Result<Revision, CommandError> {
        self.command(SubscriptionCommand::Replace(pairs.into_iter().collect()))
            .await
    }
}

#[cfg(test)]
#[path = "market_tests.rs"]
mod tests;
