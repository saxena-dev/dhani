//! Credential and secret types: `ClientId`, `AccessToken`, `Credentials`, `Pin`, `Totp`, `AppId`,
//! `AppSecret`, `AppCredentials`, `PartnerId`, `PartnerSecret`, `PartnerCredentials`, `TokenId`
//! and `ConsentId`.
//!
//! Every secret wraps a [`SecretString`], is validated on construction, exposes its value only
//! through `expose_secret()`, prints `TypeName(<redacted>)` in `Debug` and implements no
//! `Serialize`. The client ID is treated as a secret too: it identifies a brokerage account.
//!
//! Length and charset bounds are SDK policy, rejecting garbage before anything is sent, except
//! where a DOC citation states the rule (PIN digits, six-digit TOTP).
//!
//! `Credentials` cannot be serialised:
//!
//! ```compile_fail,E0277
//! fn needs<T: serde::Serialize>() {}
//! needs::<dhani::Credentials>();
//! ```

use std::fmt;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer};

const REDACTED: &str = "<redacted>";

/// A credential value was rejected. The rejected value is never included.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid {field}: {reason}")]
pub struct CredentialError {
    /// The credential that was rejected.
    pub field: &'static str,
    /// Why it was rejected, without the value.
    pub reason: &'static str,
}

/// A validation rule: accepted characters and length bounds (in bytes, which equal characters
/// for the ASCII-only charsets used here).
struct Rule {
    min: usize,
    max: usize,
    allowed: fn(u8) -> bool,
    /// The reason reported for a length violation.
    length: &'static str,
    /// The reason reported for a charset violation.
    charset: &'static str,
}

impl Rule {
    fn check(&self, field: &'static str, value: &str) -> Result<(), CredentialError> {
        let fail = |reason| Err(CredentialError { field, reason });
        if value.is_empty() {
            return fail("must not be empty");
        }
        if value.trim().is_empty() {
            return fail("must not be blank");
        }
        if !value.bytes().all(self.allowed) {
            return fail(self.charset);
        }
        if !(self.min..=self.max).contains(&value.len()) {
            return fail(self.length);
        }
        Ok(())
    }
}

fn visible_ascii(b: u8) -> bool {
    b.is_ascii_graphic()
}

/// SDK policy for identifiers and secrets whose shape DhanHQ does not document.
const OPAQUE: Rule = Rule {
    min: 1,
    max: 512,
    allowed: visible_ascii,
    length: "must be at most 512 characters",
    charset: "must contain only visible ASCII characters without whitespace",
};

macro_rules! secret_type {
    ($(#[$doc:meta])* $name:ident, $field:literal, $rule:expr) => {
        $(#[$doc])*
        #[derive(Clone)]
        pub struct $name(SecretString);

        impl $name {
            /// Validates `value` and wraps it. The error names the field, never the value.
            pub fn new(value: impl Into<String>) -> Result<Self, CredentialError> {
                // Wrapped first, so a rejected value is zeroized on drop too.
                let value = SecretString::from(value.into());
                $rule.check($field, value.expose_secret())?;
                Ok(Self(value))
            }

            /// The secret value. This is the only way to read it; do not log or persist it.
            pub fn expose_secret(&self) -> &str {
                self.0.expose_secret()
            }

            /// The wrapped secret, for registering it with a call's redactor.
            #[allow(dead_code, reason = "not every secret type is a per-call secret")]
            pub(crate) fn as_secret(&self) -> &SecretString {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), REDACTED)
            }
        }
    };
}

/// `Deserialize` without validation, for token responses and response DTOs.
macro_rules! deserialize_unvalidated {
    ($name:ident) => {
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                String::deserialize(d).map(|s| Self(SecretString::from(s)))
            }
        }
    };
}

secret_type!(
    /// A Dhan client ID: 1..=32 ASCII letters and digits (SDK policy).
    ClientId,
    "client_id",
    Rule {
        min: 1,
        max: 32,
        allowed: |b| b.is_ascii_alphanumeric(),
        length: "must be at most 32 characters",
        charset: "must contain only ASCII letters and digits",
    }
);
deserialize_unvalidated!(ClientId);

secret_type!(
    /// A Dhan access token: 1..=8192 visible ASCII characters without whitespace (SDK policy).
    AccessToken,
    "access_token",
    Rule {
        min: 1,
        max: 8192,
        allowed: visible_ascii,
        length: "must be at most 8192 characters",
        charset: "must contain only visible ASCII characters without whitespace",
    }
);
deserialize_unvalidated!(AccessToken);

