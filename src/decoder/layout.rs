//! Named offset and length constants for every binary packet.
//!
//! Every multi-byte field is little-endian (DOC:5925-5929, DOC:1925). Offsets are absolute from
//! the start of the packet unless a constant's name says it is relative (the depth-level and
//! market-depth-level constants). Security IDs and lengths are read unsigned even where the
//! documentation types them as signed integers. Each constant cites the documentation table it
//! comes from and the Python SDK's `struct` format for the same packet.
//!
//! The decoders and the test encoder use these constants, so a wrong offset shows up as a failed
//! round trip rather than a silently misread field.

// ---- Live Market Feed: header (DOC:5931-5940; PY:src/dhanhq/marketfeed.py:347 `'<BHBI'`) ----

/// Offset of the feed response code (`u8`) in a market-feed header (DOC:5931-5940).
pub const H_CODE: usize = 0;
/// Offset of the message length (`u16`) in a market-feed header (DOC:5931-5940). Whether it counts
/// the header is not established; the splitter uses the fixed packet sizes instead.
pub const H_LEN: usize = 1;
/// Offset of the exchange segment code (`u8`) in a market-feed header (DOC:5931-5940).
pub const H_SEGMENT: usize = 3;
/// Offset of the security ID (`u32`) in a market-feed header (DOC:5931-5940).
pub const H_SECURITY_ID: usize = 4;
/// Length of the market-feed header (DOC:5931-5940; PY `'<BHBI'`, 8 bytes).
pub const H_HEADER_LEN: usize = 8;

// ---- Live Market Feed: response codes (DOC:4208-4219) ----

/// Index packet: not described by the current documentation (DOC:4208-4219).
pub const CODE_INDEX: u8 = 1;
/// Ticker packet (DOC:4208-4219).
pub const CODE_TICKER: u8 = 2;
/// Quote packet (DOC:4208-4219).
pub const CODE_QUOTE: u8 = 4;
/// Open-interest packet (DOC:4208-4219).
pub const CODE_OI: u8 = 5;
/// Previous-close packet (DOC:4208-4219).
pub const CODE_PREV_CLOSE: u8 = 6;
/// Market-status packet: header only, no documented body (DOC:4208-4219).
pub const CODE_MARKET_STATUS: u8 = 7;
/// Full packet (DOC:4208-4219).
pub const CODE_FULL: u8 = 8;
/// Feed disconnect packet (DOC:4208-4219).
pub const CODE_DISCONNECT: u8 = 50;

// ---- Ticker, code 2 (DOC:5942-5950; PY:src/dhanhq/marketfeed.py:347 `'<BHBIfI'`) ----

/// Offset of the last traded price (`f32`) in a ticker packet (DOC:5942-5950).
pub const TICKER_LTP: usize = 8;
/// Offset of the last trade time (`u32`) in a ticker packet (DOC:5942-5950).
pub const TICKER_LTT: usize = 12;
/// Length of a ticker packet (DOC:5942-5950; PY `'<BHBIfI'`, 16 bytes).
pub const TICKER_LEN: usize = 16;

// ---- Previous close, code 6 (DOC:5952-5959; PY:src/dhanhq/marketfeed.py:360 `'<BHBIfI'`) ----

/// Offset of the previous day's close (`f32`) in a previous-close packet (DOC:5952-5959).
pub const PREV_CLOSE_PRICE: usize = 8;
/// Offset of the previous day's open interest (`u32`) in a previous-close packet
/// (DOC:5952-5959).
pub const PREV_CLOSE_OI: usize = 12;
/// Length of a previous-close packet (DOC:5952-5959; PY `'<BHBIfI'`, 16 bytes).
pub const PREV_CLOSE_LEN: usize = 16;

// ---- Quote, code 4 (DOC:5961-5978; PY:src/dhanhq/marketfeed.py:410 `'<BHBIfHIfIIIffff'`) ----

