//! `RawJson`: an undecoded JSON value kept verbatim.

/// A server payload kept as an undecoded JSON value.
///
/// Used where a response is exposed without a typed model. It only ever holds server data, so
/// its `Debug` output is derived.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
#[serde(transparent)]
pub struct RawJson(pub serde_json::Value);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialises_transparently() {
        let raw: RawJson = serde_json::from_str(r#"{"a":[1,"x",null]}"#).unwrap();
        assert_eq!(raw, RawJson(serde_json::json!({"a": [1, "x", null]})));
        let raw: RawJson = serde_json::from_str("3").unwrap();
        assert_eq!(raw.0, serde_json::json!(3));
    }
}
