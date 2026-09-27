//! REST facade for the account endpoints on the API host: token renewal and the profile (§9
//! rows A6–A7).

use tracing::Level;

use crate::error::Result;
use crate::obs::events::{self, emit};
use crate::rest::endpoint;
use crate::rest::transport::Call;
use crate::rest::{DhanClient, IssuedToken, Profile};

/// The Account facade, borrowed from a [`DhanClient`].
pub struct Account<'c> {
    client: &'c DhanClient,
}

impl<'c> Account<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }

    /// Renews the client's access token for another 24 hours: `GET /RenewToken`
    /// (DOC:4604-4643), sending the `access-token` and `dhanClientId` headers.
    ///
    /// The documentation describes renewing an active token generated from Dhan Web
    /// (DOC:4608-4610); whether a token from
    /// [`generate_access_token`](crate::rest::Auth::generate_access_token) can be renewed is not
    /// documented. Renewing an expired token is an error (DOC:4612, Appendix A D5). The call is
    /// made once and never retried. The client keeps its old credentials: switch with
    /// [`DhanClient::with_credentials`](crate::rest::DhanClient::with_credentials).
    pub async fn renew_token(&self) -> Result<IssuedToken> {
        let token: IssuedToken = self
            .client
            .execute(&endpoint::ACCOUNT_RENEW_TOKEN, || Ok(Call::empty()))
            .await?;
        emit!(
            Level::INFO,
            events::AUTH_TOKEN_ISSUED,
            method = "renew",
            expiry_present = token.expiry_time.is_some(),
            "access token renewed"
        );
        Ok(token)
    }

    /// The user profile: `GET /profile` (DOC:4870-4879), a quick check that the token works.
    /// The `dhanClientId` header is sent as well as the token, as the Python SDK does
    /// (Appendix A D4).
    pub async fn profile(&self) -> Result<Profile> {
        self.client
            .execute(&endpoint::ACCOUNT_PROFILE, || Ok(Call::empty()))
            .await
    }
}
