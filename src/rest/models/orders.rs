//! Request and response models for orders and trades.
//!
//! Requests: [`PlaceOrderRequest`] (DOC:3727-3741) and [`ModifyOrderRequest`] (DOC:3131-3139).
//! Responses: [`OrderAck`] (DOC:3743-3749), [`Order`] (DOC:1254-1286, DOC:1323-1356,
//! DOC:1386-1419) and [`Trade`] (DOC:1762-1782, DOC:1713-1732).

use serde::{Deserialize, Deserializer, Serialize};

use crate::credentials::ClientId;
use crate::error::{ValidationError, ValidationReason};
use crate::types::serde_ext::{na_as_none, num_or_string, one_or_many};
use crate::types::{
    AmoTime, CorrelationId, ExchangeSegment, Inbound, LegName, OptionType, OrderId, OrderStatus,
    OrderType, ProductType, SecurityId, TransactionType, Validity, WireTime,
};

/// A price that is present must be finite and not negative.
fn check_price(field: &'static str, price: Option<f64>) -> Result<(), ValidationError> {
    match price {
        Some(p) if !p.is_finite() => Err(ValidationError::new(field, ValidationReason::NotFinite)),
        Some(p) if p < 0.0 => Err(ValidationError::new(field, ValidationReason::OutOfRange)),
        _ => Ok(()),
    }
}

/// A price the order type needs must be present and above zero.
fn require_positive(field: &'static str, price: Option<f64>) -> Result<(), ValidationError> {
    match price {
        None => Err(ValidationError::new(field, ValidationReason::Missing)),
        Some(p) if p <= 0.0 => Err(ValidationError::new(field, ValidationReason::NotPositive)),
        Some(_) => Ok(()),
    }
}

/// Quantities in requests are at least 1.
fn check_quantity(field: &'static str, quantity: u32) -> Result<(), ValidationError> {
    if quantity == 0 {
        return Err(ValidationError::new(field, ValidationReason::NotPositive));
    }
    Ok(())
}

/// A new order (DOC:3727-3741).
///
/// Build it with [`new`](Self::new) and the `with_*` setters; the facade calls
/// [`validate`](Self::validate) before sending. `price` is omitted when `None`, including for
/// market orders, and the bracket-order fields `boProfitValue` and
/// `boStopLossValue` are never sent.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaceOrderRequest {
    /// A caller-chosen tracking ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<CorrelationId>,
    /// Buy or sell.
    pub transaction_type: TransactionType,
    /// The segment; `IdxI` and `InxEq` cannot be traded here.
    pub exchange_segment: ExchangeSegment,
    /// The product; `Co` and `Bo` cannot be placed.
    pub product_type: ProductType,
    /// The order type.
    pub order_type: OrderType,
    /// How long the order stays live.
    pub validity: Validity,
    /// The instrument.
    pub security_id: SecurityId,
    /// Shares or lots; at least 1.
    pub quantity: u32,
    /// Quantity shown to the market; at most `quantity`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disclosed_quantity: Option<u32>,
    /// Limit price; required above zero for `Limit` and `StopLoss`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<f64>,
    /// Trigger price; required above zero for `StopLoss` and `StopLossMarket`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_price: Option<f64>,
    /// Whether this is an after-market order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_market_order: Option<bool>,
    /// When an after-market order goes to the exchange; only with `after_market_order`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amo_time: Option<AmoTime>,
}

impl PlaceOrderRequest {
    /// An order with the required fields and nothing else.
    pub fn new(
        exchange_segment: ExchangeSegment,
        security_id: SecurityId,
        transaction_type: TransactionType,
        quantity: u32,
        order_type: OrderType,
        product_type: ProductType,
        validity: Validity,
    ) -> Self {
        Self {
            correlation_id: None,
            transaction_type,
            exchange_segment,
            product_type,
            order_type,
            validity,
            security_id,
            quantity,
            disclosed_quantity: None,
            price: None,
            trigger_price: None,
            after_market_order: None,
            amo_time: None,
        }
    }

    /// Sets the limit price.
    pub fn with_price(mut self, price: f64) -> Self {
        self.price = Some(price);
        self
    }

    /// Sets the trigger price.
    pub fn with_trigger_price(mut self, trigger_price: f64) -> Self {
        self.trigger_price = Some(trigger_price);
        self
    }

    /// Sets the disclosed quantity.
    pub fn with_disclosed_quantity(mut self, disclosed_quantity: u32) -> Self {
        self.disclosed_quantity = Some(disclosed_quantity);
        self
    }

    /// Sets the correlation ID.
    pub fn with_correlation_id(mut self, correlation_id: CorrelationId) -> Self {
        self.correlation_id = Some(correlation_id);
        self
    }

    /// Makes this an after-market order sent at `amo_time`: sets both `afterMarketOrder` and
    /// `amoTime`.
    pub fn with_amo(mut self, amo_time: AmoTime) -> Self {
        self.after_market_order = Some(true);
        self.amo_time = Some(amo_time);
        self
    }

