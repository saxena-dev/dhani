//! Live Market Feed frame splitter and packet decoders.
//!
//! A frame may hold several packets back to back; [`split_market`] walks the whole frame (the Python
//! SDK decodes only the first packet). Documented codes have fixed sizes. An undocumented code
//! is taken as length-delimited by its header's length field and delivered as
//! [`MarketPacket::Other`]; after one, the next packet must start with a documented code, or the
//! rest of the frame is abandoned with [`DecodeErrorKind::TrailingBytes`]. Every error ends only
//! the current frame.
//!
//! All fields are little-endian; security IDs and lengths are read unsigned. Prices are `f32`
//! exactly as sent (not scaled, NaN passed through). Time fields are raw `u32` values whose epoch
//! is not established; the `ltt_unix` helpers expose them as Unix seconds without converting.

use std::fmt;

use super::error::{DecodeError, DecodeErrorKind};
use super::layout::*;
use crate::types::ExchangeSegment;

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn f32_at(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// The 8-byte header every market-feed packet starts with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketHeader {
    /// Feed response code.
    pub code: u8,
    /// The header's message-length field, as sent (its meaning is not established).
    pub length: u16,
    /// Exchange segment code.
    pub segment_code: u8,
    /// Security ID.
    pub security_id: u32,
}

impl PacketHeader {
    /// Reads the header at the start of `b` (at least [`H_HEADER_LEN`] bytes).
    fn read(b: &[u8]) -> Self {
        PacketHeader {
            code: b[H_CODE],
            length: u16_at(b, H_LEN),
            segment_code: b[H_SEGMENT],
            security_id: u32_at(b, H_SECURITY_ID),
        }
    }

    /// The exchange segment, if the code is a known one.
    pub fn segment(&self) -> Option<ExchangeSegment> {
        ExchangeSegment::from_feed_code(self.segment_code)
    }
}

/// Ticker packet (code 2): last traded price and time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ticker {
    /// Header.
    pub header: PacketHeader,
    /// Last traded price.
    pub ltp: f32,
    /// Last trade time, raw.
    pub ltt: u32,
}

impl Ticker {
    /// The last trade time read as Unix seconds. The epoch is not established; this does not
    /// convert.
    pub fn ltt_unix(&self) -> i64 {
        i64::from(self.ltt)
    }
}

/// Previous-close packet (code 6), sent on subscription.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrevClose {
    /// Header.
    pub header: PacketHeader,
    /// Previous day's close.
    pub prev_close: f32,
    /// Previous day's open interest.
    pub prev_oi: u32,
}

/// Quote packet (code 4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quote {
    /// Header.
    pub header: PacketHeader,
    /// Last traded price.
    pub ltp: f32,
    /// Last traded quantity.
    pub ltq: u16,
    /// Last trade time, raw.
    pub ltt: u32,
    /// Average traded price.
    pub atp: f32,
    /// Volume.
    pub volume: u32,
    /// Total sell quantity.
    pub total_sell_qty: u32,
    /// Total buy quantity.
    pub total_buy_qty: u32,
    /// Day open.
    pub open: f32,
    /// Day close.
    pub close: f32,
    /// Day high.
    pub high: f32,
    /// Day low.
    pub low: f32,
}

impl Quote {
    /// The last trade time read as Unix seconds (not converted; epoch not established).
    pub fn ltt_unix(&self) -> i64 {
        i64::from(self.ltt)
    }
}

/// Open-interest packet (code 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenInterest {
    /// Header.
    pub header: PacketHeader,
    /// Open interest.
    pub oi: u32,
}

/// One of the five market-depth levels of a full packet; level 0 is the best.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthLevel5 {
    /// Bid quantity.
    pub bid_qty: u32,
    /// Ask quantity.
    pub ask_qty: u32,
    /// Number of bid orders.
    pub bid_orders: u16,
    /// Number of ask orders.
    pub ask_orders: u16,
    /// Bid price.
    pub bid_price: f32,
    /// Ask price.
    pub ask_price: f32,
}

/// Full packet (code 8): quote, open interest and five depth levels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Full {
    /// Header.
    pub header: PacketHeader,
    /// Last traded price.
    pub ltp: f32,
    /// Last traded quantity.
    pub ltq: u16,
    /// Last trade time, raw.
    pub ltt: u32,
    /// Average traded price.
    pub atp: f32,
    /// Volume.
    pub volume: u32,
    /// Total sell quantity.
    pub total_sell_qty: u32,
    /// Total buy quantity.
    pub total_buy_qty: u32,
    /// Open interest.
    pub oi: u32,
    /// Day's highest open interest.
    pub oi_day_high: u32,
    /// Day's lowest open interest.
    pub oi_day_low: u32,
    /// Day open.
    pub open: f32,
    /// Day close.
    pub close: f32,
    /// Day high.
    pub high: f32,
    /// Day low.
    pub low: f32,
    /// Five depth levels, best first.
    pub depth: [DepthLevel5; DEPTH5_LEVELS],
}

