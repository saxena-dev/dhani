//! Golden tests for the Live Market Feed decoder, and the synthesised WebSocket fixtures.
//!
//! The field values below are chosen by hand. `tests/support/encode.rs` turns them into packets
//! using only the public layout constants, and the canonical bytes are committed under
//! `tests/fixtures/ws/`. Each golden test checks that the committed file still equals the
//! encoder's output and that decoding it yields exactly the hand-written values.
//!
//! The depth fixtures are produced here too (the decoder for them arrives with the depth feed).
//! To regenerate every file after a deliberate change, run
//! `cargo test --no-default-features --features decoder --test decoder_market -- --ignored
//! write_fixtures`, then update the sha256 values in `tests/fixtures/MANIFEST.toml`.

mod support;

use dhani::decoder::layout::{DEPTH_CODE_ASK, DEPTH_CODE_BID};
use dhani::decoder::{
    DepthLevel5, Disconnect, Full, MarketPacket, OpenInterest, PacketHeader, PrevClose, Quote,
    Ticker, split_market,
};
use support::encode::{self, DepthLevel, Instrument, Level5, OiFields, QuoteFields};

const WS: &str = "tests/fixtures/ws";

/// NSE_EQ (feed code 1), security 1333.
const EQUITY: Instrument = Instrument {
    segment_code: 1,
    security_id: 1333,
};
/// NSE_FNO (feed code 2), security 49081.
const OPTION: Instrument = Instrument {
    segment_code: 2,
    security_id: 49081,
};
/// NSE_FNO, security 49082 (the second depth instrument).
const OPTION_2: Instrument = Instrument {
    segment_code: 2,
    security_id: 49082,
};

const TICKER_LTP: f32 = 1642.35;
const TICKER_LTT: u32 = 1_726_048_169;
const PREV_CLOSE: f32 = 1638.9;
const PREV_OI: u32 = 0;
const OI: u32 = 1_234_500;
const DISCONNECT_CODE: u16 = 805;

const QUOTE: QuoteFields = QuoteFields {
    ltp: 1642.35,
    ltq: 12,
    ltt: 1_726_048_170,
    atp: 1640.8,
    volume: 3_456_789,
    total_sell_qty: 210_000,
    total_buy_qty: 198_500,
    open: 1635.0,
    close: 1638.9,
    high: 1650.5,
    low: 1630.25,
};

const FULL_QUOTE: QuoteFields = QuoteFields {
    ltp: 212.5,
    ltq: 75,
    ltt: 1_726_048_171,
    atp: 208.75,
    volume: 9_876_000,
    total_sell_qty: 1_200_000,
    total_buy_qty: 1_150_000,
    open: 200.0,
    close: 198.5,
    high: 215.25,
    low: 196.0,
};

const FULL_OI: OiFields = OiFields {
    oi: 5_400_000,
    oi_day_high: 5_500_000,
    oi_day_low: 5_100_000,
};

/// Best level first: bids step down and asks step up by 0.05.
fn full_depth() -> [Level5; 5] {
    [
        Level5 {
            bid_qty: 1500,
            ask_qty: 1725,
            bid_orders: 9,
            ask_orders: 11,
            bid_price: 212.45,
            ask_price: 212.5,
        },
        Level5 {
            bid_qty: 3000,
            ask_qty: 2250,
            bid_orders: 14,
            ask_orders: 12,
            bid_price: 212.4,
            ask_price: 212.55,
        },
        Level5 {
            bid_qty: 4500,
            ask_qty: 3750,
            bid_orders: 21,
            ask_orders: 18,
            bid_price: 212.35,
            ask_price: 212.6,
        },
        Level5 {
            bid_qty: 2250,
            ask_qty: 5250,
            bid_orders: 10,
            ask_orders: 25,
            bid_price: 212.3,
            ask_price: 212.65,
        },
        Level5 {
            bid_qty: 750,
            ask_qty: 6000,
            bid_orders: 4,
            ask_orders: 30,
            bid_price: 212.25,
            ask_price: 212.7,
        },
    ]
}

