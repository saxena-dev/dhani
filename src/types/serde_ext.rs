//! Crate-private serde helpers: `num_or_string`, `int_or_string`, `one_or_many`,
//! `scalar_or_vec`, `na_as_none`, `bool_or_string`, `inbound_ci` and `null_as_empty`.
//!
//! Each is used with `#[serde(default, deserialize_with = "…")]` on the response fields whose
//! wire shape the documentation, the OpenAPI spec and the Python SDK disagree about, and only
//! there. Scalar helpers return `Option<T>` (a JSON `null` is `None`); collection helpers return
//! a `Vec<T>` (a JSON `null` is empty). The helpers' own error messages never repeat the
//! offending value; errors from decoding an inner `T` (in `one_or_many`, `na_as_none`) pass
//! through unchanged, so their text must be kept out of stored errors like any serde error.

use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::types::{Inbound, UnknownValue, WireEnum};

/// An `f64` from a JSON number or a numeric string (Appendix A D38, D60).
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "applied by the response models of later tasks")
)]
pub(crate) fn num_or_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    match Value::deserialize(d)? {
        Value::Null => Ok(None),
        Value::Number(n) => n
            .as_f64()
            .map(Some)
            .ok_or_else(|| D::Error::custom("number is not representable as f64")),
        Value::String(s) => match s.trim().parse::<f64>() {
            Ok(v) if v.is_finite() => Ok(Some(v)),
            _ => Err(D::Error::custom("expected a number or a numeric string")),
        },
        _ => Err(D::Error::custom("expected a number or a numeric string")),
    }
}

/// An `i64` from a JSON integer, a numeric string, or an integral float such as `3.0`; a
/// non-integral value is an error (Appendix A D54).
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "applied by the response models of later tasks")
)]
pub(crate) fn int_or_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    let invalid = || D::Error::custom("expected an integer or an integer string");
    let from_f64 = |f: f64| {
        // Both bounds exclusive: 2^63 does not fit, and -2^63 as a float can only come from an
        // integer below i64::MIN rounding up (i64::MIN itself is caught by the integer path).
        let in_range = f > -9_223_372_036_854_775_808.0 && f < 9_223_372_036_854_775_808.0;
        (f.fract() == 0.0 && in_range).then_some(f as i64)
    };
    match Value::deserialize(d)? {
        Value::Null => Ok(None),
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().and_then(from_f64))
            .map(Some)
            .ok_or_else(invalid),
        Value::String(s) => {
            let s = s.trim();
            s.parse::<i64>()
                .ok()
                .or_else(|| {
                    s.parse::<f64>()
                        .ok()
                        .filter(|f| f.is_finite())
                        .and_then(from_f64)
                })
                .map(Some)
                .ok_or_else(invalid)
        }
        _ => Err(invalid()),
    }
}

/// A `Vec<T>` from a single object or an array of objects (Appendix A D30, D37).
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "applied by the response models of later tasks")
)]
pub(crate) fn one_or_many<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let decode = |v: Value| T::deserialize(v).map_err(D::Error::custom);
    match Value::deserialize(d)? {
        Value::Null => Ok(Vec::new()),
        v @ Value::Object(_) => Ok(vec![decode(v)?]),
        Value::Array(items) => {
            if items.iter().any(|v| !v.is_object()) {
                return Err(D::Error::custom("expected an array of objects"));
            }
            items.into_iter().map(decode).collect()
        }
        _ => Err(D::Error::custom(
            "expected an object or an array of objects",
        )),
    }
}

/// A `Vec<String>` from a single scalar (string or number) or an array of scalars; numbers are
/// kept as their JSON text (Appendix A D41: a field typed string in one table and array in
/// another).
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "applied by the response models of later tasks")
)]
pub(crate) fn scalar_or_vec<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    fn scalar(v: Value) -> Option<String> {
        match v {
            Value::String(s) => Some(s),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }
    let invalid = || D::Error::custom("expected a string, a number or an array of them");
    match Value::deserialize(d)? {
        Value::Null => Ok(Vec::new()),
        Value::Array(items) => items
            .into_iter()
            .map(|v| scalar(v).ok_or_else(invalid))
            .collect(),
        v => scalar(v).map(|s| vec![s]).ok_or_else(invalid),
    }
}

/// `"NA"`, `""` or `null` as `None`; anything else decodes as `T` (DOC:6979-6980, for example
/// `"drvOptionType": "NA"`).
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "applied by the response models of later tasks")
)]
pub(crate) fn na_as_none<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    match Value::deserialize(d)? {
        Value::Null => Ok(None),
        Value::String(s) if s.is_empty() || s == "NA" => Ok(None),
        v => T::deserialize(v).map(Some).map_err(D::Error::custom),
    }
}