/// Offset of the last traded price (`f32`) in a quote packet (DOC:5961-5978).
pub const QUOTE_LTP: usize = 8;
/// Offset of the last traded quantity (`u16`) in a quote packet (DOC:5961-5978).
pub const QUOTE_LTQ: usize = 12;
/// Offset of the last trade time (`u32`) in a quote packet (DOC:5961-5978).
pub const QUOTE_LTT: usize = 14;
/// Offset of the average traded price (`f32`) in a quote packet (DOC:5961-5978).
pub const QUOTE_ATP: usize = 18;
/// Offset of the volume (`u32`) in a quote packet (DOC:5961-5978).
pub const QUOTE_VOLUME: usize = 22;
/// Offset of the total sell quantity (`u32`) in a quote packet (DOC:5961-5978).
pub const QUOTE_TOTAL_SELL_QTY: usize = 26;
/// Offset of the total buy quantity (`u32`) in a quote packet (DOC:5961-5978).
pub const QUOTE_TOTAL_BUY_QTY: usize = 30;
/// Offset of the day's open (`f32`) in a quote packet (DOC:5961-5978).
pub const QUOTE_OPEN: usize = 34;
/// Offset of the day's close (`f32`) in a quote packet (DOC:5961-5978).
pub const QUOTE_CLOSE: usize = 38;
/// Offset of the day's high (`f32`) in a quote packet (DOC:5961-5978).
pub const QUOTE_HIGH: usize = 42;
/// Offset of the day's low (`f32`) in a quote packet (DOC:5961-5978).
pub const QUOTE_LOW: usize = 46;
/// Length of a quote packet (DOC:5961-5978; PY `'<BHBIfHIfIIIffff'`, 50 bytes).
pub const QUOTE_LEN: usize = 50;

// ---- Open interest, code 5 (DOC:5980-5986; PY:src/dhanhq/marketfeed.py:431 `'<BHBII'`) ----

/// Offset of the open interest (`u32`) in an open-interest packet (DOC:5980-5986).
pub const OI_VALUE: usize = 8;
/// Length of an open-interest packet (DOC:5980-5986; PY `'<BHBII'`, 12 bytes).
pub const OI_LEN: usize = 12;

// ---- Full, code 8 (DOC:5988-6020; PY:src/dhanhq/marketfeed.py:448
// `'<BHBIfHIfIIIIIIffff100s'`) ----

