//! REST facade for the ledger and the trade history (§9 rows T1–T2).

use std::borrow::Cow;

use chrono::{Datelike, NaiveDate};

use crate::error::{Result, ValidationError, ValidationReason};
use crate::rest::endpoint;
use crate::rest::models::LedgerEntries;
use crate::rest::transport::Call;
use crate::rest::{DhanClient, HistoricalTrade, LedgerEntry};

/// The Statements facade, borrowed from a [`DhanClient`].
pub struct Statements<'c> {
    client: &'c DhanClient,
}

/// Both dates must format as `YYYY-MM-DD` (years 1..=9999), and the range must not end before
/// it starts.
fn check_range(from: NaiveDate, to: NaiveDate) -> std::result::Result<(), ValidationError> {
    for (field, date) in [("from", from), ("to", to)] {
        if !(1..=9999).contains(&date.year()) {
            return Err(ValidationError::new(field, ValidationReason::OutOfRange));
        }
    }
    if from > to {
        return Err(ValidationError::new(
            "from",
            ValidationReason::Inconsistent("must not be after to"),
        ));
    }
    Ok(())
}

impl<'c> Statements<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }

    /// Ledger entries between two dates, inclusive: `GET /ledger` with the `from-date` and
    /// `to-date` query parameters in `%Y-%m-%d` form (DOC:1068-1116).
    ///
    /// The response may be one entry or an array of them (Appendix A D37). A range that ends
    /// before it starts is refused without a request.
    pub async fn ledger(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<LedgerEntry>> {
        let query = [
            ("from-date", Cow::Owned(from.to_string())),
            ("to-date", Cow::Owned(to.to_string())),
        ];
        let entries: LedgerEntries = self
            .client
            .execute(&endpoint::STATEMENTS_LEDGER, || {
                check_range(from, to)?;
                Ok(Call {
                    query: &query,
                    ..Call::empty()
                })
            })
            .await?;
        Ok(entries.0)
    }

    /// One page of trades between two dates: `GET /trades/{from-date}/{to-date}/{page}`
    /// (DOC:1628-1694). Pages count from 0.
    ///
    /// A range that ends before it starts is refused without a request.
    pub async fn trade_history(
        &self,
        from: NaiveDate,
        to: NaiveDate,
        page: u32,
    ) -> Result<Vec<HistoricalTrade>> {
        let (from_s, to_s, page_s) = (from.to_string(), to.to_string(), page.to_string());
        let path = [from_s.as_str(), to_s.as_str(), page_s.as_str()];
        self.client
            .execute(&endpoint::STATEMENTS_TRADE_HISTORY, || {
                check_range(from, to)?;
                Ok(Call {
                    path_args: &path,
                    ..Call::empty()
                })
            })
            .await
    }
}
