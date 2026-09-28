//! Request and response models for fund limits and the margin calculators.
//!
//! Responses: [`FundLimits`] (DOC:910-919), [`Margin`] (DOC:35-44) and [`MultiMargin`]
//! (DOC:102-111). Requests: [`MarginRequest`] (DOC:21-30) and [`MultiMarginRequest`]
//! (DOC:86-97).

use serde::{Deserialize, Serialize};

use crate::credentials::ClientId;
use crate::error::{ValidationError, ValidationReason};
use crate::types::serde_ext::num_or_string;
use crate::types::{ExchangeSegment, ProductType, SecurityId, TransactionType};

/// The most legs one multi-margin request may carry. The documentation states no bound; this
/// is a local sanity bound (SDK policy).
pub(crate) const MAX_MULTI_MARGIN_LEGS: usize = 50;

/// The account's funds (DOC:910-919).
///
/// Two wire names are misspelled by the server and kept as sent: `availabelBalance` and
/// `receiveableAmount`. Not `PartialEq`, because it carries the client ID,
/// which is deliberately not comparable.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundLimits {
    /// The account; redacted in `Debug`.
    #[serde(default)]
    pub dhan_client_id: Option<ClientId>,
    /// The overall usable balance; wire name `availabelBalance`.
    #[serde(default, rename = "availabelBalance")]
    pub available_balance: Option<f64>,
    /// The start-of-day limit.
    #[serde(default)]
    pub sod_limit: Option<f64>,
    /// The amount received against pledged shares.
    #[serde(default)]
    pub collateral_amount: Option<f64>,
    /// Newly added funds or updated profit; wire name `receiveableAmount`.
    #[serde(default, rename = "receiveableAmount")]
    pub receivable_amount: Option<f64>,
    /// Funds used by trades.
    #[serde(default)]
    pub utilized_amount: Option<f64>,
    /// Funds blocked for withdrawal.
    #[serde(default)]
    pub blocked_payout_amount: Option<f64>,
    /// The amount available to withdraw.
    #[serde(default)]
    pub withdrawable_balance: Option<f64>,
}

/// One order to price with the margin calculator (DOC:21-30), also one leg of a
/// [`MultiMarginRequest`].
///
/// `price` is required: the OpenAPI spec and the Python SDK both require it, although the
/// documentation table marks it optional.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarginRequest {
    /// The segment; `IdxI` and `InxEq` cannot be requested.
    pub exchange_segment: ExchangeSegment,
    /// Buy or sell.
    pub transaction_type: TransactionType,
    /// Shares or lots; at least 1.
    pub quantity: u32,
    /// The product; `Co` and `Bo` cannot be requested.
    pub product_type: ProductType,
    /// The instrument.
    pub security_id: SecurityId,
    /// The order price; finite and not negative.
    pub price: f64,
    /// The trigger price.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_price: Option<f64>,
}

/// A price must be finite and not negative.
fn check_price(field: &'static str, price: f64) -> Result<(), ValidationError> {
    if !price.is_finite() {
        return Err(ValidationError::new(field, ValidationReason::NotFinite));
    }
    if price < 0.0 {
        return Err(ValidationError::new(field, ValidationReason::OutOfRange));
    }
    Ok(())
}

impl MarginRequest {
    /// A request with every required field.
    pub fn new(
        exchange_segment: ExchangeSegment,
        security_id: SecurityId,
        transaction_type: TransactionType,
        quantity: u32,
        product_type: ProductType,
        price: f64,
    ) -> Self {
        Self {
            exchange_segment,
            transaction_type,
            quantity,
            product_type,
            security_id,
            price,
            trigger_price: None,
        }
    }

    /// Sets the trigger price.
    pub fn with_trigger_price(mut self, trigger_price: f64) -> Self {
        self.trigger_price = Some(trigger_price);
        self
    }

    /// Checks the fields against the documented rules, in field order.
    pub fn validate(&self) -> Result<(), ValidationError> {
        // Index values and Global Stocks are not tradable segments (DOC:24 lists the tradable
        // ones; the currency segments are accepted, as for orders).
        if matches!(
            self.exchange_segment,
            ExchangeSegment::IdxI | ExchangeSegment::InxEq
        ) {
            return Err(ValidationError::new(
                "exchange_segment",
                ValidationReason::UnknownEnumValue,
            ));
        }
        if self.quantity == 0 {
            return Err(ValidationError::new(
                "quantity",
                ValidationReason::NotPositive,
            ));
        }
        if matches!(self.product_type, ProductType::Co | ProductType::Bo) {
            return Err(ValidationError::new(
                "product_type",
                ValidationReason::UnknownEnumValue,
            ));
        }
        self.security_id.validate()?;
        check_price("price", self.price)?;
        if let Some(trigger) = self.trigger_price {
            check_price("trigger_price", trigger)?;
        }
        Ok(())
    }
}

