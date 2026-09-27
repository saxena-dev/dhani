//! The instrument master (scrip master) CSVs (DOC:5718-5833), parsed by header name.
//!
//! [`InstrumentRecord`] maps the columns of both the detailed and the compact file
//! (DOC:5756-5797); columns it does not type are kept in `extra`.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::IntoDeserializer;
use serde::de::value::{Error as ValueError, StrDeserializer};

use crate::types::{Isin, SecurityId, WireTime};

/// Which instrument master to download.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScripMasterKind {
    /// The compact file, with `SEM_*` column names (DOC:5729).
    Compact,
    /// The detailed file, with plain column names and margin details (DOC:5735).
    Detailed,
}

/// One instrument from a scrip master CSV.
///
/// Every typed field is `Option`: an empty cell or a column the file does not have is `None`.
/// Where both files carry a field, the detailed and the compact column names are accepted.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InstrumentRecord {
    /// The exchange (`NSE`, `BSE`, `MCX`): `EXCH_ID` / `SEM_EXM_EXCH_ID`.
    pub exchange: Option<String>,
    /// The segment letter: `SEGMENT` / `SEM_SEGMENT`.
    pub segment: Option<String>,
    /// The security ID: `SEM_SMST_SECURITY_ID` in the compact file (Appendix A D22); the
    /// detailed file's name is not documented, so `SECURITY_ID` is also read (OQ-14). If a file
    /// had both, the first non-empty one by column position is kept.
    pub security_id: Option<SecurityId>,
    /// The ISIN: `ISIN`.
    pub isin: Option<Isin>,
    /// The instrument: `INSTRUMENT` / `SEM_INSTRUMENT_NAME`.
    pub instrument: Option<String>,
    /// The futures expiry code: `SEM_EXPIRY_CODE`.
    pub expiry_code: Option<String>,
    /// The underlying's security ID: `UNDERLYING_SECURITY_ID`.
    pub underlying_security_id: Option<String>,
    /// The underlying's symbol: `UNDERLYING_SYMBOL`.
    pub underlying_symbol: Option<String>,
    /// The symbol name: `SYMBOL_NAME` / `SM_SYMBOL_NAME`.
    pub symbol_name: Option<String>,
    /// The exchange trading symbol: `SEM_TRADING_SYMBOL`.
    pub trading_symbol: Option<String>,
    /// Dhan's display name: `DISPLAY_NAME` / `SEM_CUSTOM_SYMBOL`.
    pub display_name: Option<String>,
    /// The exchange instrument type: `INSTRUMENT_TYPE` / `SEM_EXCH_INSTRUMENT_TYPE`.
    pub instrument_type: Option<String>,
    /// The series: `SERIES` / `SEM_SERIES`.
    pub series: Option<String>,
    /// The lot size: `LOT_SIZE` / `SEM_LOT_UNITS`.
    pub lot_size: Option<f64>,
    /// The expiry date: `SM_EXPIRY_DATE` / `SEM_EXPIRY_DATE`.
    pub expiry_date: Option<WireTime>,
    /// The strike price: `STRIKE_PRICE` / `SEM_STRIKE_PRICE`.
    pub strike_price: Option<f64>,
    /// `CE` or `PE`: `OPTION_TYPE` / `SEM_OPTION_TYPE`.
    pub option_type: Option<String>,
    /// The tick size: `TICK_SIZE` / `SEM_TICK_SIZE`.
    pub tick_size: Option<f64>,
    /// `M` or `W`: `EXPIRY_FLAG` / `SEM_EXPIRY_FLAG`.
    pub expiry_flag: Option<String>,
    /// Every other column by its header, such as `BRACKET_FLAG` or `MTF_LEVERAGE`.
    pub extra: BTreeMap<String, String>,
}

/// Where one CSV column goes.
#[derive(Clone, Copy)]
enum Column {
    Exchange,
    Segment,
    SecurityId,
    Isin,
    Instrument,
    ExpiryCode,
    UnderlyingSecurityId,
    UnderlyingSymbol,
    SymbolName,
    TradingSymbol,
    DisplayName,
    InstrumentType,
    Series,
    LotSize,
    ExpiryDate,
    StrikePrice,
    OptionType,
    TickSize,
    ExpiryFlag,
    Extra,
}

fn column(header: &str) -> Column {
    match header {
        "EXCH_ID" | "SEM_EXM_EXCH_ID" => Column::Exchange,
        "SEGMENT" | "SEM_SEGMENT" => Column::Segment,
        "SEM_SMST_SECURITY_ID" | "SECURITY_ID" => Column::SecurityId,
        "ISIN" => Column::Isin,
        "INSTRUMENT" | "SEM_INSTRUMENT_NAME" => Column::Instrument,
        "SEM_EXPIRY_CODE" => Column::ExpiryCode,
        "UNDERLYING_SECURITY_ID" => Column::UnderlyingSecurityId,
        "UNDERLYING_SYMBOL" => Column::UnderlyingSymbol,
        "SYMBOL_NAME" | "SM_SYMBOL_NAME" => Column::SymbolName,
        "SEM_TRADING_SYMBOL" => Column::TradingSymbol,
        "DISPLAY_NAME" | "SEM_CUSTOM_SYMBOL" => Column::DisplayName,
        "INSTRUMENT_TYPE" | "SEM_EXCH_INSTRUMENT_TYPE" => Column::InstrumentType,
        "SERIES" | "SEM_SERIES" => Column::Series,
        "LOT_SIZE" | "SEM_LOT_UNITS" => Column::LotSize,
        "SM_EXPIRY_DATE" | "SEM_EXPIRY_DATE" => Column::ExpiryDate,
        "STRIKE_PRICE" | "SEM_STRIKE_PRICE" => Column::StrikePrice,
        "OPTION_TYPE" | "SEM_OPTION_TYPE" => Column::OptionType,
        "TICK_SIZE" | "SEM_TICK_SIZE" => Column::TickSize,
        "EXPIRY_FLAG" | "SEM_EXPIRY_FLAG" => Column::ExpiryFlag,
        _ => Column::Extra,
    }
}

