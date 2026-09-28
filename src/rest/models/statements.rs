//! Response models for the ledger and the trade history.
//!
//! [`LedgerEntry`] (DOC:1087-1097) and [`HistoricalTrade`] (DOC:1648-1675, guide
//! DOC:6951-7036).

use serde::Deserialize;

use crate::credentials::ClientId;
use crate::types::serde_ext::{na_as_none, num_or_string, one_or_many};
use crate::types::{
    ExchangeSegment, Inbound, Isin, OptionType, OrderId, OrderType, ProductType, SecurityId,
    TransactionType, WireTime,
};

/// One ledger entry (DOC:1087-1097).
///
/// The wire keys are all lowercase. Amounts are strings on the wire (DOC:6914-6925); the
/// `*_f64` methods parse them. Not `PartialEq`, because it carries the client ID, which is
/// deliberately not comparable.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub struct LedgerEntry {
    /// The account; redacted in `Debug`.
    #[serde(default, rename = "dhanClientId")]
    pub dhan_client_id: Option<ClientId>,
    /// The description of the transaction.
    #[serde(default)]
    pub narration: Option<String>,
    /// The date of the entry.
    #[serde(default)]
    pub voucherdate: Option<WireTime>,
    /// The exchange of the transaction.
    #[serde(default)]
    pub exchange: Option<String>,
    /// The nature of the transaction.
    #[serde(default)]
    pub voucherdesc: Option<String>,
    /// The voucher number.
    #[serde(default)]
    pub vouchernumber: Option<String>,
    /// The debit amount, as sent.
    #[serde(default)]
    pub debit: Option<String>,
    /// The credit amount, as sent.
    #[serde(default)]
    pub credit: Option<String>,
    /// The running balance, as sent.
    #[serde(default)]
    pub runbal: Option<String>,
}

/// A finite number parsed from an amount string.
fn amount(value: Option<&str>) -> Option<f64> {
    value
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
}

impl LedgerEntry {
    /// The debit amount as a number, if it parses.
    pub fn debit_f64(&self) -> Option<f64> {
        amount(self.debit.as_deref())
    }

    /// The credit amount as a number, if it parses.
    pub fn credit_f64(&self) -> Option<f64> {
        amount(self.credit.as_deref())
    }

    /// The running balance as a number, if it parses.
    pub fn runbal_f64(&self) -> Option<f64> {
        amount(self.runbal.as_deref())
    }
}

/// A ledger response: one entry object or an array of them.
#[derive(Debug, Deserialize)]
#[serde(transparent)]
pub(crate) struct LedgerEntries(
    #[serde(deserialize_with = "one_or_many")] pub(crate) Vec<LedgerEntry>,
);

/// A trade from the trade history (DOC:1648-1675, guide DOC:6951-7036).
///
/// The trade-book fields, plus the ISIN, the instrument kind and six charges. The charges are
/// floats in one table and strings in the guide, so both are accepted. Not
/// `PartialEq`, because it carries the client ID.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalTrade {
    /// The account; redacted in `Debug`.
    #[serde(default)]
    pub dhan_client_id: Option<ClientId>,
    /// The order the trade belongs to.
    #[serde(default)]
    pub order_id: Option<OrderId>,
    /// The exchange's order ID.
    #[serde(default)]
    pub exchange_order_id: Option<String>,
    /// The exchange's trade ID.
    #[serde(default)]
    pub exchange_trade_id: Option<String>,
    /// Buy or sell.
    #[serde(default)]
    pub transaction_type: Option<Inbound<TransactionType>>,
    /// The segment.
    #[serde(default)]
    pub exchange_segment: Option<Inbound<ExchangeSegment>>,
    /// The product.
    #[serde(default)]
    pub product_type: Option<Inbound<ProductType>>,
    /// The order type.
    #[serde(default)]
    pub order_type: Option<Inbound<OrderType>>,
    /// The trading symbol (guide only).
    #[serde(default)]
    pub trading_symbol: Option<String>,
    /// The display symbol.
    #[serde(default)]
    pub custom_symbol: Option<String>,
    /// The instrument.
    #[serde(default)]
    pub security_id: Option<SecurityId>,
    /// The traded quantity.
    #[serde(default)]
    pub traded_quantity: Option<i64>,
    /// The traded price.
    #[serde(default)]
    pub traded_price: Option<f64>,
    /// The ISIN.
    #[serde(default)]
    pub isin: Option<Isin>,
    /// The instrument kind, such as `EQUITY` or `DERIVATIVES`.
    #[serde(default)]
    pub instrument: Option<String>,
    /// The SEBI turnover fee.
    #[serde(default, deserialize_with = "num_or_string")]
    pub sebi_tax: Option<f64>,
    /// The securities transaction tax.
    #[serde(default, deserialize_with = "num_or_string")]
    pub stt: Option<f64>,
    /// The brokerage.
    #[serde(default, deserialize_with = "num_or_string")]
    pub brokerage_charges: Option<f64>,
    /// The service tax.
    #[serde(default, deserialize_with = "num_or_string")]
    pub service_tax: Option<f64>,
    /// The exchange transaction charges.
    #[serde(default, deserialize_with = "num_or_string")]
    pub exchange_transaction_charges: Option<f64>,
    /// The stamp duty.
    #[serde(default, deserialize_with = "num_or_string")]
    pub stamp_duty: Option<f64>,
    /// When the order was created.
    #[serde(default)]
    pub create_time: Option<WireTime>,
    /// When the trade was recorded.
    #[serde(default)]
    pub update_time: Option<WireTime>,
    /// The exchange's timestamp.
    #[serde(default)]
    pub exchange_time: Option<WireTime>,
    /// A derivative's expiry; a number on the wire is kept as its text.
    #[serde(default)]
    pub drv_expiry_date: Option<WireTime>,
    /// A derivative's option type; `"NA"` is `None`.
    #[serde(default, deserialize_with = "na_as_none")]
    pub drv_option_type: Option<Inbound<OptionType>>,
    /// A derivative's strike price.
    #[serde(default)]
    pub drv_strike_price: Option<f64>,
}

#[cfg(test)]
#[path = "statements_tests.rs"]
mod tests;