/// `count` depth levels on one side, best first: prices step by `step` from `best`, quantities and
/// order counts grow with the level.
fn depth_levels(best: f64, step: f64, count: usize) -> Vec<DepthLevel> {
    (0..count)
        .map(|i| DepthLevel {
            price: best + step * i as f64,
            quantity: 75 * (i as u32 + 1),
            orders: i as u32 % 7 + 1,
        })
        .collect()
}

/// Every synthesised fixture: file name and bytes.
fn fixtures() -> Vec<(&'static str, Vec<u8>)> {
    let ticker = encode::ticker(EQUITY, TICKER_LTP, TICKER_LTT);
    let quote = encode::quote(EQUITY, QUOTE);
    let full = encode::full(OPTION, FULL_QUOTE, FULL_OI, full_depth());
    vec![
        ("market_ticker.bin", ticker.clone()),
        ("market_quote.bin", quote.clone()),
        ("market_full.bin", full.clone()),
        ("market_oi.bin", encode::open_interest(OPTION, OI)),
        (
            "market_prev_close.bin",
            encode::prev_close(EQUITY, PREV_CLOSE, PREV_OI),
        ),
        (
            "market_disconnect_805.bin",
            encode::disconnect(EQUITY, DISCONNECT_CODE),
        ),
        ("market_multi.bin", encode::frame(&[ticker, quote, full])),
        (
            "depth20_bid_ask_2instr.bin",
            // Instrument 1 bid then ask, then instrument 2 bid then ask (DOC:5595).
            encode::frame(&[
                encode::depth(
                    DEPTH_CODE_BID,
                    OPTION,
                    1,
                    &depth_levels(24500.25, -0.25, 20),
                ),
                encode::depth(DEPTH_CODE_ASK, OPTION, 2, &depth_levels(24500.5, 0.25, 20)),
                encode::depth(DEPTH_CODE_BID, OPTION_2, 3, &depth_levels(310.5, -0.25, 20)),
                encode::depth(DEPTH_CODE_ASK, OPTION_2, 4, &depth_levels(310.75, 0.25, 20)),
            ]),
        ),
        (
            "depth200_rows_37.bin",
            encode::depth(
                DEPTH_CODE_BID,
                OPTION,
                37,
                &depth_levels(24500.25, -0.25, 37),
            ),
        ),
        (
            "depth_disconnect_12_13.bin",
            encode::depth_disconnect_documented(OPTION, 805),
        ),
        (
            "depth_disconnect_8_11.bin",
            encode::depth_disconnect_header(OPTION, 805),
        ),
    ]
}

fn committed(name: &str) -> Vec<u8> {
    support::fixtures::raw_bytes(&format!("{WS}/{name}"))
}

#[test]
#[ignore = "writes the fixture files; run deliberately, then update MANIFEST.toml"]
fn write_fixtures() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(WS);
    for (name, bytes) in fixtures() {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
}

#[test]
fn committed_fixtures_equal_the_encoder_output() {
    for (name, bytes) in fixtures() {
        assert_eq!(committed(name), bytes, "{name}");
    }
}

#[test]
fn fixture_sizes_follow_the_layout() {
    assert_eq!(committed("market_full.bin").len(), 162);
    assert_eq!(committed("market_multi.bin").len(), 16 + 50 + 162);
    assert_eq!(committed("depth20_bid_ask_2instr.bin").len(), 4 * 332);
    let depth200 = committed("depth200_rows_37.bin");
    assert_eq!(u16::from_le_bytes([depth200[0], depth200[1]]), 12 + 16 * 37);
    assert_eq!(depth200.len(), 12 + 16 * 37);
    assert_eq!(committed("depth_disconnect_12_13.bin").len(), 14);
    assert_eq!(committed("depth_disconnect_8_11.bin").len(), 12);
}

fn only(name: &str) -> MarketPacket {
    let bytes = committed(name);
    let mut packets: Vec<_> = split_market(&bytes).collect();
    assert_eq!(packets.len(), 1, "{name}: {packets:?}");
    packets.remove(0).unwrap()
}

fn header(code: u8, length: u16, at: Instrument) -> PacketHeader {
    PacketHeader {
        code,
        length,
        segment_code: at.segment_code,
        security_id: at.security_id,
    }
}

