use serde_json::json;

use super::*;

const TOKEN: &str = "eyJhbGciOiJIUzUxMiJ9.eyJzdWIiOiJTRU5USU5FTC1KV1QifQ.U0VOVElORUwtU0lHTkFUVVJF";

#[test]
fn a_token_decodes_every_field() {
    let t: IssuedToken = serde_json::from_value(json!({
        "dhanClientId": "1000000009",
        "dhanClientName": "SENTINEL NAME",
        "dhanClientUcc": "SENTINELUCC",
        "givenPowerOfAttorney": false,
        "accessToken": TOKEN,
        "expiryTime": "2026-09-28T09:15:00"
    }))
    .unwrap();
    assert_eq!(t.dhan_client_id.expose_secret(), "1000000009");
    assert_eq!(t.access_token.expose_secret(), TOKEN);
    assert_eq!(t.dhan_client_name.as_deref(), Some("SENTINEL NAME"));
    assert_eq!(t.dhan_client_ucc.as_deref(), Some("SENTINELUCC"));
    assert_eq!(t.given_power_of_attorney, Some(false));
    assert_eq!(
        t.expiry_time.as_ref().map(WireTime::as_str),
        Some("2026-09-28T09:15:00")
    );
    let c = t.credentials();
    assert_eq!(
        (
            c.client_id().expose_secret(),
            c.access_token().expose_secret()
        ),
        ("1000000009", TOKEN)
    );
}

#[test]
fn the_client_id_and_token_are_required() {
    for body in [
        json!({"accessToken": TOKEN}),
        json!({"dhanClientId": "1000000009"}),
        json!({}),
    ] {
        assert!(
            serde_json::from_value::<IssuedToken>(body.clone()).is_err(),
            "{body}"
        );
    }
    let minimal: IssuedToken =
        serde_json::from_value(json!({"dhanClientId": "1", "accessToken": "t"})).unwrap();
    assert_eq!(
        (minimal.dhan_client_name, minimal.expiry_time),
        (None, None)
    );
}

#[test]
fn debug_redacts_the_identity_and_the_token() {
    let t: IssuedToken = serde_json::from_value(json!({
        "dhanClientId": "1000000009",
        "dhanClientName": "SENTINEL NAME",
        "dhanClientUcc": "SENTINELUCC",
        "accessToken": TOKEN
    }))
    .unwrap();
    let debug = format!("{t:?}");
    for sentinel in [
        "1000000009",
        "SENTINEL NAME",
        "SENTINELUCC",
        TOKEN,
        "eyJ",
        "true",
    ] {
        assert!(!debug.contains(sentinel), "{sentinel} in {debug}");
    }
    assert!(debug.starts_with("IssuedToken {"), "{debug}");
}
