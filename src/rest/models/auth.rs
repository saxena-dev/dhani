//! Response models for the auth flows: [`IssuedToken`] (DOC:4433-4440).

use std::fmt;

use serde::{Deserialize, Deserializer};

use crate::credentials::{AccessToken, ClientId, Credentials};
use crate::types::WireTime;

/// A freshly issued access token and the account it belongs to (DOC:4433-4440).
///
/// `dhanClientId` and `accessToken` are required: a body without them fails to decode.
/// `Debug` shows only the field names. It has no `Serialize`
/// and no `PartialEq`, because it carries secrets. Export it with
/// [`credentials`](Self::credentials) and pass that to
/// [`DhanClient::with_credentials`](crate::rest::DhanClient::with_credentials).
#[non_exhaustive]
#[derive(Clone)]
pub struct IssuedToken {
    /// The account.
    pub dhan_client_id: ClientId,
    /// The access token, a JWT valid for about 24 hours.
    pub access_token: AccessToken,
    /// The name registered with Dhan.
    pub dhan_client_name: Option<String>,
    /// The unique client code registered with Dhan.
    pub dhan_client_ucc: Option<String>,
    /// Whether the account has a power of attorney (DDPI) on file.
    pub given_power_of_attorney: Option<bool>,
    /// When the token expires.
    pub expiry_time: Option<WireTime>,
}

impl IssuedToken {
    /// The client ID and access token as [`Credentials`]; the intended way to use the token.
    pub fn credentials(&self) -> Credentials {
        Credentials::new(self.dhan_client_id.clone(), self.access_token.clone())
    }
}

/// Shows the field names only: every value is redacted.
impl fmt::Debug for IssuedToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hidden = format_args!("<redacted>");
        f.debug_struct("IssuedToken")
            .field("dhan_client_id", &hidden)
            .field("access_token", &hidden)
            .field("dhan_client_name", &hidden)
            .field("dhan_client_ucc", &hidden)
            .field("given_power_of_attorney", &hidden)
            .field("expiry_time", &hidden)
            .finish_non_exhaustive()
    }
}

/// The wire shape; the public type is built from it so that the secrets never pass through a
/// derived `Debug`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireToken {
    dhan_client_id: ClientId,
    access_token: AccessToken,
    #[serde(default)]
    dhan_client_name: Option<String>,
    #[serde(default)]
    dhan_client_ucc: Option<String>,
    #[serde(default)]
    given_power_of_attorney: Option<bool>,
    #[serde(default)]
    expiry_time: Option<WireTime>,
}

impl<'de> Deserialize<'de> for IssuedToken {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = WireToken::deserialize(d)?;
        Ok(IssuedToken {
            dhan_client_id: w.dhan_client_id,
            access_token: w.access_token,
            dhan_client_name: w.dhan_client_name,
            dhan_client_ucc: w.dhan_client_ucc,
            given_power_of_attorney: w.given_power_of_attorney,
            expiry_time: w.expiry_time,
        })
    }
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
