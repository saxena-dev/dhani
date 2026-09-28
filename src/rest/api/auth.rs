//! REST facade for the auth host (auth.dhan.co): token generation.
//!
//! The auth calls need no client credentials, so a client built without them can use this
//! facade and then switch to the issued token with
//! [`DhanClient::with_credentials`](crate::rest::DhanClient::with_credentials).

use std::borrow::Cow;

use tracing::Level;

use crate::credentials::{ClientId, Pin, Totp};
use crate::error::Result;
use crate::obs::events::{self, emit};
use crate::rest::endpoint;
use crate::rest::transport::Call;
use crate::rest::{DhanClient, IssuedToken};

/// Access-token generation on Dhan's auth host. Borrowed from a client with
/// [`DhanClient::auth`].
///
/// These calls need no credentials, so a client built without them can generate a token and
/// then switch to it with [`DhanClient::with_credentials`]. They make exactly one attempt.
pub struct Auth<'c> {
    client: &'c DhanClient,
}

impl<'c> Auth<'c> {
    pub(crate) fn new(client: &'c DhanClient) -> Self {
        Self { client }
    }

    /// Generates an access token from the client ID, the PIN and a current TOTP code:
    /// `POST {auth}/app/generateAccessToken?dhanClientId=…&pin=…&totp=…` (DOC:4413-4451), with
    /// no body and no credential headers.
    ///
    /// Dhan issues at most one token every two minutes (DOC:8836). With the default rate
    /// limiter a second call within 120 s is refused locally, unless the window frees within
    /// the admission wait, in which case the call waits and then goes out. A call that reached
    /// the server uses up the window even if it failed (for example a mistyped TOTP), and a
    /// failed call is never retried. The client ID, PIN and TOTP are masked in any stored error
    /// text. dhani does not generate TOTP codes: that would mean holding the TOTP seed.
    pub async fn generate_access_token(
        &self,
        client_id: &ClientId,
        pin: &Pin,
        totp: &Totp,
    ) -> Result<IssuedToken> {
        let query = [
            ("dhanClientId", Cow::Borrowed(client_id.expose_secret())),
            ("pin", Cow::Borrowed(pin.expose_secret())),
            ("totp", Cow::Borrowed(totp.expose_secret())),
        ];
        let secrets = [client_id.as_secret(), pin.as_secret(), totp.as_secret()];
        let token: IssuedToken = self
            .client
            .execute(&endpoint::AUTH_GENERATE_ACCESS_TOKEN, || {
                Ok(Call {
                    query: &query,
                    secrets: &secrets,
                    ..Call::empty()
                })
            })
            .await?;
        emit!(
            Level::INFO,
            events::AUTH_TOKEN_ISSUED,
            method = "totp",
            expiry_present = token.expiry_time.is_some(),
            "access token issued"
        );
        Ok(token)
    }
}
