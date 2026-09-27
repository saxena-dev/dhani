//! REST facade for holdings and positions (§9 rows P1–P4).

use super::json_body;
use crate::error::Result;
use crate::rest::endpoint;
use crate::rest::transport::Call;
use crate::rest::{ConvertPositionRequest, DhanClient, Holding, Position};

/// The Portfolio facade, borrowed from a [`DhanClient`].
pub struct Portfolio<'c> {
    client: &'c DhanClient,
}

impl<'c> Portfolio<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }

    /// The account's demat holdings: `GET /holdings` (DOC:938-981).
    pub async fn holdings(&self) -> Result<Vec<Holding>> {
        self.client
            .execute(&endpoint::PORTFOLIO_HOLDINGS, || Ok(Call::empty()))
            .await
    }

    /// The day's positions: `GET /positions` (DOC:1470-1528).
    pub async fn positions(&self) -> Result<Vec<Position>> {
        self.client
            .execute(&endpoint::PORTFOLIO_POSITIONS, || Ok(Call::empty()))
            .await
    }

    /// Converts an open position between product types: `POST /positions/convert`
    /// (DOC:281-325). The server answers `202` with no body (DOC:293); a JSON body is also
    /// accepted.
    pub async fn convert_position(&self, req: &ConvertPositionRequest) -> Result<()> {
        self.client
            .execute_empty(&endpoint::PORTFOLIO_CONVERT_POSITION, || {
                req.validate()?;
                Ok(Call {
                    body: Some(json_body(req)?),
                    ..Call::empty()
                })
            })
            .await
    }

    /// Exits every open position and cancels every open order for the day:
    /// `DELETE /positions` (DOC:548-578). Documented as `202` with no body (DOC:560); the
    /// OpenAPI spec shows a JSON status body, which is also accepted (OQ-18).
    pub async fn exit_all(&self) -> Result<()> {
        self.client
            .execute_empty(&endpoint::PORTFOLIO_EXIT_ALL, || Ok(Call::empty()))
            .await
    }
}
