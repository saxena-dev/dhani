use super::*;

/// A header with `code`, header length `len`, NSE_EQ (feed code 1) and security ID 1333.
fn header(code: u8, len: u16) -> Vec<u8> {
    let mut b = vec![code];
    b.extend_from_slice(&len.to_le_bytes());
    b.push(1);
    b.extend_from_slice(&1333u32.to_le_bytes());
    b
}

fn put_f32(b: &mut Vec<u8>, v: f32) {
    b.extend_from_slice(&v.to_le_bytes());
}

fn put_u32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}

fn put_u16(b: &mut Vec<u8>, v: u16) {
    b.extend_from_slice(&v.to_le_bytes());
}

fn ticker_bytes() -> Vec<u8> {
    let mut b = header(2, 16);
    put_f32(&mut b, 1234.5);
    put_u32(&mut b, 1_700_000_000);
    b
}

fn quote_bytes() -> Vec<u8> {
    let mut b = header(4, 50);
    put_f32(&mut b, 101.25);
    put_u16(&mut b, 75);
    put_u32(&mut b, 1_700_000_100);
    put_f32(&mut b, 100.5);
    for v in [900_000u32, 12_000, 13_000] {
        put_u32(&mut b, v);
    }
    for v in [99.0f32, 98.5, 102.0, 97.25] {
        put_f32(&mut b, v);
    }
    b
}

fn full_bytes() -> Vec<u8> {
    let mut b = header(8, 162);
    put_f32(&mut b, 250.75);
    put_u16(&mut b, 10);
    put_u32(&mut b, 1_700_000_200);
    put_f32(&mut b, 249.5);
    for v in [5_000_000u32, 40_000, 41_000, 700_000, 710_000, 690_000] {
        put_u32(&mut b, v);
    }
    for v in [248.0f32, 247.0, 252.0, 246.5] {
        put_f32(&mut b, v);
    }
    for i in 0..5u32 {
        put_u32(&mut b, 100 + i);
        put_u32(&mut b, 200 + i);
        put_u16(&mut b, (10 + i) as u16);
        put_u16(&mut b, (20 + i) as u16);
        put_f32(&mut b, 250.0 - i as f32);
        put_f32(&mut b, 251.0 + i as f32);
    }
    b
}

fn only(frame: &[u8]) -> MarketPacket {
    let mut packets: Vec<_> = split(frame).collect();
    assert_eq!(packets.len(), 1, "{packets:?}");
    packets.remove(0).unwrap()
}

const HEADER: PacketHeader = PacketHeader {
    code: 0,
    length: 0,
    segment_code: 1,
    security_id: 1333,
};

#[test]
fn a_ticker_decodes() {
    let MarketPacket::Ticker(t) = only(&ticker_bytes()) else {
        panic!()
    };
    assert_eq!(
        t,
        Ticker {
            header: PacketHeader {
                code: 2,
                length: 16,
                ..HEADER
            },
            ltp: 1234.5,
            ltt: 1_700_000_000
        }
    );
    assert_eq!(t.ltt_unix(), 1_700_000_000);
    assert_eq!(t.header.segment(), Some(ExchangeSegment::NseEq));
}

#[test]
fn a_prev_close_decodes() {
    let mut b = header(6, 16);
    put_f32(&mut b, 1199.5);
    put_u32(&mut b, 4_500);
    let MarketPacket::PrevClose(p) = only(&b) else {
        panic!()
    };
    assert_eq!((p.header.code, p.prev_close, p.prev_oi), (6, 1199.5, 4_500));
}

#[test]
fn a_quote_decodes() {
    let MarketPacket::Quote(q) = only(&quote_bytes()) else {
        panic!()
    };
    assert_eq!(
        q,
        Quote {
            header: PacketHeader {
                code: 4,
                length: 50,
                ..HEADER
            },
            ltp: 101.25,
            ltq: 75,
            ltt: 1_700_000_100,
            atp: 100.5,
            volume: 900_000,
            total_sell_qty: 12_000,
            total_buy_qty: 13_000,
            open: 99.0,
            close: 98.5,
            high: 102.0,
            low: 97.25,
        }
    );
}

