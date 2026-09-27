//! Request and response models for holdings and positions.
//!
//! Responses: [`Holding`] (DOC:951-963) and [`Position`] (DOC:1483-1510). Request:
//! [`ConvertPositionRequest`] (DOC:298-306).

use serde::{Deserialize, Serialize};

use crate::credentials::ClientId;
use crate::error::{ValidationError, ValidationReason};
use crate::types::serde_ext::na_as_none;
use crate::types::{
    ExchangeSegment, Inbound, Isin, OptionType, PositionType, ProductType, SecurityId, WireTime,
};

/// A demat holding (DOC:951-963).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Holding {
    /// The exchange: `NSE`, `BSE`, `MCX` or `ALL`.
    #[serde(default)]
    pub exchange: Option<String>,
    /// The trading symbol.
    #[serde(default)]
    pub trading_symbol: Option<String>,
    /// The instrument.
    #[serde(default)]
    pub security_id: Option<SecurityId>,
    /// The ISIN.
    #[serde(default)]
    pub isin: Option<Isin>,
    /// The total quantity held.
    #[serde(default)]
    pub total_qty: Option<i64>,
    /// The quantity delivered to the demat account.
    #[serde(default)]
    pub dp_qty: Option<i64>,
    /// The quantity awaiting T+1 delivery.
    #[serde(default)]
    pub t1_qty: Option<i64>,
    /// The margin-trading quantity awaiting T+1 delivery; the wire name is snake_case
    /// (Appendix A D47).
    #[serde(default, rename = "mtf_t1_qty")]
    pub mtf_t1_qty: Option<i64>,
    /// The margin-trading quantity; the wire name is snake_case (Appendix A D47).
    #[serde(default, rename = "mtf_qty")]
    pub mtf_qty: Option<i64>,
    /// The quantity available to sell.
    #[serde(default)]
    pub available_qty: Option<i64>,
    /// The quantity pledged as collateral.
    #[serde(default)]
    pub collateral_qty: Option<i64>,
    /// The average cost price.
    #[serde(default)]
    pub avg_cost_price: Option<f64>,
    /// The last traded price.
    #[serde(default)]
    pub last_traded_price: Option<f64>,
}

/// An open or closed position for the day (DOC:1483-1510).
///
/// Not `PartialEq`, because it carries the client ID, which is deliberately not comparable.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    /// The account; redacted in `Debug`.
    #[serde(default)]
    pub dhan_client_id: Option<ClientId>,
    /// The trading symbol.
    #[serde(default)]
    pub trading_symbol: Option<String>,
    /// The instrument.
    #[serde(default)]
    pub security_id: Option<SecurityId>,
    /// Long, short or closed.
    #[serde(default)]
    pub position_type: Option<Inbound<PositionType>>,
    /// The segment.
    #[serde(default)]
    pub exchange_segment: Option<Inbound<ExchangeSegment>>,
    /// The product.
    #[serde(default)]
    pub product_type: Option<Inbound<ProductType>>,
    /// The average buy price.
    #[serde(default)]
    pub buy_avg: Option<f64>,
    /// The cost price.
    #[serde(default)]
    pub cost_price: Option<f64>,
    /// The quantity bought.
    #[serde(default)]
    pub buy_qty: Option<i64>,
    /// The average sell price.
    #[serde(default)]
    pub sell_avg: Option<f64>,
    /// The quantity sold.
    #[serde(default)]
    pub sell_qty: Option<i64>,
    /// The net quantity.
    #[serde(default)]
    pub net_qty: Option<i64>,
    /// The realised profit.
    #[serde(default)]
    pub realized_profit: Option<f64>,
    /// The unrealised profit.
    #[serde(default)]
    pub unrealized_profit: Option<f64>,
    /// The RBI reference rate, for currency contracts.
    #[serde(default)]
    pub rbi_reference_rate: Option<f64>,
    /// The contract multiplier.
    #[serde(default)]
    pub multiplier: Option<i64>,
    /// The carried-forward quantity bought.
    #[serde(default)]
    pub carry_forward_buy_qty: Option<i64>,
    /// The carried-forward quantity sold.
    #[serde(default)]
    pub carry_forward_sell_qty: Option<i64>,
    /// The carried-forward buy value.
    #[serde(default)]
    pub carry_forward_buy_value: Option<f64>,
    /// The carried-forward sell value.
    #[serde(default)]
    pub carry_forward_sell_value: Option<f64>,
    /// The quantity bought today.
    #[serde(default)]
    pub day_buy_qty: Option<i64>,
    /// The quantity sold today.
    #[serde(default)]
    pub day_sell_qty: Option<i64>,
    /// The value bought today.
    #[serde(default)]
    pub day_buy_value: Option<f64>,
    /// The value sold today.
    #[serde(default)]
    pub day_sell_value: Option<f64>,
    /// A derivative's expiry.
    #[serde(default)]
    pub drv_expiry_date: Option<WireTime>,
    /// A derivative's option type; `"NA"` is `None`.
    #[serde(default, deserialize_with = "na_as_none")]
    pub drv_option_type: Option<Inbound<OptionType>>,
    /// A derivative's strike price.
    #[serde(default)]
    pub drv_strike_price: Option<f64>,
    /// Whether this is a cross-currency contract.
    #[serde(default)]
    pub cross_currency: Option<bool>,
}