/// Offset of the last traded price (`f32`) in a full packet (DOC:5988-6020).
pub const FULL_LTP: usize = 8;
/// Offset of the last traded quantity (`u16`) in a full packet (DOC:5988-6020).
pub const FULL_LTQ: usize = 12;
/// Offset of the last trade time (`u32`) in a full packet (DOC:5988-6020).
pub const FULL_LTT: usize = 14;
/// Offset of the average traded price (`f32`) in a full packet (DOC:5988-6020).
pub const FULL_ATP: usize = 18;
/// Offset of the volume (`u32`) in a full packet (DOC:5988-6020).
pub const FULL_VOLUME: usize = 22;
/// Offset of the total sell quantity (`u32`) in a full packet (DOC:5988-6020).
pub const FULL_TOTAL_SELL_QTY: usize = 26;
/// Offset of the total buy quantity (`u32`) in a full packet (DOC:5988-6020).
pub const FULL_TOTAL_BUY_QTY: usize = 30;
/// Offset of the open interest (`u32`) in a full packet (DOC:5988-6020).
pub const FULL_OI: usize = 34;
/// Offset of the day's highest open interest (`u32`) in a full packet (DOC:5988-6020).
pub const FULL_OI_DAY_HIGH: usize = 38;
/// Offset of the day's lowest open interest (`u32`) in a full packet (DOC:5988-6020).
pub const FULL_OI_DAY_LOW: usize = 42;
/// Offset of the day's open (`f32`) in a full packet (DOC:5988-6020).
pub const FULL_OPEN: usize = 46;
/// Offset of the day's close (`f32`) in a full packet (DOC:5988-6020).
pub const FULL_CLOSE: usize = 50;
/// Offset of the day's high (`f32`) in a full packet (DOC:5988-6020).
pub const FULL_HIGH: usize = 54;
/// Offset of the day's low (`f32`) in a full packet (DOC:5988-6020).
pub const FULL_LOW: usize = 58;
/// Offset of the first of the five market-depth levels in a full packet (DOC:6011-6020).
pub const DEPTH5_BASE: usize = 62;
/// Length of one market-depth level; level `i` starts at `DEPTH5_BASE + DEPTH5_STRIDE * i`, level
/// 0 being the best (DOC:6011-6020; PY:src/dhanhq/marketfeed.py:452 `'<IIHHff'`, 20 bytes).
pub const DEPTH5_STRIDE: usize = 20;
/// Number of market-depth levels in a full packet (DOC:6011-6020).
pub const DEPTH5_LEVELS: usize = 5;
/// Level-relative offset of the bid quantity (`u32`) (DOC:6011-6020).
pub const DEPTH5_BID_QTY: usize = 0;
/// Level-relative offset of the ask quantity (`u32`) (DOC:6011-6020).
pub const DEPTH5_ASK_QTY: usize = 4;
/// Level-relative offset of the number of bid orders (`u16`) (DOC:6011-6020).
pub const DEPTH5_BID_ORDERS: usize = 8;
/// Level-relative offset of the number of ask orders (`u16`) (DOC:6011-6020).
pub const DEPTH5_ASK_ORDERS: usize = 10;
/// Level-relative offset of the bid price (`f32`) (DOC:6011-6020).
pub const DEPTH5_BID_PRICE: usize = 12;
/// Level-relative offset of the ask price (`f32`) (DOC:6011-6020).
pub const DEPTH5_ASK_PRICE: usize = 16;
/// Length of a full packet (DOC:5988-6020; PY `'<BHBIfHIfIIIIIIffff100s'`, 162 bytes).
pub const FULL_LEN: usize = 162;

// ---- Disconnect, code 50 (DOC:6036-6039; PY:src/dhanhq/marketfeed.py:495 `'<BHBIH'`) ----

/// Offset of the disconnection reason code (`u16`) in a market-feed disconnect packet
/// (DOC:6036-6039).
pub const DISCONNECT_REASON: usize = 8;
/// Length of a market-feed disconnect packet (DOC:6036-6039; PY `'<BHBIH'`, 10 bytes).
pub const DISCONNECT_LEN: usize = 10;

// ---- Legacy index packet, code 1 (LEGACY:live-market-feed, a commented-out table; no PY
// format) ----

/// Offset of the index value (`f32`) in the legacy index layout (LEGACY:live-market-feed).
pub const LEGACY_INDEX_VALUE: usize = 8;
/// Offset of the day's open (`f32`) in the legacy index layout (LEGACY:live-market-feed).
pub const LEGACY_INDEX_OPEN: usize = 12;
/// Offset of the day's close (`f32`) in the legacy index layout (LEGACY:live-market-feed).
pub const LEGACY_INDEX_CLOSE: usize = 16;
/// Offset of the day's high (`f32`) in the legacy index layout (LEGACY:live-market-feed).
pub const LEGACY_INDEX_HIGH: usize = 20;
/// Offset of the day's low (`f32`) in the legacy index layout (LEGACY:live-market-feed).
pub const LEGACY_INDEX_LOW: usize = 24;
/// Offset of the last update time (`u32`) in the legacy index layout (LEGACY:live-market-feed).
pub const LEGACY_INDEX_UPDATED: usize = 28;
/// Length of the legacy index layout (LEGACY:live-market-feed).
pub const LEGACY_INDEX_LEN: usize = 32;

