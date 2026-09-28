use super::*;
use crate::credentials::{AccessToken, ClientId};

fn instrument(segment: ExchangeSegment, id: &str) -> Instrument {
    Instrument::new(segment, SecurityId::new(id).unwrap()).unwrap()
}

fn protocol() -> MarketProtocol {
    let credentials = Credentials::new(
        ClientId::new("9999888877").unwrap(),
        AccessToken::new("eyJ0eXAi.eyJzdWIi.c2ln").unwrap(),
    );
    MarketProtocol::new(
        &credentials,
        &url::Url::parse("wss://api-feed.dhan.co").unwrap(),
    )
}

fn set(subs: &[(Instrument, Mode)]) -> BTreeSet<MarketSub> {
    subs.iter().cloned().collect()
}

/// `(RequestCode, [(ExchangeSegment, SecurityId)])` of each message.
fn parsed(messages: &[String]) -> Vec<(u64, Vec<(String, String)>)> {
    messages
        .iter()
        .map(|m| {
            let v: serde_json::Value = serde_json::from_str(m).unwrap();
            let list = v["InstrumentList"].as_array().unwrap();
            assert_eq!(v["InstrumentCount"].as_u64().unwrap() as usize, list.len());
            let items = list
                .iter()
                .map(|i| {
                    (
                        i["ExchangeSegment"].as_str().unwrap().to_owned(),
                        // Security IDs are sent as strings.
                        i["SecurityId"].as_str().expect("a string id").to_owned(),
                    )
                })
                .collect();
            (v["RequestCode"].as_u64().unwrap(), items)
        })
        .collect()
}

#[test]
fn a_hundred_and_fifty_full_instruments_take_two_messages() {
    let desired: Vec<_> = (1..=150)
        .map(|n| {
            (
                instrument(ExchangeSegment::NseEq, &n.to_string()),
                Mode::Full,
            )
        })
        .collect();
    let messages = parsed(&protocol().reconcile(&BTreeSet::new(), &set(&desired)));
    assert_eq!(messages.len(), 2);
    assert_eq!((messages[0].0, messages[0].1.len()), (21, 100));
    assert_eq!((messages[1].0, messages[1].1.len()), (21, 50));
    assert_eq!(messages[0].1[0], ("NSE_EQ".to_owned(), "1".to_owned()));
    // Numeric order, not string order: 99 comes before 100.
    assert_eq!(messages[0].1[98].1, "99");
    assert_eq!(messages[0].1[99].1, "100");
}

#[test]
fn a_mode_change_unsubscribes_the_old_mode_then_subscribes_the_new() {
    let a = instrument(ExchangeSegment::NseEq, "1333");
    let sent = set(&[(a.clone(), Mode::Ticker)]);
    let desired = set(&[(a, Mode::Quote)]);
    let messages = parsed(&protocol().reconcile(&sent, &desired));
    let one = vec![("NSE_EQ".to_owned(), "1333".to_owned())];
    assert_eq!(messages, [(16, one.clone()), (17, one)]);
}

#[test]
fn removing_one_of_two_sends_a_single_unsubscribe() {
    let a = instrument(ExchangeSegment::NseEq, "1333");
    let b = instrument(ExchangeSegment::BseEq, "532540");
    let sent = set(&[(a.clone(), Mode::Ticker), (b, Mode::Ticker)]);
    let desired = set(&[(a, Mode::Ticker)]);
    let messages = parsed(&protocol().reconcile(&sent, &desired));
    assert_eq!(
        messages,
        [(16, vec![("BSE_EQ".to_owned(), "532540".to_owned())])]
    );
}

#[test]
fn an_unchanged_set_sends_nothing() {
    let s = set(&[(instrument(ExchangeSegment::NseEq, "1333"), Mode::Full)]);
    assert!(protocol().reconcile(&s, &s).is_empty());
}

#[test]
fn messages_are_ordered_by_segment_then_numeric_id() {
    let desired = set(&[
        (instrument(ExchangeSegment::NseFno, "49081"), Mode::Ticker),
        (instrument(ExchangeSegment::BseEq, "532540"), Mode::Ticker),
        (instrument(ExchangeSegment::NseEq, "22"), Mode::Ticker),
        (instrument(ExchangeSegment::NseEq, "1333"), Mode::Ticker),
        (instrument(ExchangeSegment::NseEq, "3"), Mode::Full),
    ]);
    let messages = parsed(&protocol().reconcile(&BTreeSet::new(), &desired));
    let ids = |m: &(u64, Vec<(String, String)>)| {
        m.1.iter()
            .map(|(s, i)| format!("{s}:{i}"))
            .collect::<Vec<_>>()
    };
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].0, 15);
    assert_eq!(
        ids(&messages[0]),
        ["BSE_EQ:532540", "NSE_EQ:22", "NSE_EQ:1333", "NSE_FNO:49081"]
    );
    assert_eq!(
        (messages[1].0, ids(&messages[1])),
        (21, vec!["NSE_EQ:3".to_owned()])
    );
}