secret_type!(
    /// The user's Dhan PIN: ASCII digits (DOC:4427, "numeric code"); 1..=12 digits is SDK policy.
    Pin,
    "pin",
    Rule {
        min: 1,
        max: 12,
        allowed: |b| b.is_ascii_digit(),
        length: "must be at most 12 digits",
        charset: "must contain only digits",
    }
);

secret_type!(
    /// A time-based one-time password: exactly 6 ASCII digits (DOC:4856).
    Totp,
    "totp",
    Rule {
        min: 6,
        max: 6,
        allowed: |b| b.is_ascii_digit(),
        length: "must be exactly 6 digits",
        charset: "must contain only digits",
    }
);

secret_type!(
    /// An API-key app ID (1..=512 visible ASCII characters, SDK policy).
    AppId,
    "app_id",
    OPAQUE
);

secret_type!(
    /// An API-key app secret (1..=512 visible ASCII characters, SDK policy).
    AppSecret,
    "app_secret",
    OPAQUE
);

secret_type!(
    /// A partner ID (1..=512 visible ASCII characters, SDK policy).
    PartnerId,
    "partner_id",
    OPAQUE
);

secret_type!(
    /// A partner secret (1..=512 visible ASCII characters, SDK policy).
    PartnerSecret,
    "partner_secret",
    OPAQUE
);

secret_type!(
    /// The `tokenId` returned by the consent login redirect (DOC:4790-4792); 1..=512 visible
    /// ASCII characters (SDK policy).
    TokenId,
    "token_id",
    OPAQUE
);

secret_type!(
    /// A consent session ID: `consentAppId` (DOC:4397) or the partner `consentId` (DOC:4590);
    /// 1..=512 visible ASCII characters (SDK policy).
    ConsentId,
    "consent_id",
    OPAQUE
);

/// The client ID and access token that authenticate ordinary API calls.
///
/// Cloning copies the values; there is no shared or refreshed state. To rotate a token, build new
/// credentials and a new client.
#[derive(Clone)]
pub struct Credentials {
    client_id: ClientId,
    access_token: AccessToken,
}

impl Credentials {
    /// Pairs a client ID with its access token.
    pub fn new(client_id: ClientId, access_token: AccessToken) -> Self {
        Self {
            client_id,
            access_token,
        }
    }

    /// The client ID.
    pub fn client_id(&self) -> &ClientId {
        &self.client_id
    }

    /// The access token.
    pub fn access_token(&self) -> &AccessToken {
        &self.access_token
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("client_id", &self.client_id)
            .field("access_token", &self.access_token)
            .finish()
    }
}

/// The app ID and secret of an API-key app, used by the consent flow.
pub struct AppCredentials {
    /// The app ID.
    pub app_id: AppId,
    /// The app secret.
    pub app_secret: AppSecret,
}

impl fmt::Debug for AppCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppCredentials")
            .field("app_id", &self.app_id)
            .field("app_secret", &self.app_secret)
            .finish()
    }
}

/// The partner ID and secret used by the partner consent flow.
pub struct PartnerCredentials {
    /// The partner ID.
    pub partner_id: PartnerId,
    /// The partner secret.
    pub partner_secret: PartnerSecret,
}

