//! Binary packet encoder built on `dhani::decoder` layout constants.
//!
//! Every offset and length comes from `dhani::decoder::layout`, so an encoder/decoder round trip
//! checks the constants themselves. [`Packet`] writes little-endian fields into a zeroed buffer;
//! the family helpers fill the headers.

use dhani::decoder::layout::*;

/// A packet under construction: a zeroed buffer of its full length.
#[derive(Clone, Debug)]
pub struct Packet(Vec<u8>);

impl Packet {
    /// A zeroed packet of `len` bytes.
    pub fn zeroed(len: usize) -> Self {
        Packet(vec![0; len])
    }

    fn put(mut self, offset: usize, bytes: &[u8]) -> Self {
        self.0[offset..offset + bytes.len()].copy_from_slice(bytes);
        self
    }

    /// Writes a `u8` at `offset`.
    pub fn u8(self, offset: usize, v: u8) -> Self {
        self.put(offset, &[v])
    }

    /// Writes a little-endian `u16` at `offset`.
    pub fn u16(self, offset: usize, v: u16) -> Self {
        self.put(offset, &v.to_le_bytes())
    }

    /// Writes a little-endian `i16` at `offset`.
    pub fn i16(self, offset: usize, v: i16) -> Self {
        self.put(offset, &v.to_le_bytes())
    }

    /// Writes a little-endian `u32` at `offset`.
    pub fn u32(self, offset: usize, v: u32) -> Self {
        self.put(offset, &v.to_le_bytes())
    }

    /// Writes a little-endian `i32` at `offset`.
    pub fn i32(self, offset: usize, v: i32) -> Self {
        self.put(offset, &v.to_le_bytes())
    }

    /// Writes a little-endian `f32` at `offset`.
    pub fn f32(self, offset: usize, v: f32) -> Self {
        self.put(offset, &v.to_le_bytes())
    }

    /// Writes a little-endian `f64` at `offset`.
    pub fn f64(self, offset: usize, v: f64) -> Self {
        self.put(offset, &v.to_le_bytes())
    }

    /// Copies `bytes` to `offset`.
    pub fn raw(self, offset: usize, bytes: &[u8]) -> Self {
        self.put(offset, bytes)
    }

    /// The finished packet.
    pub fn bytes(self) -> Vec<u8> {
        self.0
    }
}

/// Concatenates packets into one frame.
pub fn frame(packets: &[Vec<u8>]) -> Vec<u8> {
    packets.concat()
}

// ---- Live Market Feed ----

/// Instrument fields of a market-feed header.
#[derive(Clone, Copy, Debug)]
pub struct Instrument {
    pub segment_code: u8,
    pub security_id: u32,
}

/// A market-feed packet of `code` with its header filled in. The length field is the packet's
/// total length, and the packet is sized from the fixed-size table (or `len` for others).
pub fn market(code: u8, len: usize, at: Instrument) -> Packet {
    Packet::zeroed(len)
        .u8(H_CODE, code)
        .u16(H_LEN, u16::try_from(len).unwrap())
        .u8(H_SEGMENT, at.segment_code)
        .u32(H_SECURITY_ID, at.security_id)
}

/// A Ticker packet (code 2).
pub fn ticker(at: Instrument, ltp: f32, ltt: u32) -> Vec<u8> {
    market(CODE_TICKER, TICKER_LEN, at)
        .f32(TICKER_LTP, ltp)
        .u32(TICKER_LTT, ltt)
        .bytes()
}

/// A previous-close packet (code 6).
pub fn prev_close(at: Instrument, prev_close: f32, prev_oi: u32) -> Vec<u8> {
    market(CODE_PREV_CLOSE, PREV_CLOSE_LEN, at)
        .f32(PREV_CLOSE_PRICE, prev_close)
        .u32(PREV_CLOSE_OI, prev_oi)
        .bytes()
}

/// An open-interest packet (code 5).
pub fn open_interest(at: Instrument, oi: u32) -> Vec<u8> {
    market(CODE_OI, OI_LEN, at).u32(OI_VALUE, oi).bytes()
}

/// A market-feed disconnect packet (code 50).
pub fn disconnect(at: Instrument, reason_code: u16) -> Vec<u8> {
    market(CODE_DISCONNECT, DISCONNECT_LEN, at)
        .u16(DISCONNECT_REASON, reason_code)
        .bytes()
}

/// An undocumented-code packet delimited by its length field.
pub fn other(code: u8, at: Instrument, payload: &[u8]) -> Vec<u8> {
    market(code, H_HEADER_LEN + payload.len(), at)
        .raw(H_HEADER_LEN, payload)
        .bytes()
}

