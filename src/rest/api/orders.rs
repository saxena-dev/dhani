//! REST facade for orders and trades.

use super::json_body as body;
use crate::error::{Result, ValidationError};
use crate::rest::endpoint::{self, Endpoint};
use crate::rest::models::SlicedAcks;
use crate::rest::transport::Call;
use crate::rest::{DhanClient, ModifyOrderRequest, Order, OrderAck, PlaceOrderRequest, Trade};
use crate::types::{CorrelationId, OrderId};

/// Placing, modifying and cancelling orders, the order book and the trade book. Borrowed from a
/// client with [`DhanClient::orders`].
///
/// Placement, modification and cancellation make exactly one attempt and are validated before
/// anything is sent. If one fails after the request may have reached Dhan
/// ([`Error::may_have_reached_server`](crate::Error::may_have_reached_server)), the order may
/// exist: look it up by its correlation ID with
/// [`get_by_correlation_id`](Self::get_by_correlation_id) before placing it again. An
/// acknowledgement means Dhan accepted the order, not that it filled. The reads follow the
/// client's [retry rules](crate::rest#retries).
///
/// ```no_run
/// # async fn run(client: &dhani::DhanClient) -> dhani::Result<()> {
/// for order in client.orders().list().await? {
///     println!("{} {:?}", order.order_id, order.order_status);
/// }
/// # Ok(()) }
/// ```
pub struct Orders<'c> {
    client: &'c DhanClient,
}

impl<'c> Orders<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }

    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        ep: &'static Endpoint,
        req: &PlaceOrderRequest,
        validate: fn(&PlaceOrderRequest) -> std::result::Result<(), ValidationError>,
    ) -> Result<T> {
        let correlation = req.correlation_id.as_ref().map(CorrelationId::as_ref);
        self.client
            .execute(ep, || {
                validate(req)?;
                Ok(Call {
                    body: Some(body(req)?),
                    correlation_id: correlation,
                    ..Call::empty()
                })
            })
            .await
    }

    /// Places an order: `POST /orders` (DOC:3712-3770).
    ///
    /// The request is validated first; an invalid one is refused without a request.
    pub async fn place(&self, req: &PlaceOrderRequest) -> Result<OrderAck> {
        self.post(&endpoint::ORDERS_PLACE, req, PlaceOrderRequest::validate)
            .await
    }

    /// Places an order that the exchange's freeze limit splits into several:
    /// `POST /orders/slicing` (DOC:3898-3954).
    ///
    /// The slicing endpoint requires a price (DOC:3924). The response may be one
    /// acknowledgement or an array of them; an empty or `null` list decodes as
    /// an empty `Vec`.
    pub async fn place_sliced(&self, req: &PlaceOrderRequest) -> Result<Vec<OrderAck>> {
        let acks: SlicedAcks = self
            .post(
                &endpoint::ORDERS_PLACE_SLICED,
                req,
                PlaceOrderRequest::validate_for_slice,
            )
            .await?;
        Ok(acks.0)
    }

    /// Modifies a pending order: `PUT /orders/{order-id}` (DOC:3109-3166).
    ///
    /// Only the fields set on the request are sent.
    pub async fn modify(&self, req: &ModifyOrderRequest) -> Result<OrderAck> {
        let path = [req.order_id.as_ref()];
        self.client
            .execute(&endpoint::ORDERS_MODIFY, || {
                req.validate()?;
                Ok(Call {
                    path_args: &path,
                    body: Some(body(req)?),
                    order_id: Some(&req.order_id),
                    ..Call::empty()
                })
            })
            .await
    }

    /// Cancels a pending order: `DELETE /orders/{order-id}` (DOC:131-170).
    ///
    /// An ID taken from a response is revalidated before it is sent.
    pub async fn cancel(&self, order_id: &OrderId) -> Result<OrderAck> {
        let path = [order_id.as_ref()];
        self.client
            .execute(&endpoint::ORDERS_CANCEL, || {
                order_id.validate()?;
                Ok(Call {
                    path_args: &path,
                    order_id: Some(order_id),
                    ..Call::empty()
                })
            })
            .await
    }

    /// The day's orders: `GET /orders` (DOC:1375-1437).
    pub async fn list(&self) -> Result<Vec<Order>> {
        self.client
            .execute(&endpoint::ORDERS_LIST, || Ok(Call::empty()))
            .await
    }

    /// One order by ID: `GET /orders/{order-id}` (DOC:1305-1374). The response is a single
    /// object.
    pub async fn get(&self, order_id: &OrderId) -> Result<Order> {
        let path = [order_id.as_ref()];
        self.client
            .execute(&endpoint::ORDERS_GET, || {
                order_id.validate()?;
                Ok(Call {
                    path_args: &path,
                    order_id: Some(order_id),
                    ..Call::empty()
                })
            })
            .await
    }

    /// One order by its correlation ID: `GET /orders/external/{correlation-id}`
    /// (DOC:1236-1304). The response is a single object.
    pub async fn get_by_correlation_id(&self, id: &CorrelationId) -> Result<Order> {
        let path = [id.as_ref()];
        self.client
            .execute(&endpoint::ORDERS_GET_BY_CORRELATION, || {
                id.validate()?;
                Ok(Call {
                    path_args: &path,
                    correlation_id: Some(id.as_ref()),
                    ..Call::empty()
                })
            })
            .await
    }

    /// The day's trades: `GET /trades` (DOC:1751-1800), with no trailing slash.
    pub async fn trades(&self) -> Result<Vec<Trade>> {
        self.client
            .execute(&endpoint::TRADES_LIST, || Ok(Call::empty()))
            .await
    }

    /// The trades of one order: `GET /trades/{order-id}` (DOC:1695-1750).
    pub async fn trades_for_order(&self, order_id: &OrderId) -> Result<Vec<Trade>> {
        let path = [order_id.as_ref()];
        self.client
            .execute(&endpoint::TRADES_FOR_ORDER, || {
                order_id.validate()?;
                Ok(Call {
                    path_args: &path,
                    order_id: Some(order_id),
                    ..Call::empty()
                })
            })
            .await
    }
}