/// A row that could not be read; `row` counts data rows from 1 (the header is not counted), and
/// 0 means the header row itself, including a header with no known column.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MalformedRow {
    pub(crate) row: usize,
}

/// A value decoded through its type's lenient `Deserialize`.
fn lenient<T: for<'de> Deserialize<'de>>(value: &str) -> Option<T> {
    let d: StrDeserializer<'_, ValueError> = value.into_deserializer();
    T::deserialize(d).ok()
}

/// A number cell: empty is `None`, anything else must parse.
fn number(value: &str) -> Result<Option<f64>, ()> {
    if value.is_empty() {
        return Ok(None);
    }
    value
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .map(Some)
        .ok_or(())
}

/// Sets `slot` from a non-empty cell unless an earlier column already did.
fn fill<T>(slot: &mut Option<T>, value: Option<T>) {
    if slot.is_none() {
        *slot = value;
    }
}

impl InstrumentRecord {
    fn set(&mut self, column: Column, header: &str, value: &str) -> Result<(), ()> {
        let text = || (!value.is_empty()).then(|| value.to_owned());
        match column {
            Column::Exchange => fill(&mut self.exchange, text()),
            Column::Segment => fill(&mut self.segment, text()),
            Column::SecurityId if !value.is_empty() => {
                fill(&mut self.security_id, Some(lenient(value).ok_or(())?));
            }
            Column::Isin if !value.is_empty() => {
                fill(&mut self.isin, Some(lenient(value).ok_or(())?));
            }
            Column::SecurityId | Column::Isin => {}
            Column::Instrument => fill(&mut self.instrument, text()),
            Column::ExpiryCode => fill(&mut self.expiry_code, text()),
            Column::UnderlyingSecurityId => fill(&mut self.underlying_security_id, text()),
            Column::UnderlyingSymbol => fill(&mut self.underlying_symbol, text()),
            Column::SymbolName => fill(&mut self.symbol_name, text()),
            Column::TradingSymbol => fill(&mut self.trading_symbol, text()),
            Column::DisplayName => fill(&mut self.display_name, text()),
            Column::InstrumentType => fill(&mut self.instrument_type, text()),
            Column::Series => fill(&mut self.series, text()),
            Column::LotSize => fill(&mut self.lot_size, number(value)?),
            Column::ExpiryDate if !value.is_empty() => {
                fill(&mut self.expiry_date, Some(lenient(value).ok_or(())?));
            }
            Column::ExpiryDate => {}
            Column::StrikePrice => fill(&mut self.strike_price, number(value)?),
            Column::OptionType => fill(&mut self.option_type, text()),
            Column::TickSize => fill(&mut self.tick_size, number(value)?),
            Column::ExpiryFlag => fill(&mut self.expiry_flag, text()),
            Column::Extra => {
                // Like the typed fields, a repeated header keeps its first value.
                self.extra
                    .entry(header.to_owned())
                    .or_insert_with(|| value.to_owned());
            }
        }
        Ok(())
    }
}

/// Parses a scrip master CSV by header name. A row with the wrong number of fields, or a number
/// or ID column that does not parse, fails with that row's index and nothing else. One record
/// buffer is reused across rows.
pub(crate) fn parse_scrip_master(text: &str) -> Result<Vec<InstrumentRecord>, MalformedRow> {
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_reader(text.as_bytes());
    let headers: Vec<(Column, String)> = reader
        .headers()
        .map_err(|_| MalformedRow { row: 0 })?
        .iter()
        .map(|h| {
            let h = h.trim_start_matches('\u{feff}');
            (column(h), h.to_owned())
        })
        .collect();
    // A body with no known column (an error page, say) is not a scrip master.
    if !headers.iter().any(|(c, _)| !matches!(c, Column::Extra)) {
        return Err(MalformedRow { row: 0 });
    }
    let mut records = Vec::new();
    let mut row = csv::StringRecord::new();
    for index in 1.. {
        let malformed = MalformedRow { row: index };
        match reader.read_record(&mut row) {
            Ok(true) => {}
            Ok(false) => break,
            Err(_) => return Err(malformed),
        }
        let mut record = InstrumentRecord::default();
        for ((column, header), value) in headers.iter().zip(row.iter()) {
            if record.set(*column, header, value).is_err() {
                return Err(malformed);
            }
        }
        records.push(record);
    }
    Ok(records)
}

#[cfg(test)]
#[path = "instruments_tests.rs"]
mod tests;