/// The quote fields shared by Quote and Full packets.
#[derive(Clone, Copy, Debug)]
pub struct QuoteFields {
    pub ltp: f32,
    pub ltq: u16,
    pub ltt: u32,
    pub atp: f32,
    pub volume: u32,
    pub total_sell_qty: u32,
    pub total_buy_qty: u32,
    pub open: f32,
    pub close: f32,
    pub high: f32,
    pub low: f32,
}

/// A Quote packet (code 4).
pub fn quote(at: Instrument, q: QuoteFields) -> Vec<u8> {
    market(CODE_QUOTE, QUOTE_LEN, at)
        .f32(QUOTE_LTP, q.ltp)
        .u16(QUOTE_LTQ, q.ltq)
        .u32(QUOTE_LTT, q.ltt)
        .f32(QUOTE_ATP, q.atp)
        .u32(QUOTE_VOLUME, q.volume)
        .u32(QUOTE_TOTAL_SELL_QTY, q.total_sell_qty)
        .u32(QUOTE_TOTAL_BUY_QTY, q.total_buy_qty)
        .f32(QUOTE_OPEN, q.open)
        .f32(QUOTE_CLOSE, q.close)
        .f32(QUOTE_HIGH, q.high)
        .f32(QUOTE_LOW, q.low)
        .bytes()
}

/// One market-depth level of a Full packet.
#[derive(Clone, Copy, Debug)]
pub struct Level5 {
    pub bid_qty: u32,
    pub ask_qty: u32,
    pub bid_orders: u16,
    pub ask_orders: u16,
    pub bid_price: f32,
    pub ask_price: f32,
}

/// The open-interest fields of a Full packet.
#[derive(Clone, Copy, Debug)]
pub struct OiFields {
    pub oi: u32,
    pub oi_day_high: u32,
    pub oi_day_low: u32,
}

/// A Full packet (code 8) with five depth levels.
pub fn full(
    at: Instrument,
    q: QuoteFields,
    oi: OiFields,
    depth: [Level5; DEPTH5_LEVELS],
) -> Vec<u8> {
    let mut p = market(CODE_FULL, FULL_LEN, at)
        .f32(FULL_LTP, q.ltp)
        .u16(FULL_LTQ, q.ltq)
        .u32(FULL_LTT, q.ltt)
        .f32(FULL_ATP, q.atp)
        .u32(FULL_VOLUME, q.volume)
        .u32(FULL_TOTAL_SELL_QTY, q.total_sell_qty)
        .u32(FULL_TOTAL_BUY_QTY, q.total_buy_qty)
        .u32(FULL_OI, oi.oi)
        .u32(FULL_OI_DAY_HIGH, oi.oi_day_high)
        .u32(FULL_OI_DAY_LOW, oi.oi_day_low)
        .f32(FULL_OPEN, q.open)
        .f32(FULL_CLOSE, q.close)
        .f32(FULL_HIGH, q.high)
        .f32(FULL_LOW, q.low);
    for (i, l) in depth.iter().enumerate() {
        let o = DEPTH5_BASE + DEPTH5_STRIDE * i;
        p = p
            .u32(o + DEPTH5_BID_QTY, l.bid_qty)
            .u32(o + DEPTH5_ASK_QTY, l.ask_qty)
            .u16(o + DEPTH5_BID_ORDERS, l.bid_orders)
            .u16(o + DEPTH5_ASK_ORDERS, l.ask_orders)
            .f32(o + DEPTH5_BID_PRICE, l.bid_price)
            .f32(o + DEPTH5_ASK_PRICE, l.ask_price);
    }
    p.bytes()
}

// ---- Full Market Depth ----

/// One depth level: price, quantity, orders.
#[derive(Clone, Copy, Debug)]
pub struct DepthLevel {
    pub price: f64,
    pub quantity: u32,
    pub orders: u32,
}

/// A depth packet (`code` 41 bid or 51 ask) with `levels`; the length field counts the header.
/// `seq_or_rows` is the 20-level sequence or the 200-level row count.
pub fn depth(code: u8, at: Instrument, seq_or_rows: u32, levels: &[DepthLevel]) -> Vec<u8> {
    let len = D_HEADER_LEN + D_LEVEL_LEN * levels.len();
    let mut p = Packet::zeroed(len)
        .u16(D_LEN, u16::try_from(len).unwrap())
        .u8(D_CODE, code)
        .u8(D_SEGMENT, at.segment_code)
        .u32(D_SECURITY_ID, at.security_id)
        .u32(D_SEQ_OR_ROWS, seq_or_rows);
    for (i, l) in levels.iter().enumerate() {
        let o = D_HEADER_LEN + D_LEVEL_LEN * i;
        p = p
            .f64(o + D_LEVEL_PRICE, l.price)
            .u32(o + D_LEVEL_QUANTITY, l.quantity)
            .u32(o + D_LEVEL_ORDERS, l.orders);
    }
    p.bytes()
}