/// Market-feed codes with a fixed packet length (DOC:5942-6039): `(code, length)`.
pub const FIXED_MARKET_SIZES: [(u8, usize); 6] = [
    (CODE_TICKER, TICKER_LEN),
    (CODE_QUOTE, QUOTE_LEN),
    (CODE_OI, OI_LEN),
    (CODE_PREV_CLOSE, PREV_CLOSE_LEN),
    (CODE_FULL, FULL_LEN),
    (CODE_DISCONNECT, DISCONNECT_LEN),
];

/// The fixed length of a market-feed packet with `code`, or `None` for an unknown or
/// undocumented code.
pub const fn market_size(code: u8) -> Option<usize> {
    let mut i = 0;
    while i < FIXED_MARKET_SIZES.len() {
        if FIXED_MARKET_SIZES[i].0 == code {
            return Some(FIXED_MARKET_SIZES[i].1);
        }
        i += 1;
    }
    None
}

// ---- Full Market Depth: header (DOC:5533-5543, DOC:5565-5575;
// PY:src/dhanhq/fulldepth.py:217 `'<hBBiI'`) ----

/// Offset of the message length (`u16`, including the 12-byte header) in a depth header
/// (DOC:5533-5543; PY:src/dhanhq/fulldepth.py:223-232).
pub const D_LEN: usize = 0;
/// Offset of the feed response code (`u8`) in a depth header (DOC:5533-5543).
pub const D_CODE: usize = 2;
/// Offset of the exchange segment code (`u8`) in a depth header (DOC:5533-5543).
pub const D_SEGMENT: usize = 3;
/// Offset of the security ID (`u32`) in a depth header (DOC:5533-5543).
pub const D_SECURITY_ID: usize = 4;
/// Offset of the message sequence (20-level, to be ignored) or the number of rows (200-level)
/// (`u32`) in a depth header (DOC:5533-5543, DOC:5565-5575).
pub const D_SEQ_OR_ROWS: usize = 8;
/// Length of a depth header (DOC:5533-5543; PY `'<hBBiI'`, 12 bytes).
pub const D_HEADER_LEN: usize = 12;
/// Length of one depth level (DOC:5545-5561; PY:src/dhanhq/fulldepth.py:275 `'<dII'`, 16 bytes).
pub const D_LEVEL_LEN: usize = 16;
/// Level-relative offset of the price (`f64`) (DOC:5545-5561).
pub const D_LEVEL_PRICE: usize = 0;
/// Level-relative offset of the quantity (`u32`) (DOC:5545-5561).
pub const D_LEVEL_QUANTITY: usize = 8;
/// Level-relative offset of the number of orders (`u32`) (DOC:5545-5561).
pub const D_LEVEL_ORDERS: usize = 12;
/// Number of levels in a 20-level depth packet (DOC:5545-5561).
pub const DEPTH20_LEVELS: usize = 20;
/// Length of a 20-level depth packet: header plus 20 levels (DOC:5545-5561).
pub const DEPTH20_LEN: usize = D_HEADER_LEN + DEPTH20_LEVELS * D_LEVEL_LEN;
/// Largest number of rows a 200-level depth packet carries (DOC:5577-5593).
pub const DEPTH200_MAX_ROWS: usize = 200;
/// Depth code of a bid (buy) packet (DOC:5545-5561; PY:src/dhanhq/fulldepth.py:217-275).
pub const DEPTH_CODE_BID: u8 = 41;
/// Depth code of an ask (sell) packet (DOC:5545-5561; PY:src/dhanhq/fulldepth.py:217-275).
pub const DEPTH_CODE_ASK: u8 = 51;
/// Depth code of a disconnect packet (DOC:5613-5614).
pub const DEPTH_CODE_DISCONNECT: u8 = 50;
/// Offset of the documented disconnection code (`i16`, when the packet has at least 14 bytes)
/// in a depth disconnect packet (DOC:5613-5614).
pub const D_DISCONNECT_CODE: usize = 12;
/// Offset of the disconnection code (`u32`) where the Python SDK reads it in a depth disconnect
/// packet (the sequence/rows field; PY:src/dhanhq/fulldepth.py:361-365).
pub const D_DISCONNECT_CODE_ALT: usize = 8;

