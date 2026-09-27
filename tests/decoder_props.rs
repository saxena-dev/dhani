//! Property campaigns for the binary decoders, with an in-test SplitMix64 (no fuzzing crate).
//!
//! The default seed is `0xD4A1_2026`; `DHANI_FUZZ_SEED` (decimal or `0x` hex) and
//! `DHANI_FUZZ_SCALE` (a multiplier on the case counts, default 1) override it. They are read
//! here only. Every failure message carries the seed, so rerunning with that seed reproduces it.
//!
//! Campaigns (architecture §10.5): (1) random fields round-trip bit-identically; (2) random
//! concatenations decode to the same sequence; (3) every truncation of every fixture is an error,
//! never a panic or a partial packet; (4) random mutations never panic and always terminate;
//! (5) broken length fields end the frame with an error instead of looping. Each campaign is a
//! table of rows, so other feeds add rows rather than tests.

mod support;

use dhani::decoder::{DecodeErrorKind, MarketPacket, split_market};
use support::encode::{self, Instrument, Level5, OiFields, QuoteFields};

/// SplitMix64 (Steele, Lea and Flood), as in the crate's jitter source.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn u32(&mut self) -> u32 {
        (self.next() >> 32) as u32
    }

    fn u16(&mut self) -> u16 {
        (self.next() >> 48) as u16
    }

    fn u8(&mut self) -> u8 {
        (self.next() >> 56) as u8
    }

    /// Any bit pattern, NaNs and infinities included.
    fn f32(&mut self) -> f32 {
        f32::from_bits(self.u32())
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// The campaign seed and case-count multiplier.
struct Config {
    seed: u64,
    scale: usize,
}

fn config() -> Config {
    let seed = std::env::var("DHANI_FUZZ_SEED")
        .ok()
        .map(|s| {
            let digits = s.replace('_', "");
            let parsed = match digits
                .strip_prefix("0x")
                .or_else(|| digits.strip_prefix("0X"))
            {
                Some(hex) => u64::from_str_radix(hex, 16),
                None => digits.parse(),
            };
            parsed.unwrap_or_else(|_| panic!("DHANI_FUZZ_SEED={s:?} is not a number"))
        })
        .unwrap_or(0xD4A1_2026);
    let scale = std::env::var("DHANI_FUZZ_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n: &usize| n > 0)
        .unwrap_or(1);
    println!("decoder_props: DHANI_FUZZ_SEED={seed:#x} DHANI_FUZZ_SCALE={scale}");
    Config { seed, scale }
}

// ---- Rows: one per packet family ----

/// Decodes a frame and re-encodes every packet: `Ok(bytes)` per packet, `Err(kind)` for an
/// error (which ends the frame).
type Roundtrip = fn(&[u8]) -> Vec<Result<Vec<u8>, DecodeErrorKind>>;

/// A packet family: how to generate one random packet and how to decode and re-encode a frame.
struct Row {
    name: &'static str,
    generate: fn(&mut Rng) -> Vec<u8>,
    roundtrip: Roundtrip,
}

fn instrument(r: &mut Rng) -> Instrument {
    Instrument {
        segment_code: r.u8(),
        security_id: r.u32(),
    }
}

fn quote_fields(r: &mut Rng) -> QuoteFields {
    QuoteFields {
        ltp: r.f32(),
        ltq: r.u16(),
        ltt: r.u32(),
        atp: r.f32(),
        volume: r.u32(),
        total_sell_qty: r.u32(),
        total_buy_qty: r.u32(),
        open: r.f32(),
        close: r.f32(),
        high: r.f32(),
        low: r.f32(),
    }
}

fn level5(r: &mut Rng) -> Level5 {
    Level5 {
        bid_qty: r.u32(),
        ask_qty: r.u32(),
        bid_orders: r.u16(),
        ask_orders: r.u16(),
        bid_price: r.f32(),
        ask_price: r.f32(),
    }
}

fn gen_ticker(r: &mut Rng) -> Vec<u8> {
    let at = instrument(r);
    encode::ticker(at, r.f32(), r.u32())
}

fn gen_prev_close(r: &mut Rng) -> Vec<u8> {
    let at = instrument(r);
    encode::prev_close(at, r.f32(), r.u32())
}

fn gen_quote(r: &mut Rng) -> Vec<u8> {
    let at = instrument(r);
    encode::quote(at, quote_fields(r))
}

fn gen_oi(r: &mut Rng) -> Vec<u8> {
    let at = instrument(r);
    encode::open_interest(at, r.u32())
}

fn gen_full(r: &mut Rng) -> Vec<u8> {
    let at = instrument(r);
    let q = quote_fields(r);
    let oi = OiFields {
        oi: r.u32(),
        oi_day_high: r.u32(),
        oi_day_low: r.u32(),
    };
    let depth = [level5(r), level5(r), level5(r), level5(r), level5(r)];
    encode::full(at, q, oi, depth)
}

fn gen_disconnect(r: &mut Rng) -> Vec<u8> {
    let at = instrument(r);
    encode::disconnect(at, r.u16())
}

/// Re-encodes a decoded market packet with the header's own length field.
fn reencode_market(p: &MarketPacket) -> Vec<u8> {
    let at = |h: &dhani::decoder::PacketHeader| Instrument {
        segment_code: h.segment_code,
        security_id: h.security_id,
    };
    let mut bytes = match p {
        MarketPacket::Ticker(t) => encode::ticker(at(&t.header), t.ltp, t.ltt),
        MarketPacket::PrevClose(c) => encode::prev_close(at(&c.header), c.prev_close, c.prev_oi),
        MarketPacket::Quote(q) => encode::quote(
            at(&q.header),
            QuoteFields {
                ltp: q.ltp,
                ltq: q.ltq,
                ltt: q.ltt,
                atp: q.atp,
                volume: q.volume,
                total_sell_qty: q.total_sell_qty,
                total_buy_qty: q.total_buy_qty,
                open: q.open,
                close: q.close,
                high: q.high,
                low: q.low,
            },
        ),
        MarketPacket::OpenInterest(o) => encode::open_interest(at(&o.header), o.oi),
        MarketPacket::Full(f) => encode::full(
            at(&f.header),
            QuoteFields {
                ltp: f.ltp,
                ltq: f.ltq,
                ltt: f.ltt,
                atp: f.atp,
                volume: f.volume,
                total_sell_qty: f.total_sell_qty,
                total_buy_qty: f.total_buy_qty,
                open: f.open,
                close: f.close,
                high: f.high,
                low: f.low,
            },
            OiFields {
                oi: f.oi,
                oi_day_high: f.oi_day_high,
                oi_day_low: f.oi_day_low,
            },
            f.depth.map(|l| Level5 {
                bid_qty: l.bid_qty,
                ask_qty: l.ask_qty,
                bid_orders: l.bid_orders,
                ask_orders: l.ask_orders,
                bid_price: l.bid_price,
                ask_price: l.ask_price,
            }),
        ),
        MarketPacket::Disconnect(d) => encode::disconnect(at(&d.header), d.reason_code),
        MarketPacket::Other(o) => encode::other(o.header.code, at(&o.header), &o.payload),
        other => panic!("unexpected packet {other:?}"),
    };
    // Keep the header's length field exactly as decoded.
    let header = match p {
        MarketPacket::Ticker(t) => t.header,
        MarketPacket::PrevClose(c) => c.header,
        MarketPacket::Quote(q) => q.header,
        MarketPacket::OpenInterest(o) => o.header,
        MarketPacket::Full(f) => f.header,
        MarketPacket::Disconnect(d) => d.header,
        MarketPacket::Other(o) => o.header,
        _ => unreachable!(),
    };
    bytes[1..3].copy_from_slice(&header.length.to_le_bytes());
    bytes
}

fn market_roundtrip(frame: &[u8]) -> Vec<Result<Vec<u8>, DecodeErrorKind>> {
    split_market(frame)
        .map(|r| r.map(|p| reencode_market(&p)).map_err(|e| e.kind))
        .collect()
}

fn rows() -> Vec<Row> {
    let market = |name, generate| Row {
        name,
        generate,
        roundtrip: market_roundtrip,
    };
    vec![
        market("market ticker", gen_ticker),
        market("market prev close", gen_prev_close),
        market("market quote", gen_quote),
        market("market open interest", gen_oi),
        market("market full", gen_full),
        market("market disconnect", gen_disconnect),
    ]
}

// ---- Campaigns ----

#[test]
fn campaign_1_random_fields_round_trip_bit_identically() {
    let cfg = config();
    for row in rows() {
        let mut r = Rng(cfg.seed);
        for case in 0..20_000 * cfg.scale {
            let packet = (row.generate)(&mut r);
            let got = (row.roundtrip)(&packet);
            assert_eq!(
                got,
                vec![Ok(packet.clone())],
                "{} case {case} (seed {:#x})",
                row.name,
                cfg.seed
            );
        }
    }
}

#[test]
fn campaign_2_random_concatenations_decode_to_the_same_sequence() {
    let cfg = config();
    let rows = rows();
    let mut r = Rng(cfg.seed ^ 0x2);
    for case in 0..2_000 * cfg.scale {
        let count = 1 + r.below(50);
        let packets: Vec<Vec<u8>> = (0..count)
            .map(|_| (rows[r.below(rows.len())].generate)(&mut r))
            .collect();
        let frame = packets.concat();
        let got = market_roundtrip(&frame);
        let want: Vec<_> = packets.into_iter().map(Ok).collect();
        assert_eq!(got, want, "case {case} (seed {:#x})", cfg.seed);
    }
}

/// A committed fixture and the decoder that reads it.
struct FixtureRow {
    name: &'static str,
    roundtrip: Roundtrip,
}

fn fixture_rows() -> Vec<FixtureRow> {
    [
        "market_ticker.bin",
        "market_quote.bin",
        "market_full.bin",
        "market_oi.bin",
        "market_prev_close.bin",
        "market_disconnect_805.bin",
        "market_multi.bin",
    ]
    .into_iter()
    .map(|name| FixtureRow {
        name,
        roundtrip: market_roundtrip,
    })
    .collect()
}

#[test]
fn campaign_3_every_truncation_is_an_error_never_a_partial_packet() {
    for row in fixture_rows() {
        let bytes = support::fixtures::raw_bytes(&format!("tests/fixtures/ws/{}", row.name));
        let whole: Vec<Vec<u8>> = (row.roundtrip)(&bytes)
            .into_iter()
            .map(|r| r.unwrap_or_else(|e| panic!("{}: {e:?}", row.name)))
            .collect();
        assert_eq!(whole.concat(), bytes, "{} does not round-trip", row.name);
        // Packet boundaries within the whole fixture.
        let mut ends = Vec::new();
        let mut end = 0;
        for packet in &whole {
            end += packet.len();
            ends.push(end);
        }
        for cut in 1..bytes.len() {
            let got = (row.roundtrip)(&bytes[..cut]);
            let complete = ends.iter().filter(|&&e| e <= cut).count();
            let (oks, rest) = got.split_at(complete.min(got.len()));
            assert_eq!(
                oks.to_vec(),
                whole[..complete]
                    .iter()
                    .cloned()
                    .map(Ok)
                    .collect::<Vec<_>>(),
                "{} cut at {cut}",
                row.name
            );
            if ends.contains(&cut) {
                assert!(
                    rest.is_empty(),
                    "{} cut at a boundary {cut}: {rest:?}",
                    row.name
                );
            } else {
                assert_eq!(rest.len(), 1, "{} cut at {cut}: {rest:?}", row.name);
                assert!(rest[0].is_err(), "{} cut at {cut}: {rest:?}", row.name);
            }
        }
    }
}

#[test]
fn campaign_4_random_mutations_never_panic_and_terminate() {
    let cfg = config();
    let rows = rows();
    let mut r = Rng(cfg.seed ^ 0x4);
    for case in 0..20_000 * cfg.scale {
        let count = 1 + r.below(5);
        let mut frame: Vec<u8> = (0..count)
            .flat_map(|_| (rows[r.below(rows.len())].generate)(&mut r))
            .collect();
        for _ in 0..1 + r.below(8) {
            match r.below(3) {
                0 => {
                    let i = r.below(frame.len());
                    frame[i] = r.u8();
                }
                1 => frame.truncate(r.below(frame.len() + 1)),
                _ => {
                    let extra = r.below(16);
                    frame.extend((0..extra).map(|_| r.u8()));
                }
            }
            if frame.is_empty() {
                break;
            }
        }
        // Every item consumes at least 8 bytes, so a terminating splitter yields at most
        // len / 8 + 1 items.
        let bound = frame.len() / 8 + 1;
        let mut split = split_market(&frame);
        let items = split.by_ref().take(bound + 1).count();
        assert!(
            items <= bound,
            "case {case} (seed {:#x}): {items} items",
            cfg.seed
        );
        assert!(split.next().is_none(), "case {case} (seed {:#x})", cfg.seed);
    }
}

/// A frame with a broken length field and the error that must end it.
struct LengthRow {
    name: &'static str,
    frame: fn() -> Vec<u8>,
    decode: Roundtrip,
    expect: DecodeErrorKind,
}

const EQUITY: Instrument = Instrument {
    segment_code: 1,
    security_id: 1333,
};

/// An undocumented packet (code 1) with its length field set to `len`, padded to 32 bytes.
fn other_with_len(len: u16) -> Vec<u8> {
    let mut b = encode::other(1, EQUITY, &[0; 24]);
    b[1..3].copy_from_slice(&len.to_le_bytes());
    b
}

fn length_rows() -> Vec<LengthRow> {
    vec![
        LengthRow {
            name: "market Other with length 0",
            frame: || other_with_len(0),
            decode: market_roundtrip,
            expect: DecodeErrorKind::UnknownCode,
        },
        LengthRow {
            name: "market Other with length 5, below the header",
            frame: || other_with_len(5),
            decode: market_roundtrip,
            expect: DecodeErrorKind::UnknownCode,
        },
        LengthRow {
            name: "market Other with length past the buffer",
            frame: || other_with_len(u16::MAX),
            decode: market_roundtrip,
            expect: DecodeErrorKind::UnknownCode,
        },
        LengthRow {
            name: "market Other with length 8, then garbage",
            frame: || {
                let mut b = other_with_len(8);
                b[8] = 0xEE;
                b
            },
            decode: market_roundtrip,
            expect: DecodeErrorKind::TrailingBytes,
        },
    ]
}

#[test]
fn campaign_5_broken_lengths_end_the_frame_with_an_error() {
    for row in length_rows() {
        let frame = (row.frame)();
        let got = (row.decode)(&frame);
        let last = got
            .last()
            .unwrap_or_else(|| panic!("{}: no items", row.name));
        assert_eq!(last, &Err(row.expect), "{}: {got:?}", row.name);
        assert!(got.len() <= frame.len() / 8 + 1, "{}: {got:?}", row.name);
    }
}