/// The margin needed for one order (DOC:35-44). Values are indicative for the current session.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Margin {
    /// The overall margin required.
    #[serde(default)]
    pub total_margin: Option<f64>,
    /// The SPAN margin.
    #[serde(default)]
    pub span_margin: Option<f64>,
    /// The exposure margin.
    #[serde(default)]
    pub exposure_margin: Option<f64>,
    /// The usable balance.
    #[serde(default)]
    pub available_balance: Option<f64>,
    /// The variable margin.
    #[serde(default)]
    pub variable_margin: Option<f64>,
    /// The shortfall, if any.
    #[serde(default)]
    pub insufficient_balance: Option<f64>,
    /// The brokerage.
    #[serde(default)]
    pub brokerage: Option<f64>,
    /// The leverage; a string on the wire.
    #[serde(default)]
    pub leverage: Option<String>,
}

/// Several orders priced together, with hedge benefits (DOC:86-97).
///
/// The wire names are `includeOrder` and `scripList`, as in the documentation table and the
/// Python SDK; the OpenAPI spec's `includeOrders` and `scripts` are not used.
/// Between 1 and 50 legs; the upper bound is SDK policy.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiMarginRequest {
    /// Whether existing positions count toward the margin.
    pub include_position: bool,
    /// Whether pending orders count toward the margin.
    pub include_order: bool,
    /// The orders to price.
    pub scrip_list: Vec<MarginRequest>,
}

impl MultiMarginRequest {
    /// A request over `legs`, excluding existing positions and orders.
    pub fn new(legs: impl IntoIterator<Item = MarginRequest>) -> Self {
        Self {
            include_position: false,
            include_order: false,
            scrip_list: legs.into_iter().collect(),
        }
    }

    /// Counts existing positions toward the margin.
    pub fn with_include_position(mut self, include: bool) -> Self {
        self.include_position = include;
        self
    }

    /// Counts pending orders toward the margin.
    pub fn with_include_order(mut self, include: bool) -> Self {
        self.include_order = include;
        self
    }

    /// Checks the leg count, then every leg.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.scrip_list.is_empty() {
            return Err(ValidationError::new("scrip_list", ValidationReason::Empty));
        }
        if self.scrip_list.len() > MAX_MULTI_MARGIN_LEGS {
            return Err(ValidationError::new(
                "scrip_list",
                ValidationReason::TooMany {
                    max: MAX_MULTI_MARGIN_LEGS,
                },
            ));
        }
        self.scrip_list.iter().try_for_each(MarginRequest::validate)
    }
}

/// The combined margin for several orders (DOC:102-111).
///
/// The documentation table uses camelCase floats; the OpenAPI spec uses snake_case strings
/// (`MultiMarginResponse`). Both are accepted: each amount has its camelCase name, a snake_case
/// alias and accepts a number or a numeric string. Not `PartialEq`, because it
/// carries the client ID.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
pub struct MultiMargin {
    /// The account; redacted in `Debug`.
    #[serde(default, rename = "clientId")]
    pub client_id: Option<ClientId>,
    /// The overall margin required.
    #[serde(
        default,
        rename = "totalMargin",
        alias = "total_margin",
        deserialize_with = "num_or_string"
    )]
    pub total_margin: Option<f64>,
    /// The SPAN margin.
    #[serde(
        default,
        rename = "spanMargin",
        alias = "span_margin",
        deserialize_with = "num_or_string"
    )]
    pub span_margin: Option<f64>,
    /// The exposure margin; `exposure` in the documentation, `exposure_margin` in the OpenAPI
    /// spec.
    #[serde(
        default,
        rename = "exposure",
        alias = "exposure_margin",
        deserialize_with = "num_or_string"
    )]
    pub exposure_margin: Option<f64>,
    /// The equity margin.
    #[serde(
        default,
        rename = "equityMargin",
        alias = "equity_margin",
        deserialize_with = "num_or_string"
    )]
    pub equity_margin: Option<f64>,
    /// The futures and options margin.
    #[serde(
        default,
        rename = "foMargin",
        alias = "fo_margin",
        deserialize_with = "num_or_string"
    )]
    pub fo_margin: Option<f64>,
    /// The commodity margin; `commodity` in the documentation, `commodity_margin` in the
    /// OpenAPI spec.
    #[serde(
        default,
        rename = "commodity",
        alias = "commodity_margin",
        deserialize_with = "num_or_string"
    )]
    pub commodity_margin: Option<f64>,
    /// The currency margin.
    #[serde(default, deserialize_with = "num_or_string")]
    pub currency: Option<f64>,
    /// The hedge benefit; only in the OpenAPI spec.
    #[serde(default, deserialize_with = "num_or_string")]
    pub hedge_benefit: Option<f64>,
}

#[cfg(test)]
#[path = "funds_tests.rs"]
mod tests;