// ---- Global Stocks Live Feed: header (DOC:1929-1941;
// PY:src/dhanhq/global_stocks_feed.py:260 `'<BiiBB'`) ----

/// Offset of the exchange segment (`u8`; 14 is `INX_EQ`, 0 in error packets) in a global-feed
/// header (DOC:1929-1941).
pub const G_EXCH_SEG: usize = 0;
/// Offset of the scrip ID (`u32`) in a global-feed header (DOC:1929-1941).
pub const G_SCRIP_ID: usize = 1;
/// Offset of the duplicate scrip ID (`u32`) in a global-feed header (DOC:1929-1941).
pub const G_SCRIP_ID2: usize = 5;
/// Offset of the whole-packet length (`u8`) in a global-feed header (DOC:1929-1941).
pub const G_MSG_LENGTH: usize = 9;
/// Offset of the message code (`u8`) in a global-feed header; byte 0 is not the code
/// (DOC:1929-1941).
pub const G_MSG_CODE: usize = 10;
/// Length of a global-feed header (DOC:1929-1941; PY `'<BiiBB'`, 11 bytes).
pub const G_HEADER_LEN: usize = 11;

// ---- Global Stocks Live Feed: messages (DOC:1943-1963, DOC:2008-2027, DOC:2048-2057) ----

/// Trade message code (DOC:1943-1963).
pub const G_CODE_TRADE: u8 = 1;
/// OHLC message code (DOC:1943-1963).
pub const G_CODE_OHLC: u8 = 3;
/// Market-status message code (DOC:1943-1963).
pub const G_CODE_MARKET_STATUS: u8 = 29;
/// Previous-close message code (DOC:1943-1963).
pub const G_CODE_PREV_CLOSE: u8 = 32;
/// Circuit-limit message code (DOC:1943-1963).
pub const G_CODE_CIRCUIT_LIMIT: u8 = 33;
/// 52-week high/low message code (DOC:1943-1963).
pub const G_CODE_WEEK52: u8 = 36;
/// Error (disconnect) message code (DOC:2008-2027).
pub const G_CODE_ERROR: u8 = 50;

/// Offset of the last traded price (`f32`) in a trade message (DOC:2048-2057).
pub const G_TRADE_LTP: usize = 11;
/// Offset of the volume (`u32`) in a trade message (DOC:2048-2057).
pub const G_TRADE_VOLUME: usize = 15;
/// Offset of the last trade time (`u32`) in a trade message (DOC:2048-2057).
pub const G_TRADE_LTT: usize = 19;
/// Offset of the last update time (`u32`) in a trade message (DOC:2048-2057).
pub const G_TRADE_LUT: usize = 23;
/// Length of a trade message (DOC:1943-1963, DOC:2048-2057). The Python SDK assumes 37 bytes
/// (PY:src/dhanhq/global_stocks_feed.py:282); the documentation's 27 is used.
pub const G_TRADE_LEN: usize = 27;

/// Offset of the open (`f32`) in an OHLC message (DOC:2048-2057).
pub const G_OHLC_OPEN: usize = 11;
/// Offset of the close (`f32`) in an OHLC message (DOC:2048-2057).
pub const G_OHLC_CLOSE: usize = 15;
/// Offset of the high (`f32`) in an OHLC message (DOC:2048-2057).
pub const G_OHLC_HIGH: usize = 19;
/// Offset of the low (`f32`) in an OHLC message (DOC:2048-2057).
pub const G_OHLC_LOW: usize = 23;
/// Length of an OHLC message (DOC:2048-2057; PY:src/dhanhq/global_stocks_feed.py:299).
pub const G_OHLC_LEN: usize = 27;