impl Full {
    /// The last trade time read as Unix seconds (not converted; epoch not established).
    pub fn ltt_unix(&self) -> i64 {
        i64::from(self.ltt)
    }
}

/// Feed disconnect packet (code 50).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Disconnect {
    /// Header.
    pub header: PacketHeader,
    /// Disconnection reason code (a data API error code such as 805).
    pub reason_code: u16,
}

/// A packet with an undocumented code (for example Index, 1, or Market Status, 7), delimited by
/// its header's length field.
#[derive(Clone, PartialEq, Eq)]
pub struct OtherPacket {
    /// Header.
    pub header: PacketHeader,
    /// The bytes after the 8-byte header.
    pub payload: Vec<u8>,
}

impl fmt::Debug for OtherPacket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OtherPacket")
            .field("header", &self.header)
            .field("payload_len", &self.payload.len())
            .finish()
    }
}

impl OtherPacket {
    /// Decodes an Index packet (code 1) with the layout of a commented-out table on the older
    /// documentation site. Unverified: `None` unless the code is 1 and the packet (header
    /// included) has at least 32 bytes.
    pub fn as_legacy_index(&self) -> Option<LegacyIndex> {
        if self.header.code != CODE_INDEX || H_HEADER_LEN + self.payload.len() < LEGACY_INDEX_LEN {
            return None;
        }
        let p = &self.payload;
        let at = |offset: usize| offset - H_HEADER_LEN;
        Some(LegacyIndex {
            value: f32_at(p, at(LEGACY_INDEX_VALUE)),
            open: f32_at(p, at(LEGACY_INDEX_OPEN)),
            close: f32_at(p, at(LEGACY_INDEX_CLOSE)),
            high: f32_at(p, at(LEGACY_INDEX_HIGH)),
            low: f32_at(p, at(LEGACY_INDEX_LOW)),
            updated: u32_at(p, at(LEGACY_INDEX_UPDATED)),
        })
    }
}

/// An Index packet read with the unverified legacy layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LegacyIndex {
    /// Latest index value.
    pub value: f32,
    /// Day open.
    pub open: f32,
    /// Day close (sent after market close).
    pub close: f32,
    /// Day high.
    pub high: f32,
    /// Day low.
    pub low: f32,
    /// Last update time, raw.
    pub updated: u32,
}

/// One decoded market-feed packet.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum MarketPacket {
    /// Code 2.
    Ticker(Ticker),
    /// Code 6.
    PrevClose(PrevClose),
    /// Code 4.
    Quote(Quote),
    /// Code 5.
    OpenInterest(OpenInterest),
    /// Code 8.
    Full(Box<Full>),
    /// Code 50.
    Disconnect(Disconnect),
    /// Any undocumented code.
    Other(OtherPacket),
}

fn error(kind: DecodeErrorKind, offset: usize, code: Option<u8>) -> DecodeError {
    DecodeError {
        kind,
        offset,
        packet_code: code,
    }
}

fn depth_level(b: &[u8], i: usize) -> DepthLevel5 {
    let o = DEPTH5_BASE + DEPTH5_STRIDE * i;
    DepthLevel5 {
        bid_qty: u32_at(b, o + DEPTH5_BID_QTY),
        ask_qty: u32_at(b, o + DEPTH5_ASK_QTY),
        bid_orders: u16_at(b, o + DEPTH5_BID_ORDERS),
        ask_orders: u16_at(b, o + DEPTH5_ASK_ORDERS),
        bid_price: f32_at(b, o + DEPTH5_BID_PRICE),
        ask_price: f32_at(b, o + DEPTH5_ASK_PRICE),
    }
}