#[test]
fn an_open_interest_packet_decodes() {
    let mut b = header(5, 12);
    put_u32(&mut b, 123_456);
    assert_eq!(
        only(&b),
        MarketPacket::OpenInterest(OpenInterest {
            header: PacketHeader {
                code: 5,
                length: 12,
                ..HEADER
            },
            oi: 123_456
        })
    );
}

#[test]
fn a_full_packet_decodes_with_five_levels() {
    let MarketPacket::Full(f) = only(&full_bytes()) else {
        panic!()
    };
    assert_eq!(
        (
            f.ltp,
            f.ltq,
            f.ltt,
            f.atp,
            f.volume,
            f.total_sell_qty,
            f.total_buy_qty
        ),
        (250.75, 10, 1_700_000_200, 249.5, 5_000_000, 40_000, 41_000)
    );
    assert_eq!(
        (f.oi, f.oi_day_high, f.oi_day_low),
        (700_000, 710_000, 690_000)
    );
    assert_eq!(
        (f.open, f.close, f.high, f.low),
        (248.0, 247.0, 252.0, 246.5)
    );
    assert_eq!(
        f.depth[0],
        DepthLevel5 {
            bid_qty: 100,
            ask_qty: 200,
            bid_orders: 10,
            ask_orders: 20,
            bid_price: 250.0,
            ask_price: 251.0
        }
    );
    assert_eq!(
        f.depth[4],
        DepthLevel5 {
            bid_qty: 104,
            ask_qty: 204,
            bid_orders: 14,
            ask_orders: 24,
            bid_price: 246.0,
            ask_price: 255.0
        }
    );
}

#[test]
fn a_disconnect_decodes() {
    let mut b = header(50, 10);
    put_u16(&mut b, 805);
    let MarketPacket::Disconnect(d) = only(&b) else {
        panic!()
    };
    assert_eq!(d.reason_code, 805);
}

#[test]
fn a_short_ticker_is_truncated() {
    let b = &ticker_bytes()[..15];
    let got: Vec<_> = split(b).collect();
    assert_eq!(
        got,
        [Err(DecodeError {
            kind: DecodeErrorKind::Truncated,
            offset: 0,
            packet_code: Some(2)
        })]
    );
}

fn index_bytes() -> Vec<u8> {
    let mut b = header(1, 32);
    for v in [22_150.5f32, 22_000.0, 21_990.0, 22_200.0, 21_950.25] {
        put_f32(&mut b, v);
    }
    put_u32(&mut b, 1_700_000_300);
    b
}

#[test]
fn an_index_packet_is_other_with_a_legacy_reading() {
    let MarketPacket::Other(other) = only(&index_bytes()) else {
        panic!()
    };
    assert_eq!((other.header.code, other.payload.len()), (1, 24));
    assert_eq!(
        other.as_legacy_index(),
        Some(LegacyIndex {
            value: 22_150.5,
            open: 22_000.0,
            close: 21_990.0,
            high: 22_200.0,
            low: 21_950.25,
            updated: 1_700_000_300
        })
    );
    assert_eq!(
        format!("{other:?}"),
        "OtherPacket { header: PacketHeader { code: 1, length: 32, segment_code: 1, security_id: 1333 }, payload_len: 24 }"
    );
    // Too short, or another code: no legacy reading.
    let short = OtherPacket {
        header: other.header,
        payload: vec![0; 23],
    };
    assert_eq!(short.as_legacy_index(), None);
    let status = OtherPacket {
        header: PacketHeader {
            code: 7,
            ..other.header
        },
        payload: other.payload.clone(),
    };
    assert_eq!(status.as_legacy_index(), None);
}

#[test]
fn an_unknown_code_after_other_abandons_the_frame() {
    let mut frame = index_bytes();
    frame.extend(header(99, 8));
    let got: Vec<_> = split(&frame).collect();
    assert_eq!(got.len(), 2);
    assert!(matches!(got[0], Ok(MarketPacket::Other(_))));
    assert_eq!(
        got[1],
        Err(DecodeError {
            kind: DecodeErrorKind::TrailingBytes,
            offset: 32,
            packet_code: Some(99)
        })
    );
}