/// Offset of the market type (`[u8; 3]`, NUL-padded ASCII) in a market-status message
/// (DOC:2048-2057).
pub const G_MARKET_TYPE: usize = 11;
/// Length of the market type field (DOC:2048-2057).
pub const G_MARKET_TYPE_LEN: usize = 3;
/// Offset of the market status (`i32`) in a market-status message (DOC:2048-2057).
pub const G_MARKET_STATUS: usize = 14;
/// Length of a market-status message (DOC:2048-2057; PY:src/dhanhq/global_stocks_feed.py:349).
pub const G_MARKET_STATUS_LEN: usize = 18;

/// Offset of the previous close (`f32`) in a previous-close message (DOC:2048-2057).
pub const G_PREV_CLOSE: usize = 11;
/// Length of a previous-close message (DOC:2048-2057). The Python SDK assumes 19 bytes
/// (PY:src/dhanhq/global_stocks_feed.py:313); the documentation's 15 is used.
pub const G_PREV_CLOSE_LEN: usize = 15;

/// Offset of the upper circuit limit (`f32`) in a circuit-limit message (DOC:2048-2057).
pub const G_CIRCUIT_UPPER: usize = 11;
/// Offset of the lower circuit limit (`f32`) in a circuit-limit message (DOC:2048-2057).
pub const G_CIRCUIT_LOWER: usize = 15;
/// Length of a circuit-limit message (DOC:2048-2057; PY:src/dhanhq/global_stocks_feed.py:325).
pub const G_CIRCUIT_LIMIT_LEN: usize = 19;

/// Offset of the 52-week high (`f32`) in a 52-week message (DOC:2048-2057).
pub const G_WEEK52_HIGH: usize = 11;
/// Offset of the 52-week low (`f32`) in a 52-week message (DOC:2048-2057).
pub const G_WEEK52_LOW: usize = 15;
/// Length of a 52-week message (DOC:2048-2057; PY:src/dhanhq/global_stocks_feed.py:337).
pub const G_WEEK52_LEN: usize = 19;

/// Offset of the disconnection code (`i16`) in an error message (DOC:2008-2027).
pub const G_ERROR_CODE: usize = 11;
/// Length of an error message (DOC:2008-2027). The Python SDK reads a 10-byte `'<BHBiH'`
/// packet with the code at byte 0 (PY:src/dhanhq/global_stocks_feed.py:238-239, 360); the
/// documentation's 13 is used.
pub const G_ERROR_LEN: usize = 13;

/// Global-feed message codes and their documented lengths (DOC:1943-1963, DOC:2008-2027):
/// `(code, length)`.
pub const GLOBAL_SIZES: [(u8, usize); 7] = [
    (G_CODE_TRADE, G_TRADE_LEN),
    (G_CODE_OHLC, G_OHLC_LEN),
    (G_CODE_MARKET_STATUS, G_MARKET_STATUS_LEN),
    (G_CODE_PREV_CLOSE, G_PREV_CLOSE_LEN),
    (G_CODE_CIRCUIT_LIMIT, G_CIRCUIT_LIMIT_LEN),
    (G_CODE_WEEK52, G_WEEK52_LEN),
    (G_CODE_ERROR, G_ERROR_LEN),
];