/// Decodes one packet with a documented fixed-size `code` from `b`, which starts at the packet's
/// header (whose code byte must equal `code`). Offsets in errors are relative to `b`.
pub fn decode_market_packet(code: u8, b: &[u8]) -> Result<MarketPacket, DecodeError> {
    let unknown = || error(DecodeErrorKind::UnknownCode, 0, Some(code));
    let size = market_size(code).ok_or_else(unknown)?;
    if b.len() < size {
        return Err(error(DecodeErrorKind::Truncated, 0, Some(code)));
    }
    if b[H_CODE] != code {
        return Err(unknown());
    }
    let header = PacketHeader::read(b);
    Ok(match code {
        CODE_TICKER => MarketPacket::Ticker(Ticker {
            header,
            ltp: f32_at(b, TICKER_LTP),
            ltt: u32_at(b, TICKER_LTT),
        }),
        CODE_PREV_CLOSE => MarketPacket::PrevClose(PrevClose {
            header,
            prev_close: f32_at(b, PREV_CLOSE_PRICE),
            prev_oi: u32_at(b, PREV_CLOSE_OI),
        }),
        CODE_QUOTE => MarketPacket::Quote(Quote {
            header,
            ltp: f32_at(b, QUOTE_LTP),
            ltq: u16_at(b, QUOTE_LTQ),
            ltt: u32_at(b, QUOTE_LTT),
            atp: f32_at(b, QUOTE_ATP),
            volume: u32_at(b, QUOTE_VOLUME),
            total_sell_qty: u32_at(b, QUOTE_TOTAL_SELL_QTY),
            total_buy_qty: u32_at(b, QUOTE_TOTAL_BUY_QTY),
            open: f32_at(b, QUOTE_OPEN),
            close: f32_at(b, QUOTE_CLOSE),
            high: f32_at(b, QUOTE_HIGH),
            low: f32_at(b, QUOTE_LOW),
        }),
        CODE_OI => MarketPacket::OpenInterest(OpenInterest {
            header,
            oi: u32_at(b, OI_VALUE),
        }),
        CODE_FULL => MarketPacket::Full(Box::new(Full {
            header,
            ltp: f32_at(b, FULL_LTP),
            ltq: u16_at(b, FULL_LTQ),
            ltt: u32_at(b, FULL_LTT),
            atp: f32_at(b, FULL_ATP),
            volume: u32_at(b, FULL_VOLUME),
            total_sell_qty: u32_at(b, FULL_TOTAL_SELL_QTY),
            total_buy_qty: u32_at(b, FULL_TOTAL_BUY_QTY),
            oi: u32_at(b, FULL_OI),
            oi_day_high: u32_at(b, FULL_OI_DAY_HIGH),
            oi_day_low: u32_at(b, FULL_OI_DAY_LOW),
            open: f32_at(b, FULL_OPEN),
            close: f32_at(b, FULL_CLOSE),
            high: f32_at(b, FULL_HIGH),
            low: f32_at(b, FULL_LOW),
            depth: std::array::from_fn(|i| depth_level(b, i)),
        })),
        CODE_DISCONNECT => MarketPacket::Disconnect(Disconnect {
            header,
            reason_code: u16_at(b, DISCONNECT_REASON),
        }),
        _ => return Err(unknown()),
    })
}

/// A header length field that disagreed with its packet's documented size (neither the total
/// size nor the size without the header). Reported, never fatal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LenMismatch {
    /// Offset of the packet in the frame.
    pub offset: usize,
    /// The packet's code.
    pub code: u8,
    /// The documented total size.
    pub size: usize,
    /// The header's length field.
    pub header_len: u16,
}

/// Splits a market-feed frame into packets. Stops after the first error.
pub fn split_market(frame: &[u8]) -> MarketSplit<'_> {
    MarketSplit {
        frame,
        offset: 0,
        after_other: false,
        done: false,
        len_mismatch: None,
    }
}

/// The iterator returned by [`split_market`].
#[derive(Debug)]
pub struct MarketSplit<'a> {
    frame: &'a [u8],
    offset: usize,
    /// The previous packet was length-delimited, so the next must have a documented code.
    after_other: bool,
    done: bool,
    len_mismatch: Option<LenMismatch>,
}

impl MarketSplit<'_> {
    /// The first header length that disagreed with its packet's documented size, if any.
    pub fn len_mismatch(&self) -> Option<LenMismatch> {
        self.len_mismatch
    }

    fn fail(
        &mut self,
        kind: DecodeErrorKind,
        code: Option<u8>,
    ) -> Option<Result<MarketPacket, DecodeError>> {
        self.done = true;
        Some(Err(error(kind, self.offset, code)))
    }
}

impl std::iter::FusedIterator for MarketSplit<'_> {}

impl Iterator for MarketSplit<'_> {
    type Item = Result<MarketPacket, DecodeError>;

    fn next(&mut self) -> Option<Self::Item> {
        let (buf, o) = (self.frame, self.offset);
        if self.done || o >= buf.len() {
            return None;
        }
        let code = buf[o];
        if self.after_other && market_size(code).is_none() {
            // Alignment after a length-delimited packet is no longer trustworthy, however few
            // bytes remain.
            return self.fail(DecodeErrorKind::TrailingBytes, Some(code));
        }
        if buf.len() - o < H_HEADER_LEN {
            return self.fail(DecodeErrorKind::Truncated, Some(code));
        }
        let Some(size) = market_size(code) else {
            let len = usize::from(u16_at(buf, o + H_LEN));
            if len >= H_HEADER_LEN && o + len <= buf.len() {
                let header = PacketHeader::read(&buf[o..]);
                let payload = buf[o + H_HEADER_LEN..o + len].to_vec();
                self.offset = o + len;
                self.after_other = true;
                return Some(Ok(MarketPacket::Other(OtherPacket { header, payload })));
            }
            return self.fail(DecodeErrorKind::UnknownCode, Some(code));
        };
        self.after_other = false;
        if o + size > buf.len() {
            return self.fail(DecodeErrorKind::Truncated, Some(code));
        }
        let packet = &buf[o..o + size];
        let header_len = u16_at(packet, H_LEN);
        let header_size = usize::from(header_len);
        if header_size != size && header_size + H_HEADER_LEN != size && self.len_mismatch.is_none()
        {
            self.len_mismatch = Some(LenMismatch {
                offset: o,
                code,
                size,
                header_len,
            });
        }
        self.offset = o + size;
        Some(decode_market_packet(code, packet))
    }
}

#[cfg(test)]
#[path = "market_tests.rs"]
mod tests;