impl fmt::Debug for PartnerCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PartnerCredentials")
            .field("partner_id", &self.partner_id)
            .field("partner_secret", &self.partner_secret)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Asserts a rejection whose Display names the field and does not contain the input.
    fn rejected<T: fmt::Debug>(
        result: Result<T, CredentialError>,
        field: &str,
        input: &str,
    ) -> CredentialError {
        let err = result.expect_err("input should be rejected");
        assert_eq!(err.field, field);
        let text = err.to_string();
        assert!(text.contains(field), "{text}");
        if !input.trim().is_empty() {
            assert!(!text.contains(input), "{text} echoes {input:?}");
        }
        err
    }

    #[test]
    fn constructors_reject_bad_input_without_echoing_it() {
        let e = rejected(ClientId::new(""), "client_id", "");
        assert_eq!(e.to_string(), "invalid client_id: must not be empty");
        let e = rejected(ClientId::new(" "), "client_id", " ");
        assert_eq!(e.to_string(), "invalid client_id: must not be blank");
        let long = "SENTINELCLIENT7".repeat(3)[..33].to_owned();
        let e = rejected(ClientId::new(long.clone()), "client_id", &long);
        assert_eq!(e.reason, "must be at most 32 characters");
        rejected(ClientId::new("10000-0009"), "client_id", "10000-0009");
        let e = rejected(Pin::new("12a4"), "pin", "12a4");
        assert_eq!(e.reason, "must contain only digits");
        let e = rejected(Totp::new("12345"), "totp", "12345");
        assert_eq!(e.reason, "must be exactly 6 digits");
        rejected(Totp::new("1234567"), "totp", "1234567");
        let e = rejected(AccessToken::new("a b"), "access_token", "a b");
        assert_eq!(
            e.reason,
            "must contain only visible ASCII characters without whitespace"
        );
        rejected(
            AccessToken::new("x".repeat(8193)),
            "access_token",
            &"x".repeat(8193),
        );
        rejected(Pin::new("1".repeat(13)), "pin", &"1".repeat(13));
        rejected(AppSecret::new("s\u{e9}cret"), "app_secret", "s\u{e9}cret");
        rejected(TokenId::new("\t"), "token_id", "\t");
        rejected(
            ConsentId::new("y".repeat(513)),
            "consent_id",
            &"y".repeat(513),
        );
    }

    #[test]
    fn constructors_accept_valid_input() {
        assert_eq!(
            ClientId::new("1000000009").unwrap().expose_secret(),
            "1000000009"
        );
        assert_eq!(
            ClientId::new("x".repeat(32)).unwrap().expose_secret().len(),
            32
        );
        assert!(AccessToken::new("eyJ0eXAiOiJKV1Qi.eyJzdWIi.sig-_~").is_ok());
        assert!(AccessToken::new("x".repeat(8192)).is_ok());
        assert!(Pin::new("1234").is_ok());
        assert!(Totp::new("123456").is_ok());
        for ok in [
            AppId::new("app").map(|v| v.expose_secret().to_owned()),
            PartnerSecret::new("p@ss!").map(|v| v.expose_secret().to_owned()),
            TokenId::new("tok-1").map(|v| v.expose_secret().to_owned()),
        ] {
            assert!(ok.is_ok());
        }
    }

    #[test]
    fn debug_is_redacted() {
        let creds = Credentials::new(
            ClientId::new("SENTINELCID42").unwrap(),
            AccessToken::new("SENTINEL.TOKEN.VALUE").unwrap(),
        );
        let text = format!("{creds:?}");
        assert!(text.contains("<redacted>"), "{text}");
        assert!(
            !text.contains("SENTINELCID42") && !text.contains("SENTINEL.TOKEN.VALUE"),
            "{text}"
        );
        assert_eq!(
            text,
            "Credentials { client_id: ClientId(<redacted>), access_token: AccessToken(<redacted>) }"
        );
        assert_eq!(
            format!("{:?}", Pin::new("9876").unwrap()),
            "Pin(<redacted>)"
        );
        assert_eq!(
            format!("{:?}", Totp::new("654321").unwrap()),
            "Totp(<redacted>)"
        );

        let app = AppCredentials {
            app_id: AppId::new("SENTINEL-APP").unwrap(),
            app_secret: AppSecret::new("SENTINEL-SECRET").unwrap(),
        };
        let text = format!("{app:?}");
        assert!(!text.contains("SENTINEL"), "{text}");
        assert!(
            text.contains("app_id") && text.contains("app_secret"),
            "{text}"
        );
        let partner = PartnerCredentials {
            partner_id: PartnerId::new("SENTINEL-PID").unwrap(),
            partner_secret: PartnerSecret::new("SENTINEL-PSECRET").unwrap(),
        };
        let text = format!("{partner:?}");
        assert!(!text.contains("SENTINEL"), "{text}");
        assert!(
            text.contains("partner_id") && text.contains("partner_secret"),
            "{text}"
        );
    }

    #[test]
    fn accessors_return_the_wrapped_values() {
        let creds = Credentials::new(
            ClientId::new("1000000009").unwrap(),
            AccessToken::new("tok").unwrap(),
        );
        assert_eq!(creds.client_id().expose_secret(), "1000000009");
        assert_eq!(creds.clone().access_token().expose_secret(), "tok");
    }

    #[test]
    fn client_id_and_access_token_deserialise_without_validation() {
        let id: ClientId = serde_json::from_str("\"x\"").unwrap();
        assert_eq!(id.expose_secret(), "x");
        // Not a valid client ID for new(), but responses are taken as they are.
        let id: ClientId = serde_json::from_str("\"a b-c\"").unwrap();
        assert_eq!(id.expose_secret(), "a b-c");
        let token: AccessToken = serde_json::from_str("\"t o k\"").unwrap();
        assert_eq!(token.expose_secret(), "t o k");
        assert!(serde_json::from_str::<ClientId>("1").is_err());
    }
}