/// The documented length of a global-feed message with `code`, or `None` for an unknown code.
pub const fn global_size(code: u8) -> Option<usize> {
    let mut i = 0;
    while i < GLOBAL_SIZES.len() {
        if GLOBAL_SIZES[i].0 == code {
            return Some(GLOBAL_SIZES[i].1);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_lengths_are_the_documented_sizes() {
        assert_eq!(
            (
                TICKER_LEN,
                PREV_CLOSE_LEN,
                QUOTE_LEN,
                OI_LEN,
                FULL_LEN,
                DISCONNECT_LEN
            ),
            (16, 16, 50, 12, 162, 10)
        );
        assert_eq!(
            (
                DEPTH5_BASE + DEPTH5_LEVELS * DEPTH5_STRIDE,
                LEGACY_INDEX_LEN
            ),
            (162, 32)
        );
        assert_eq!((D_HEADER_LEN, D_LEVEL_LEN, DEPTH20_LEN), (12, 16, 332));
        assert_eq!(
            [
                G_TRADE_LEN,
                G_OHLC_LEN,
                G_MARKET_STATUS_LEN,
                G_PREV_CLOSE_LEN,
                G_CIRCUIT_LIMIT_LEN,
                G_WEEK52_LEN,
                G_ERROR_LEN
            ],
            [27, 27, 18, 15, 19, 19, 13]
        );
    }

    #[test]
    fn each_packet_ends_where_its_last_field_ends() {
        // Last field offset plus its width equals the packet length.
        assert_eq!(TICKER_LTT + 4, TICKER_LEN);
        assert_eq!(PREV_CLOSE_OI + 4, PREV_CLOSE_LEN);
        assert_eq!(QUOTE_LOW + 4, QUOTE_LEN);
        assert_eq!(OI_VALUE + 4, OI_LEN);
        assert_eq!(FULL_LOW + 4, DEPTH5_BASE);
        assert_eq!(DEPTH5_ASK_PRICE + 4, DEPTH5_STRIDE);
        assert_eq!(DISCONNECT_REASON + 2, DISCONNECT_LEN);
        assert_eq!(LEGACY_INDEX_UPDATED + 4, LEGACY_INDEX_LEN);
        assert_eq!(H_SECURITY_ID + 4, H_HEADER_LEN);
        assert_eq!(D_SEQ_OR_ROWS + 4, D_HEADER_LEN);
        assert_eq!(D_LEVEL_ORDERS + 4, D_LEVEL_LEN);
        assert_eq!(D_DISCONNECT_CODE + 2, 14);
        assert_eq!(G_MSG_CODE + 1, G_HEADER_LEN);
        assert_eq!(G_TRADE_LUT + 4, G_TRADE_LEN);
        assert_eq!(G_OHLC_LOW + 4, G_OHLC_LEN);
        assert_eq!(G_MARKET_STATUS + 4, G_MARKET_STATUS_LEN);
        assert_eq!(G_MARKET_TYPE + G_MARKET_TYPE_LEN, G_MARKET_STATUS);
        assert_eq!(G_PREV_CLOSE + 4, G_PREV_CLOSE_LEN);
        assert_eq!(G_CIRCUIT_LOWER + 4, G_CIRCUIT_LIMIT_LEN);
        assert_eq!(G_WEEK52_LOW + 4, G_WEEK52_LEN);
        assert_eq!(G_ERROR_CODE + 2, G_ERROR_LEN);
    }

    #[test]
    fn composite_lengths_hold() {
        assert_eq!(FULL_LEN, DEPTH5_BASE + DEPTH5_LEVELS * DEPTH5_STRIDE);
        assert!(GLOBAL_SIZES.iter().all(|&(_, n)| n >= G_HEADER_LEN));
        assert!(FIXED_MARKET_SIZES.iter().all(|&(_, n)| n >= H_HEADER_LEN));
    }

    #[test]
    fn size_lookups_cover_the_documented_codes() {
        assert_eq!(
            [2, 4, 5, 6, 8, 50].map(market_size),
            [Some(16), Some(50), Some(12), Some(16), Some(162), Some(10)]
        );
        assert_eq!([1, 7, 0, 3].map(market_size), [None; 4]);
        assert_eq!(
            [1, 3, 29, 32, 33, 36, 50].map(global_size),
            [
                Some(27),
                Some(27),
                Some(18),
                Some(15),
                Some(19),
                Some(19),
                Some(13)
            ]
        );
        assert_eq!([0, 2, 51].map(global_size), [None; 3]);
    }
}