    /// Checks the fields against the documented rules, in field order.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if let Some(id) = &self.correlation_id {
            id.validate()?;
        }
        if matches!(
            self.exchange_segment,
            ExchangeSegment::IdxI | ExchangeSegment::InxEq
        ) {
            return Err(ValidationError::new(
                "exchange_segment",
                ValidationReason::UnknownEnumValue,
            ));
        }
        if matches!(self.product_type, ProductType::Co | ProductType::Bo) {
            return Err(ValidationError::new(
                "product_type",
                ValidationReason::UnknownEnumValue,
            ));
        }
        self.security_id.validate()?;
        check_quantity("quantity", self.quantity)?;
        if self.disclosed_quantity.is_some_and(|d| d > self.quantity) {
            return Err(ValidationError::new(
                "disclosed_quantity",
                ValidationReason::Inconsistent("must not exceed quantity"),
            ));
        }
        check_price("price", self.price)?;
        check_price("trigger_price", self.trigger_price)?;
        if matches!(self.order_type, OrderType::Limit | OrderType::StopLoss) {
            require_positive("price", self.price)?;
        }
        if matches!(
            self.order_type,
            OrderType::StopLoss | OrderType::StopLossMarket
        ) {
            require_positive("trigger_price", self.trigger_price)?;
        }
        if self.amo_time.is_some() && self.after_market_order != Some(true) {
            return Err(ValidationError::new(
                "amo_time",
                ValidationReason::Inconsistent("requires after_market_order = true"),
            ));
        }
        Ok(())
    }

    /// [`validate`](Self::validate), plus the price the slicing endpoint requires (DOC:3924).
    pub fn validate_for_slice(&self) -> Result<(), ValidationError> {
        self.validate()?;
        if self.price.is_none() {
            return Err(ValidationError::new("price", ValidationReason::Missing));
        }
        Ok(())
    }
}

/// A change to a pending order (DOC:3131-3139).
///
/// Fields left `None` are omitted rather than sent as `null`. `quantity` is the
/// order's placed quantity, not the pending remainder (DOC:6545).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModifyOrderRequest {
    /// The order to modify; also the path parameter.
    pub order_id: OrderId,
    /// The order type.
    pub order_type: OrderType,
    /// How long the order stays live.
    pub validity: Validity,
    /// The new placed quantity; at least 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<u32>,
    /// The new limit price.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<f64>,
    /// The new disclosed quantity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disclosed_quantity: Option<u32>,
    /// The new trigger price.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_price: Option<f64>,
    /// The super-order leg to modify.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leg_name: Option<LegName>,
}

impl ModifyOrderRequest {
    /// A modification with the required fields and nothing else.
    pub fn new(order_id: OrderId, order_type: OrderType, validity: Validity) -> Self {
        Self {
            order_id,
            order_type,
            validity,
            quantity: None,
            price: None,
            disclosed_quantity: None,
            trigger_price: None,
            leg_name: None,
        }
    }

    /// Sets the placed quantity.
    pub fn with_quantity(mut self, quantity: u32) -> Self {
        self.quantity = Some(quantity);
        self
    }

    /// Sets the limit price.
    pub fn with_price(mut self, price: f64) -> Self {
        self.price = Some(price);
        self
    }

    /// Sets the disclosed quantity.
    pub fn with_disclosed_quantity(mut self, disclosed_quantity: u32) -> Self {
        self.disclosed_quantity = Some(disclosed_quantity);
        self
    }

    /// Sets the trigger price.
    pub fn with_trigger_price(mut self, trigger_price: f64) -> Self {
        self.trigger_price = Some(trigger_price);
        self
    }

    /// Sets the super-order leg.
    pub fn with_leg_name(mut self, leg_name: LegName) -> Self {
        self.leg_name = Some(leg_name);
        self
    }

    /// Checks the fields against the documented rules, in field order.
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.order_id.validate()?;
        if let Some(quantity) = self.quantity {
            check_quantity("quantity", quantity)?;
        }
        check_price("price", self.price)?;
        if let (Some(disclosed), Some(quantity)) = (self.disclosed_quantity, self.quantity)
            && disclosed > quantity
        {
            return Err(ValidationError::new(
                "disclosed_quantity",
                ValidationReason::Inconsistent("must not exceed quantity"),
            ));
        }
        check_price("trigger_price", self.trigger_price)
    }
}

/// The acknowledgement of a place, modify or cancel request (DOC:3743-3749).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderAck {
    /// The order's ID. Some pages spell it `order-id` (DOC:194, DOC:3206).
    #[serde(alias = "order-id")]
    pub order_id: OrderId,
    /// The order's status when acknowledged.
    #[serde(default)]
    pub order_status: Option<Inbound<OrderStatus>>,
}

/// A slice response: one acknowledgement object or an array of them.
pub(crate) fn one_or_many_acks<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<OrderAck>, D::Error> {
    one_or_many(d)
}

