//! The `OrderUpdateFeed` builder and the re-export of `OrderUpdateEvent`.
//!
//! The Live Order Update feed connects to `wss://api-order-update.dhan.co` with no query
//! (DOC:6072) and authenticates with a login message sent right after the handshake: for an
//! individual, the client ID and access token (DOC:6083-6092); for a partner, the partner ID and
//! secret (DOC:6101-6110). Messages are JSON text; an `order_alert` becomes
//! [`OrderUpdateEvent::Order`], anything else [`OrderUpdateEvent::Other`] with the raw message,
//! which may contain the client ID, so treat it as sensitive when logging it.
//!
//! The feed has no subscriptions. Its keepalive is undocumented, so the client pings every 20 s
//! and the default liveness timeout is 45 s.

use std::collections::BTreeSet;
use std::fmt;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};

use super::protocol::{Decoded, FeedProtocol, FeedTypes, Message, sealed};
use super::{FeedBuilder, FeedLimits};
use crate::config::{Environment, Urls};
use crate::credentials::{Credentials, PartnerCredentials};
pub use crate::decoder::OrderUpdateEvent;
use crate::decoder::parse_order_update_frame;
use crate::labels::FeedKind;

/// The client ping interval (the keepalive is undocumented).
const CLIENT_PING: Duration = Duration::from_secs(20);
/// The default liveness timeout for this feed.
const LIVENESS: Duration = Duration::from_secs(45);

/// The Live Order Update feed.
///
/// Dhan allows at most five WebSocket connections per user; opening a sixth disconnects the
/// oldest with code 805 (DOC:6034). This SDK does not count connections across feeds.
///
/// The builders set a 45 s liveness timeout; when passing your own limits, start from the
/// builder's (`liveness_timeout: Duration::from_secs(45)`) rather than `FeedLimits::default()`.
#[derive(Debug)]
pub struct OrderUpdateFeed;

impl OrderUpdateFeed {
    /// A builder for an individual account's order updates.
    pub fn builder(credentials: Credentials) -> FeedBuilder<OrderUpdateProtocol> {
        let login = serde_json::json!({
            "LoginReq": {
                "MsgCode": 42,
                "ClientId": credentials.client_id().expose_secret(),
                "Token": credentials.access_token().expose_secret(),
            },
            "UserType": "SELF",
        });
        Self::with_login(login)
    }

    /// A builder for a partner platform's order updates across its users.
    pub fn partner(credentials: PartnerCredentials) -> FeedBuilder<OrderUpdateProtocol> {
        let login = serde_json::json!({
            "LoginReq": {
                "MsgCode": 42,
                "ClientId": credentials.partner_id.expose_secret(),
            },
            "UserType": "PARTNER",
            "Secret": credentials.partner_secret.expose_secret(),
        });
        Self::with_login(login)
    }

    fn with_login(login: serde_json::Value) -> FeedBuilder<OrderUpdateProtocol> {
        let urls = Urls::for_env(Environment::Live);
        let protocol = OrderUpdateProtocol {
            url: SecretString::from(urls.order_update.as_str()),
            login: SecretString::from(login.to_string()),
        };
        let limits = FeedLimits {
            liveness_timeout: LIVENESS,
            ..FeedLimits::default()
        };
        FeedBuilder::new(protocol, true).limits(limits)
    }
}

/// The wire protocol of the Live Order Update feed.
pub struct OrderUpdateProtocol {
    url: SecretString,
    /// The login message; it holds a credential.
    login: SecretString,
}

impl fmt::Debug for OrderUpdateProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OrderUpdateProtocol")
            .finish_non_exhaustive()
    }
}

impl sealed::Sealed for OrderUpdateProtocol {}

impl FeedTypes for OrderUpdateProtocol {
    type Sub = ();
    type Data = OrderUpdateEvent;
}

impl FeedProtocol for OrderUpdateProtocol {
    const FEED: FeedKind = FeedKind::OrderUpdate;

    fn url(&self) -> &SecretString {
        &self.url
    }

    fn on_open(&self) -> Vec<Message> {
        vec![Message::Text(self.login.expose_secret().to_owned().into())]
    }

    fn capacity(&self) -> usize {
        0
    }

    fn reconcile(&self, _sent: &BTreeSet<()>, _desired: &BTreeSet<()>) -> Vec<String> {
        Vec::new()
    }