/// A conversion of an open position between product types (DOC:298-306).
///
/// Every documented field is required here (Appendix A D36). `trading_symbol` is optional and
/// sent only when set: the OpenAPI spec requires it while the documentation table and the Python
/// SDK omit it (Appendix A D61), so setting it is recommended.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertPositionRequest {
    /// The product converted from: `Cnc`, `Intraday` or `Margin`.
    pub from_product_type: ProductType,
    /// The segment.
    pub exchange_segment: ExchangeSegment,
    /// The position to convert.
    pub position_type: PositionType,
    /// The instrument.
    pub security_id: SecurityId,
    /// The trading symbol.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trading_symbol: Option<String>,
    /// The quantity to convert; at least 1.
    pub convert_qty: u32,
    /// The product converted to: `Cnc`, `Intraday` or `Margin`.
    pub to_product_type: ProductType,
}

/// The products a position can be converted from and to (DOC:301, DOC:306).
fn convertible(product: ProductType) -> bool {
    matches!(
        product,
        ProductType::Cnc | ProductType::Intraday | ProductType::Margin
    )
}

impl ConvertPositionRequest {
    /// A conversion with every required field.
    pub fn new(
        from_product_type: ProductType,
        to_product_type: ProductType,
        exchange_segment: ExchangeSegment,
        position_type: PositionType,
        security_id: SecurityId,
        convert_qty: u32,
    ) -> Self {
        Self {
            from_product_type,
            exchange_segment,
            position_type,
            security_id,
            trading_symbol: None,
            convert_qty,
            to_product_type,
        }
    }

    /// Sets the trading symbol (recommended; Appendix A D61).
    pub fn with_trading_symbol(mut self, trading_symbol: impl Into<String>) -> Self {
        self.trading_symbol = Some(trading_symbol.into());
        self
    }

    /// Checks the fields against the documented rules, in field order.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if !convertible(self.from_product_type) {
            return Err(ValidationError::new(
                "from_product_type",
                ValidationReason::UnknownEnumValue,
            ));
        }
        self.security_id.validate()?;
        if self.trading_symbol.as_deref() == Some("") {
            return Err(ValidationError::new(
                "trading_symbol",
                ValidationReason::Empty,
            ));
        }
        if self.convert_qty == 0 {
            return Err(ValidationError::new(
                "convert_qty",
                ValidationReason::NotPositive,
            ));
        }
        if !convertible(self.to_product_type) {
            return Err(ValidationError::new(
                "to_product_type",
                ValidationReason::UnknownEnumValue,
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "portfolio_tests.rs"]
mod tests;