#[test]
fn an_unknown_code_without_a_usable_length_is_unknown() {
    let mut frame = header(9, 0);
    frame.extend_from_slice(&[0; 8]);
    let got: Vec<_> = split(&frame).collect();
    assert_eq!(
        got,
        [Err(DecodeError {
            kind: DecodeErrorKind::UnknownCode,
            offset: 0,
            packet_code: Some(9)
        })]
    );
}

#[test]
fn a_frame_of_three_packets_yields_three() {
    let mut frame = ticker_bytes();
    frame.extend(quote_bytes());
    frame.extend(full_bytes());
    let got: Vec<_> = split(&frame).map(Result::unwrap).collect();
    assert!(matches!(
        got.as_slice(),
        [
            MarketPacket::Ticker(_),
            MarketPacket::Quote(_),
            MarketPacket::Full(_)
        ]
    ));
}

#[test]
fn a_known_packet_after_other_is_fine_and_a_short_tail_is_truncated() {
    let mut frame = index_bytes();
    frame.extend(ticker_bytes());
    frame.extend_from_slice(&[2, 16, 0]);
    let got: Vec<_> = split(&frame).collect();
    assert_eq!(got.len(), 3);
    assert!(matches!(got[1], Ok(MarketPacket::Ticker(_))));
    assert_eq!(
        got[2],
        Err(DecodeError {
            kind: DecodeErrorKind::Truncated,
            offset: 48,
            packet_code: Some(2)
        })
    );
}

#[test]
fn header_length_mismatches_are_reported_not_fatal() {
    for len in [16u16, 8] {
        let mut b = ticker_bytes();
        b[1..3].copy_from_slice(&len.to_le_bytes());
        let mut s = split(&b);
        assert!(s.next().unwrap().is_ok());
        assert_eq!(s.len_mismatch(), None, "{len}");
    }
    let mut b = ticker_bytes();
    b[1..3].copy_from_slice(&99u16.to_le_bytes());
    let mut s = split(&b);
    assert!(s.next().unwrap().is_ok());
    assert_eq!(
        s.len_mismatch(),
        Some(LenMismatch {
            offset: 0,
            code: 2,
            size: 16,
            header_len: 99
        })
    );
}

#[test]
fn decode_packet_checks_code_and_length() {
    assert_eq!(
        decode_packet(3, &[0; 20]),
        Err(DecodeError {
            kind: DecodeErrorKind::UnknownCode,
            offset: 0,
            packet_code: Some(3)
        })
    );
    assert_eq!(
        decode_packet(8, &[0; 161]),
        Err(DecodeError {
            kind: DecodeErrorKind::Truncated,
            offset: 0,
            packet_code: Some(8)
        })
    );
    assert!(split(&[]).next().is_none());
}

#[test]
fn prev_close_and_disconnect_carry_their_headers() {
    let mut b = header(6, 16);
    put_f32(&mut b, 1199.5);
    put_u32(&mut b, 4_500);
    assert_eq!(
        only(&b),
        MarketPacket::PrevClose(PrevClose {
            header: PacketHeader {
                code: 6,
                length: 16,
                ..HEADER
            },
            prev_close: 1199.5,
            prev_oi: 4_500
        })
    );
    let mut b = header(50, 10);
    put_u16(&mut b, 805);
    assert_eq!(
        only(&b),
        MarketPacket::Disconnect(Disconnect {
            header: PacketHeader {
                code: 50,
                length: 10,
                ..HEADER
            },
            reason_code: 805
        })
    );
}

#[test]
fn a_short_unknown_tail_after_other_is_trailing_bytes() {
    let mut frame = index_bytes();
    frame.extend_from_slice(&[99, 1, 2]);
    let got: Vec<_> = split(&frame).collect();
    assert_eq!(got.len(), 2);
    assert!(matches!(got[0], Ok(MarketPacket::Other(_))));
    assert_eq!(
        got[1],
        Err(DecodeError {
            kind: DecodeErrorKind::TrailingBytes,
            offset: 32,
            packet_code: Some(99)
        })
    );
}

#[test]
fn decode_packet_rejects_a_mismatched_code_byte() {
    assert_eq!(
        decode_packet(2, &full_bytes()),
        Err(DecodeError {
            kind: DecodeErrorKind::UnknownCode,
            offset: 0,
            packet_code: Some(2)
        })
    );
}