/// A depth disconnect with the documented `i16` reason at bytes 12-13 (length 14).
pub fn depth_disconnect_documented(at: Instrument, code: i16) -> Vec<u8> {
    let len = D_DISCONNECT_CODE + size_of::<i16>();
    Packet::zeroed(len)
        .u16(D_LEN, u16::try_from(len).unwrap())
        .u8(D_CODE, DEPTH_CODE_DISCONNECT)
        .u8(D_SEGMENT, at.segment_code)
        .u32(D_SECURITY_ID, at.security_id)
        .i16(D_DISCONNECT_CODE, code)
        .bytes()
}

/// A depth disconnect with the reason as a `u32` at bytes 8-11, where the Python SDK reads it
/// (length 12, header only).
pub fn depth_disconnect_header(at: Instrument, code: u32) -> Vec<u8> {
    Packet::zeroed(D_HEADER_LEN)
        .u16(D_LEN, u16::try_from(D_HEADER_LEN).unwrap())
        .u8(D_CODE, DEPTH_CODE_DISCONNECT)
        .u8(D_SEGMENT, at.segment_code)
        .u32(D_SECURITY_ID, at.security_id)
        .u32(D_DISCONNECT_CODE_ALT, code)
        .bytes()
}

// ---- Global Stocks Live Feed ----

/// A global-feed packet of `code` and documented length with its header filled in.
pub fn global(code: u8, exch_seg: u8, scrip_id: u32) -> Packet {
    let len = global_size(code).expect("a documented global message code");
    Packet::zeroed(len)
        .u8(G_EXCH_SEG, exch_seg)
        .u32(G_SCRIP_ID, scrip_id)
        .u32(G_SCRIP_ID2, scrip_id)
        .u8(G_MSG_LENGTH, u8::try_from(len).unwrap())
        .u8(G_MSG_CODE, code)
}

/// A global Trade message (code 1).
pub fn global_trade(
    exch_seg: u8,
    scrip_id: u32,
    ltp: f32,
    volume: u32,
    ltt: u32,
    lut: u32,
) -> Vec<u8> {
    global(G_CODE_TRADE, exch_seg, scrip_id)
        .f32(G_TRADE_LTP, ltp)
        .u32(G_TRADE_VOLUME, volume)
        .u32(G_TRADE_LTT, ltt)
        .u32(G_TRADE_LUT, lut)
        .bytes()
}

/// A global OHLC message (code 3).
pub fn global_ohlc(
    exch_seg: u8,
    scrip_id: u32,
    open: f32,
    close: f32,
    high: f32,
    low: f32,
) -> Vec<u8> {
    global(G_CODE_OHLC, exch_seg, scrip_id)
        .f32(G_OHLC_OPEN, open)
        .f32(G_OHLC_CLOSE, close)
        .f32(G_OHLC_HIGH, high)
        .f32(G_OHLC_LOW, low)
        .bytes()
}

/// A global market-status message (code 29).
pub fn global_market_status(
    exch_seg: u8,
    scrip_id: u32,
    market_type: [u8; 3],
    status: i32,
) -> Vec<u8> {
    global(G_CODE_MARKET_STATUS, exch_seg, scrip_id)
        .raw(G_MARKET_TYPE, &market_type)
        .i32(G_MARKET_STATUS, status)
        .bytes()
}

/// A global previous-close message (code 32).
pub fn global_prev_close(exch_seg: u8, scrip_id: u32, prev_close: f32) -> Vec<u8> {
    global(G_CODE_PREV_CLOSE, exch_seg, scrip_id)
        .f32(G_PREV_CLOSE, prev_close)
        .bytes()
}

/// A global circuit-limit message (code 33).
pub fn global_circuit_limit(exch_seg: u8, scrip_id: u32, upper: f32, lower: f32) -> Vec<u8> {
    global(G_CODE_CIRCUIT_LIMIT, exch_seg, scrip_id)
        .f32(G_CIRCUIT_UPPER, upper)
        .f32(G_CIRCUIT_LOWER, lower)
        .bytes()
}

/// A global 52-week message (code 36).
pub fn global_week52(exch_seg: u8, scrip_id: u32, high: f32, low: f32) -> Vec<u8> {
    global(G_CODE_WEEK52, exch_seg, scrip_id)
        .f32(G_WEEK52_HIGH, high)
        .f32(G_WEEK52_LOW, low)
        .bytes()
}

/// A global error message (code 50) with the standard header zeroed.
pub fn global_error(error_code: i16) -> Vec<u8> {
    global(G_CODE_ERROR, 0, 0)
        .i16(G_ERROR_CODE, error_code)
        .bytes()
}