    fn decode(&mut self, frame: &Message, out: &mut Vec<Decoded<OrderUpdateEvent>>) {
        let result = match frame {
            Message::Text(text) => parse_order_update_frame(false, text.as_bytes()),
            Message::Binary(bytes) => parse_order_update_frame(true, bytes),
            _ => return,
        };
        out.push(match result {
            Ok(event) => Decoded::Data(event),
            Err(error) => Decoded::Error(error),
        });
    }

    fn disconnect_message(&self) -> Option<String> {
        None
    }

    fn client_ping_interval(&self) -> Option<Duration> {
        Some(CLIENT_PING)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::{AccessToken, ClientId, PartnerId, PartnerSecret};
    use crate::decoder::DecodeErrorKind;

    const TOKEN: &str = "eyJhbGciOiJIUzUxMiJ9.eyJzdWIiOiI5OTk5ODg4ODc3In0.U0VOVElORUwtU0lH";

    fn login(protocol: &OrderUpdateProtocol) -> serde_json::Value {
        let messages = protocol.on_open();
        assert_eq!(messages.len(), 1);
        match &messages[0] {
            Message::Text(text) => serde_json::from_str(text.as_str()).unwrap(),
            other => panic!("{other:?}"),
        }
    }

    fn individual() -> OrderUpdateProtocol {
        let credentials = Credentials::new(
            ClientId::new("9999888877").unwrap(),
            AccessToken::new(TOKEN).unwrap(),
        );
        OrderUpdateFeed::builder(credentials).into_protocol_for_tests()
    }

    #[test]
    fn the_individual_login_message() {
        assert_eq!(
            login(&individual()),
            serde_json::json!({
                "LoginReq": {"MsgCode": 42, "ClientId": "9999888877", "Token": TOKEN},
                "UserType": "SELF"
            })
        );
    }

    #[test]
    fn the_partner_login_message() {
        let credentials = PartnerCredentials {
            partner_id: PartnerId::new("PARTNER-7").unwrap(),
            partner_secret: PartnerSecret::new("S3CRET-7").unwrap(),
        };
        let protocol = OrderUpdateFeed::partner(credentials).into_protocol_for_tests();
        assert_eq!(
            login(&protocol),
            serde_json::json!({
                "LoginReq": {"MsgCode": 42, "ClientId": "PARTNER-7"},
                "UserType": "PARTNER",
                "Secret": "S3CRET-7"
            })
        );
    }

    #[test]
    fn keepalive_and_subscriptions() {
        let p = individual();
        assert_eq!(p.client_ping_interval(), Some(Duration::from_secs(20)));
        assert_eq!((p.capacity(), p.disconnect_message()), (0, None));
        assert!(p.reconcile(&BTreeSet::new(), &BTreeSet::new()).is_empty());
        assert_eq!(p.url().expose_secret(), "wss://api-order-update.dhan.co/");
        assert_eq!(format!("{p:?}"), "OrderUpdateProtocol { .. }");
    }

    #[test]
    fn frames_decode_to_order_events() {
        let mut p = individual();
        let mut out = Vec::new();
        p.decode(&Message::Binary(vec![1, 2, 3].into()), &mut out);
        p.decode(
            &Message::Text(r#"{"Type":"order_alert","Data":{"OrderNo":"1124091136546"}}"#.into()),
            &mut out,
        );
        p.decode(&Message::Text(r#"{"Type":"hello"}"#.into()), &mut out);
        p.decode(&Message::Text("not json".into()), &mut out);
        assert_eq!(out.len(), 4);
        assert!(
            matches!(&out[0], Decoded::Error(e) if e.kind == DecodeErrorKind::UnexpectedBinary)
        );
        assert!(
            matches!(&out[1], Decoded::Data(OrderUpdateEvent::Order(o)) if o.order_no.as_deref() == Some("1124091136546"))
        );
        assert!(
            matches!(&out[2], Decoded::Data(OrderUpdateEvent::Other { kind: Some(k), .. }) if k == "hello")
        );
        assert!(matches!(&out[3], Decoded::Error(e) if e.kind == DecodeErrorKind::Json));
    }

    #[test]
    fn the_builder_defaults_to_a_45_second_liveness_timeout() {
        let limits = OrderUpdateFeed::builder(Credentials::new(
            ClientId::new("9999888877").unwrap(),
            AccessToken::new(TOKEN).unwrap(),
        ))
        .limits_for_tests();
        assert_eq!(limits.liveness_timeout, Duration::from_secs(45));
    }
}