#[test]
fn the_url_carries_percent_encoded_credentials_and_debug_hides_them() {
    let token: AccessToken = serde_json::from_str("\"fake tok\"").unwrap();
    let credentials = Credentials::new(ClientId::new("9999888877").unwrap(), token);
    let p = MarketProtocol::new(
        &credentials,
        &url::Url::parse("wss://api-feed.dhan.co").unwrap(),
    );
    assert!(
        p.url_text()
            .ends_with("?version=2&token=fake%20tok&clientId=9999888877&authType=2"),
        "{}",
        p.url_text()
    );
    assert_eq!(format!("{p:?}"), "MarketProtocol { .. }");
    // An override base gets the same credentials.
    let local = url::Url::parse("ws://127.0.0.1:9000/feed").unwrap();
    assert_eq!(
        p.url_for(&local).expose_secret(),
        "ws://127.0.0.1:9000/feed?version=2&token=fake%20tok&clientId=9999888877&authType=2"
    );
}

#[test]
fn instruments_need_numeric_ids() {
    let err =
        Instrument::new(ExchangeSegment::NseEq, SecurityId::new("AAPL").unwrap()).unwrap_err();
    assert_eq!(
        err,
        ValidationError::new("security_id", ValidationReason::InvalidCharacters)
    );
}

#[test]
fn decoding_maps_packets_and_the_disconnect() {
    let mut p = protocol();
    // A ticker then a disconnect (code 805) in one frame, then trailing bytes that are ignored.
    let mut frame = vec![2u8, 16, 0, 1];
    frame.extend_from_slice(&1333u32.to_le_bytes());
    frame.extend_from_slice(&1642.5f32.to_le_bytes());
    frame.extend_from_slice(&1_726_048_169u32.to_le_bytes());
    frame.extend_from_slice(&[50, 10, 0, 1]);
    frame.extend_from_slice(&1333u32.to_le_bytes());
    frame.extend_from_slice(&805u16.to_le_bytes());
    frame.extend_from_slice(&[2, 16, 0]);
    let mut out = Vec::new();
    p.decode(&Message::Binary(frame.into()), &mut out);
    assert_eq!(out.len(), 2);
    assert!(matches!(&out[0], Decoded::Data(MarketPacket::Ticker(t)) if t.ltp == 1642.5));
    assert!(matches!(
        out[1],
        Decoded::ServerDisconnect {
            code: Some(805),
            ambiguous: None
        }
    ));
    let mut out = Vec::new();
    p.decode(&Message::Text("{}".into()), &mut out);
    assert!(matches!(&out[..], [Decoded::Error(e)] if e.kind == DecodeErrorKind::UnexpectedText));
}

#[test]
fn the_protocol_constants() {
    let p = protocol();
    assert_eq!(p.capacity(), 5000);
    assert_eq!(
        p.disconnect_message().as_deref(),
        Some(r#"{"RequestCode":12}"#)
    );
    assert_eq!(p.client_ping_interval(), None);
    assert!(p.on_open().is_empty());
    let a = instrument(ExchangeSegment::NseEq, "1333");
    assert_eq!(
        MarketProtocol::key(&(a.clone(), Mode::Full)),
        (a, Mode::Ticker)
    );
}

#[test]
fn leading_zeros_do_not_make_a_different_instrument() {
    let padded = instrument(ExchangeSegment::NseEq, "01333");
    let plain = instrument(ExchangeSegment::NseEq, "1333");
    assert_eq!(padded, plain);
    assert_eq!(padded.cmp(&plain), std::cmp::Ordering::Equal);
    assert_eq!(padded.security_id().as_ref(), "1333");
    assert_eq!(padded.segment(), ExchangeSegment::NseEq);
    // A mode change between the two spellings is a real change on the wire.
    let sent = set(&[(padded, Mode::Ticker)]);
    let desired = set(&[(plain, Mode::Full)]);
    let codes: Vec<u64> = parsed(&protocol().reconcile(&sent, &desired))
        .iter()
        .map(|m| m.0)
        .collect();
    assert_eq!(codes, [16, 21]);
}

#[test]
fn a_fragment_on_an_override_base_is_dropped() {
    let p = protocol();
    let base = url::Url::parse("ws://127.0.0.1:9000/feed#x").unwrap();
    assert!(!p.url_for(&base).expose_secret().contains('#'));
}