/// `true`, `false`, `"true"` or `"false"` (lowercase only); anything else is an error
/// (Appendix A D41: a flag documented as both a boolean and a string).
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "applied by the response models of later tasks")
)]
pub(crate) fn bool_or_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    match Value::deserialize(d)? {
        Value::Null => Ok(None),
        Value::Bool(b) => Ok(Some(b)),
        Value::String(s) if s == "true" => Ok(Some(true)),
        Value::String(s) if s == "false" => Ok(Some(false)),
        _ => Err(D::Error::custom(
            "expected true, false, \"true\" or \"false\"",
        )),
    }
}

/// An `Inbound<T>` whose string matches a wire value ignoring ASCII case; the original text is
/// preserved when nothing matches (Appendix A S15: the order-update sample sends "Cancelled"
/// for `CANCELLED`). Numbers and booleans are kept as unknown text, arrays and objects are
/// errors, as for any `Inbound`.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "applied by the response models of later tasks")
)]
pub(crate) fn inbound_ci<'de, D, T>(d: D) -> Result<Option<Inbound<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: WireEnum,
{
    let unknown = |text: &str| Ok(Some(Inbound::Unknown(UnknownValue::new(text))));
    match Value::deserialize(d)? {
        Value::Null => Ok(None),
        Value::String(s) => match T::ALL.iter().find(|v| v.as_wire().eq_ignore_ascii_case(&s)) {
            Some(v) => Ok(Some(Inbound::Known(*v))),
            None => unknown(&s),
        },
        Value::Number(n) => unknown(&n.to_string()),
        Value::Bool(b) => unknown(if b { "true" } else { "false" }),
        _ => Err(D::Error::custom(
            "expected a string, number or boolean enum value",
        )),
    }
}

