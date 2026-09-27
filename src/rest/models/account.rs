//! Response models for the account endpoints: [`Profile`] (DOC:4870-4879).

use std::fmt;

use serde::Deserialize;

use crate::types::RawJson;

/// The user profile, kept as raw JSON: no source documents its fields (Appendix A D14, OQ-6).
///
/// `Debug` lists only the top-level key names, because a profile carries account details.
#[non_exhaustive]
#[derive(Clone, Deserialize)]
#[serde(transparent)]
pub struct Profile(pub RawJson);

impl fmt::Debug for Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &(self.0).0 {
            serde_json::Value::Object(map) => {
                let keys: Vec<&str> = map.keys().map(String::as_str).collect();
                f.debug_tuple("Profile").field(&keys).finish()
            }
            _ => f.write_str("Profile(<redacted>)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_profile_keeps_the_body_and_debug_shows_only_keys() {
        let p: Profile = serde_json::from_value(json!({
            "dhanClientId": "1000000009",
            "tokenValidity": "30/09/2026 09:15"
        }))
        .unwrap();
        assert_eq!(p.0.0["dhanClientId"], json!("1000000009"));
        let debug = format!("{p:?}");
        assert_eq!(debug, r#"Profile(["dhanClientId", "tokenValidity"])"#);
        let empty: Profile = serde_json::from_value(json!({})).unwrap();
        assert_eq!(format!("{empty:?}"), "Profile([])");
        let scalar: Profile = serde_json::from_value(json!("x")).unwrap();
        assert_eq!(format!("{scalar:?}"), "Profile(<redacted>)");
    }
}