/// The body of a slice response, decoded with [`one_or_many_acks`].
#[derive(Debug, Deserialize)]
#[serde(transparent)]
pub(crate) struct SlicedAcks(
    #[serde(deserialize_with = "one_or_many_acks")] pub(crate) Vec<OrderAck>,
);

/// An order from the order book or an order lookup (DOC:1254-1286, DOC:1323-1356,
/// DOC:1386-1419).
///
/// A lookup by ID or correlation ID returns a single object. It is not
/// `PartialEq`, because it carries the client ID, which is deliberately not comparable.
/// The struct is `#[non_exhaustive]`, so it cannot be built outside the crate:
///
/// ```compile_fail,E0639
/// # fn check(order: dhani::rest::Order) {
/// let _ = dhani::rest::Order { order_id: dhani::types::OrderId::new("1").unwrap(), ..order };
/// # }
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Order {
    /// The account; redacted in `Debug`.
    #[serde(default)]
    pub dhan_client_id: Option<ClientId>,
    /// The order's ID. One table spells it `order-id` (DOC:1326).
    #[serde(alias = "order-id")]
    pub order_id: OrderId,
    /// The exchange's order ID.
    #[serde(default)]
    pub exchange_order_id: Option<String>,
    /// The caller's correlation ID. One table spells it `correlation-id` (DOC:1258).
    #[serde(default, alias = "correlation-id")]
    pub correlation_id: Option<String>,
    /// The order's status.
    #[serde(default)]
    pub order_status: Option<Inbound<OrderStatus>>,
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
    /// The validity.
    #[serde(default)]
    pub validity: Option<Inbound<Validity>>,
    /// The trading symbol.
    #[serde(default)]
    pub trading_symbol: Option<String>,
    /// The instrument.
    #[serde(default)]
    pub security_id: Option<SecurityId>,
    /// The placed quantity.
    #[serde(default)]
    pub quantity: Option<i64>,
    /// The disclosed quantity.
    #[serde(default)]
    pub disclosed_quantity: Option<i64>,
    /// The limit price.
    #[serde(default)]
    pub price: Option<f64>,
    /// The trigger price.
    #[serde(default)]
    pub trigger_price: Option<f64>,
    /// Whether this is an after-market order.
    #[serde(default)]
    pub after_market_order: Option<bool>,
    /// The bracket-order target.
    #[serde(default)]
    pub bo_profit_value: Option<f64>,
    /// The bracket-order stop loss.
    #[serde(default)]
    pub bo_stop_loss_value: Option<f64>,
    /// The super-order leg; may be `null` (DOC:6285).
    #[serde(default)]
    pub leg_name: Option<Inbound<LegName>>,
    /// When the order was created.
    #[serde(default)]
    pub create_time: Option<WireTime>,
    /// When the order last changed.
    #[serde(default)]
    pub update_time: Option<WireTime>,
    /// The exchange's timestamp.
    #[serde(default)]
    pub exchange_time: Option<WireTime>,
    /// A derivative's expiry.
    #[serde(default)]
    pub drv_expiry_date: Option<WireTime>,
    /// A derivative's option type; `"NA"` is `None`.
    #[serde(default, deserialize_with = "na_as_none")]
    pub drv_option_type: Option<Inbound<OptionType>>,
    /// A derivative's strike price.
    #[serde(default)]
    pub drv_strike_price: Option<f64>,
    /// The order system's error code.
    #[serde(default)]
    pub oms_error_code: Option<String>,
    /// The order system's error text.
    #[serde(default)]
    pub oms_error_description: Option<String>,
    /// The algorithm ID.
    #[serde(default)]
    pub algo_id: Option<String>,
    /// The quantity still pending.
    #[serde(default)]
    pub remaining_quantity: Option<i64>,
    /// The average traded price; documented as an integer, a float in the fixture, so a number
    /// or a numeric string is accepted.
    #[serde(default, deserialize_with = "num_or_string")]
    pub average_traded_price: Option<f64>,
    /// The filled quantity.
    #[serde(default)]
    pub filled_qty: Option<i64>,
}

/// A trade from the trade book (DOC:1762-1782, DOC:1713-1732).
///
/// Not `PartialEq`, because it carries the client ID, which is deliberately not comparable.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trade {
    /// The account; redacted in `Debug`.
    #[serde(default)]
    pub dhan_client_id: Option<ClientId>,
    /// The order the trade belongs to. One table spells it `order-id` (DOC:1716).
    #[serde(default, alias = "order-id")]
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
    /// The trading symbol.
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
    /// When the order was created.
    #[serde(default)]
    pub create_time: Option<WireTime>,
    /// When the trade was recorded.
    #[serde(default)]
    pub update_time: Option<WireTime>,
    /// The exchange's timestamp.
    #[serde(default)]
    pub exchange_time: Option<WireTime>,
    /// A derivative's expiry.
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
#[path = "orders_tests.rs"]
mod tests;