fn expected_quote() -> Quote {
    Quote {
        header: header(4, 50, EQUITY),
        ltp: 1642.35,
        ltq: 12,
        ltt: 1_726_048_170,
        atp: 1640.8,
        volume: 3_456_789,
        total_sell_qty: 210_000,
        total_buy_qty: 198_500,
        open: 1635.0,
        close: 1638.9,
        high: 1650.5,
        low: 1630.25,
    }
}

fn expected_full() -> Full {
    let level = |bid_qty, ask_qty, bid_orders, ask_orders, bid_price, ask_price| DepthLevel5 {
        bid_qty,
        ask_qty,
        bid_orders,
        ask_orders,
        bid_price,
        ask_price,
    };
    Full {
        header: header(8, 162, OPTION),
        ltp: 212.5,
        ltq: 75,
        ltt: 1_726_048_171,
        atp: 208.75,
        volume: 9_876_000,
        total_sell_qty: 1_200_000,
        total_buy_qty: 1_150_000,
        oi: 5_400_000,
        oi_day_high: 5_500_000,
        oi_day_low: 5_100_000,
        open: 200.0,
        close: 198.5,
        high: 215.25,
        low: 196.0,
        depth: [
            level(1500, 1725, 9, 11, 212.45, 212.5),
            level(3000, 2250, 14, 12, 212.4, 212.55),
            level(4500, 3750, 21, 18, 212.35, 212.6),
            level(2250, 5250, 10, 25, 212.3, 212.65),
            level(750, 6000, 4, 30, 212.25, 212.7),
        ],
    }
}

#[test]
fn market_ticker_bin_decodes() {
    assert_eq!(
        only("market_ticker.bin"),
        MarketPacket::Ticker(Ticker {
            header: header(2, 16, EQUITY),
            ltp: 1642.35,
            ltt: 1_726_048_169
        })
    );
}

#[test]
fn market_quote_bin_decodes() {
    assert_eq!(
        only("market_quote.bin"),
        MarketPacket::Quote(expected_quote())
    );
}

#[test]
fn market_full_bin_decodes() {
    assert_eq!(
        only("market_full.bin"),
        MarketPacket::Full(Box::new(expected_full()))
    );
}

#[test]
fn market_oi_bin_decodes() {
    assert_eq!(
        only("market_oi.bin"),
        MarketPacket::OpenInterest(OpenInterest {
            header: header(5, 12, OPTION),
            oi: 1_234_500
        })
    );
}

#[test]
fn market_prev_close_bin_decodes() {
    assert_eq!(
        only("market_prev_close.bin"),
        MarketPacket::PrevClose(PrevClose {
            header: header(6, 16, EQUITY),
            prev_close: 1638.9,
            prev_oi: 0
        })
    );
}

#[test]
fn market_disconnect_805_bin_decodes() {
    assert_eq!(
        only("market_disconnect_805.bin"),
        MarketPacket::Disconnect(Disconnect {
            header: header(50, 10, EQUITY),
            reason_code: 805
        })
    );
}

#[test]
fn market_multi_bin_yields_ticker_quote_full() {
    let bytes = committed("market_multi.bin");
    let packets: Vec<_> = split_market(&bytes).map(Result::unwrap).collect();
    assert_eq!(packets.len(), 3);
    assert_eq!(
        packets[0],
        MarketPacket::Ticker(Ticker {
            header: header(2, 16, EQUITY),
            ltp: 1642.35,
            ltt: 1_726_048_169
        })
    );
    assert_eq!(packets[1], MarketPacket::Quote(expected_quote()));
    assert_eq!(packets[2], MarketPacket::Full(Box::new(expected_full())));
}

#[test]
fn an_undocumented_packet_is_delivered_whole() {
    let payload = [7u8; 24];
    let bytes = encode::other(1, EQUITY, &payload);
    let packets: Vec<_> = split_market(&bytes).map(Result::unwrap).collect();
    match packets.as_slice() {
        [MarketPacket::Other(other)] => {
            assert_eq!(
                (other.header.code, other.payload.as_slice()),
                (1, &payload[..])
            );
        }
        other => panic!("{other:?}"),
    }
}