/// A collection (or any `Default` value) where JSON `null` counts as empty (§7.0: an absent or
/// `null` collection decodes as empty; `#[serde(default)]` alone covers only absence).
pub(crate) fn null_as_empty<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{OptionType, OrderStatus};

    #[derive(Debug, PartialEq, serde::Deserialize)]
    struct Num(#[serde(deserialize_with = "num_or_string")] Option<f64>);
    #[derive(Debug, PartialEq, serde::Deserialize)]
    struct Int(#[serde(deserialize_with = "int_or_string")] Option<i64>);
    #[derive(Debug, PartialEq, serde::Deserialize)]
    struct Leg {
        id: u32,
    }
    #[derive(Debug, PartialEq, serde::Deserialize)]
    struct Legs(#[serde(deserialize_with = "one_or_many")] Vec<Leg>);
    #[derive(Debug, PartialEq, serde::Deserialize)]
    struct Scalars(#[serde(deserialize_with = "scalar_or_vec")] Vec<String>);
    #[derive(Debug, PartialEq, serde::Deserialize)]
    struct Na(#[serde(deserialize_with = "na_as_none")] Option<Inbound<OptionType>>);
    #[derive(Debug, PartialEq, serde::Deserialize)]
    struct Flag(#[serde(deserialize_with = "bool_or_string")] Option<bool>);
    #[derive(Debug, PartialEq, serde::Deserialize)]
    struct Status(#[serde(deserialize_with = "inbound_ci")] Option<Inbound<OrderStatus>>);

    fn de<T: DeserializeOwned>(json: &str) -> Result<T, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn num_or_string_d38() {
        assert_eq!(de::<Num>("1.5").unwrap(), Num(Some(1.5)));
        assert_eq!(de::<Num>(r#""1.5""#).unwrap(), Num(Some(1.5)));
        assert_eq!(de::<Num>("7").unwrap(), Num(Some(7.0)));
        assert_eq!(de::<Num>(r#""250.00""#).unwrap(), Num(Some(250.0)));
        assert_eq!(de::<Num>("null").unwrap(), Num(None));
        for bad in [
            r#""x""#,
            r#""""#,
            r#""NaN""#,
            r#""inf""#,
            r#""SENTINEL42""#,
            "true",
            "[1]",
            "{}",
        ] {
            let err = de::<Num>(bad).unwrap_err().to_string();
            assert!(!err.contains("SENTINEL42"), "{bad}: {err}");
        }
    }

    #[test]
    fn int_or_string_d54() {
        assert_eq!(de::<Int>("3").unwrap(), Int(Some(3)));
        assert_eq!(de::<Int>(r#""3""#).unwrap(), Int(Some(3)));
        assert_eq!(de::<Int>("3.0").unwrap(), Int(Some(3)));
        assert_eq!(de::<Int>("-2147483648").unwrap(), Int(Some(-2_147_483_648)));
        assert_eq!(de::<Int>("null").unwrap(), Int(None));
        for bad in [
            "3.5",
            r#""3.5""#,
            r#""x""#,
            "18446744073709551615",
            "1e30",
            "-9223372036854775809",
            r#""-9223372036854775809""#,
            "true",
            "[3]",
        ] {
            assert!(de::<Int>(bad).is_err(), "{bad}");
        }
        assert_eq!(
            de::<Int>("-9223372036854775808").unwrap(),
            Int(Some(i64::MIN))
        );
        assert_eq!(
            de::<Int>(r#""9223372036854775807""#).unwrap(),
            Int(Some(i64::MAX))
        );
    }

    #[test]
    fn one_or_many_d30_d37() {
        assert_eq!(
            de::<Legs>(r#"{"id":1}"#).unwrap(),
            Legs(vec![Leg { id: 1 }])
        );
        assert_eq!(
            de::<Legs>(r#"[{"id":1},{"id":2}]"#).unwrap(),
            Legs(vec![Leg { id: 1 }, Leg { id: 2 }])
        );
        assert_eq!(de::<Legs>("[]").unwrap(), Legs(vec![]));
        assert_eq!(de::<Legs>("null").unwrap(), Legs(vec![]));
        for bad in ["1", r#""x""#, "[1]", r#"[{"id":1},2]"#] {
            assert!(de::<Legs>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn scalar_or_vec_d41() {
        assert_eq!(de::<Scalars>(r#""a""#).unwrap(), Scalars(vec!["a".into()]));
        assert_eq!(
            de::<Scalars>(r#"["a",1]"#).unwrap(),
            Scalars(vec!["a".into(), "1".into()])
        );
        assert_eq!(de::<Scalars>("2.5").unwrap(), Scalars(vec!["2.5".into()]));
        assert_eq!(de::<Scalars>("null").unwrap(), Scalars(vec![]));
        for bad in ["{}", "true", r#"[["a"]]"#, r#"[{"a":1}]"#, "[null]"] {
            assert!(de::<Scalars>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn na_as_none_maps_placeholders_to_none() {
        assert_eq!(de::<Na>(r#""NA""#).unwrap(), Na(None));
        assert_eq!(de::<Na>(r#""""#).unwrap(), Na(None));
        assert_eq!(de::<Na>("null").unwrap(), Na(None));
        assert_eq!(
            de::<Na>(r#""CALL""#).unwrap(),
            Na(Some(Inbound::Known(OptionType::Call)))
        );
        let other = de::<Na>(r#""na""#).unwrap();
        assert_eq!(other.0.unwrap().as_wire(), "na");
    }

    #[test]
    fn bool_or_string_d41() {
        assert_eq!(de::<Flag>("true").unwrap(), Flag(Some(true)));
        assert_eq!(de::<Flag>("false").unwrap(), Flag(Some(false)));
        assert_eq!(de::<Flag>(r#""false""#).unwrap(), Flag(Some(false)));
        assert_eq!(de::<Flag>(r#""true""#).unwrap(), Flag(Some(true)));
        assert_eq!(de::<Flag>("null").unwrap(), Flag(None));
        for bad in [r#""TRUE""#, r#""True""#, r#""1""#, "1", r#""yes""#] {
            assert!(de::<Flag>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn inbound_ci_s15() {
        assert_eq!(
            de::<Status>(r#""cancelled""#).unwrap(),
            Status(Some(Inbound::Known(OrderStatus::Cancelled)))
        );
        assert_eq!(
            de::<Status>(r#""Cancelled""#).unwrap(),
            Status(Some(Inbound::Known(OrderStatus::Cancelled)))
        );
        assert_eq!(
            de::<Status>(r#""PART_TRADED""#).unwrap(),
            Status(Some(Inbound::Known(OrderStatus::PartTraded)))
        );
        let unknown = de::<Status>(r#""Queued""#).unwrap().0.unwrap();
        assert_eq!(unknown.known(), None);
        assert_eq!(unknown.as_wire(), "Queued");
        assert_eq!(de::<Status>("7").unwrap().0.unwrap().as_wire(), "7");
        assert_eq!(de::<Status>("null").unwrap(), Status(None));
        assert!(de::<Status>("[]").is_err());
    }

    #[test]
    fn helpers_work_on_optional_fields_with_default() {
        #[derive(Debug, serde::Deserialize)]
        struct Row {
            #[serde(default, deserialize_with = "num_or_string")]
            price: Option<f64>,
            #[serde(default, deserialize_with = "one_or_many")]
            legs: Vec<Leg>,
        }
        let row: Row = de("{}").unwrap();
        assert_eq!(row.price, None);
        assert!(row.legs.is_empty());
    }
}
