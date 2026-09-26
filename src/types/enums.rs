//! The `wire_enum!` macro, the `WireEnum` trait, `Inbound<T>` and `UnknownValue`.
//!
//! Request enums are plain `T` and serialise to their exact wire string. Response enums are
//! [`Inbound<T>`]: a value this build recognises is `Known`, anything else is preserved as
//! `Unknown`. There is deliberately no conversion from `Inbound<T>` to `T`, so an unknown
//! inbound value can never be sent back to the broker.

use std::fmt;

use serde::de::{self, Deserializer, Visitor};

/// An enum with a fixed set of broker wire strings.
pub trait WireEnum: Sized + Copy + Eq + std::hash::Hash + fmt::Debug + 'static {
    /// Every variant, in declaration order.
    const ALL: &'static [Self];
    /// The exact wire string of this value.
    fn as_wire(self) -> &'static str;
    /// The value whose wire string is exactly `s`, if any. Matching is case-sensitive.
    fn from_wire(s: &str) -> Option<Self>;
}

/// An inbound value this build does not recognise, preserved as text.
///
/// The text keeps at most 64 bytes of the original, cut on a character boundary; longer input
/// sets [`is_truncated`](UnknownValue::is_truncated). The bound is SDK policy: it keeps a hostile
/// or corrupt response from growing every decoded record.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UnknownValue {
    text: String,
    truncated: bool,
}

impl UnknownValue {
    const MAX_BYTES: usize = 64;

    pub(crate) fn new(raw: &str) -> Self {
        if raw.len() <= Self::MAX_BYTES {
            return Self {
                text: raw.to_owned(),
                truncated: false,
            };
        }
        let mut end = Self::MAX_BYTES;
        while !raw.is_char_boundary(end) {
            end -= 1;
        }
        Self {
            text: raw[..end].to_owned(),
            truncated: true,
        }
    }

    /// The preserved text, possibly truncated.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Whether the original text was longer than the preserved text.
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }
}

/// A known enum value, or the preserved text of an unknown one.
///
/// Response fields use `Inbound<T>` so that a value missing from the documentation (DhanHQ's
/// enum lists are incomplete) decodes instead of failing the whole response. It cannot be turned
/// back into a `T` for an outbound request:
///
/// ```compile_fail,E0277
/// use dhani::types::{Inbound, OrderStatus};
///
/// let s: OrderStatus = Inbound::<OrderStatus>::Known(OrderStatus::Traded).into();
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Inbound<T> {
    /// A value this build recognises.
    Known(T),
    /// A value this build does not recognise, preserved.
    Unknown(UnknownValue),
}

impl<T: WireEnum> Inbound<T> {
    /// Classifies a wire string: an exact match is `Known`, anything else `Unknown`.
    pub(crate) fn from_wire(s: &str) -> Self {
        match T::from_wire(s) {
            Some(v) => Self::Known(v),
            None => Self::Unknown(UnknownValue::new(s)),
        }
    }

    /// The known value, or `None` for an unknown one.
    pub fn known(&self) -> Option<T> {
        match self {
            Self::Known(v) => Some(*v),
            Self::Unknown(_) => None,
        }
    }

    /// The wire text: the known value's wire string, or the preserved text.
    pub fn as_wire(&self) -> &str {
        match self {
            Self::Known(v) => v.as_wire(),
            Self::Unknown(u) => u.as_str(),
        }
    }
}

/// Deserialises an `Inbound<T>` under the inbound policy: a JSON string is classified by
/// [`Inbound::from_wire`]; a number or bool becomes `Unknown` holding its JSON text; an array,
/// object or null is an error (null is handled by an enclosing `Option`).
pub(crate) fn deserialize_inbound<'de, T: WireEnum, D: Deserializer<'de>>(
    d: D,
) -> Result<Inbound<T>, D::Error> {
    struct InboundVisitor<T>(std::marker::PhantomData<T>);

    impl<T: WireEnum> Visitor<'_> for InboundVisitor<T> {
        type Value = Inbound<T>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a string, number or boolean enum value")
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            Ok(Inbound::from_wire(v))
        }

        fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
            Ok(Inbound::Unknown(UnknownValue::new(if v {
                "true"
            } else {
                "false"
            })))
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            Ok(Inbound::Unknown(UnknownValue::new(&v.to_string())))
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            Ok(Inbound::Unknown(UnknownValue::new(&v.to_string())))
        }

        fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
            // serde_json's own formatting, so 1.0 stays "1.0" as it appeared on the wire.
            let text =
                serde_json::Number::from_f64(v).map_or_else(|| v.to_string(), |n| n.to_string());
            Ok(Inbound::Unknown(UnknownValue::new(&text)))
        }
    }

    // Relies on serde_json visiting numbers as numbers; its `arbitrary_precision` feature would
    // present them as maps and turn them into errors instead of `Unknown`.
    d.deserialize_any(InboundVisitor(std::marker::PhantomData))
}

/// Deserialises a `T` strictly: only an exact wire string is accepted. The offending text is
/// not echoed into the error.
pub(crate) fn deserialize_strict<'de, T: WireEnum, D: Deserializer<'de>>(
    d: D,
    name: &'static str,
) -> Result<T, D::Error> {
    struct StrictVisitor<T>(&'static str, std::marker::PhantomData<T>);

    impl<T: WireEnum> Visitor<'_> for StrictVisitor<T> {
        type Value = T;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "a {} wire string", self.0)
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            T::from_wire(v).ok_or_else(|| self.unknown())
        }

        // Non-string scalars are rejected with the same message, so their value is not echoed.
        fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
            Err(self.unknown())
        }

        fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
            Err(self.unknown())
        }

        fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
            Err(self.unknown())
        }

        fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
            Err(self.unknown())
        }
    }

    impl<T> StrictVisitor<T> {
        fn unknown<E: de::Error>(&self) -> E {
            E::custom(format_args!("unknown {} value", self.0))
        }
    }

    // `deserialize_any`, so that non-string scalars reach the visitor instead of being echoed
    // by the format's own type error.
    d.deserialize_any(StrictVisitor(name, std::marker::PhantomData))
}

/// Declares a closed wire enum: `#[non_exhaustive]` with the standard derives, plus
/// [`WireEnum`], `Display` and `FromStr` (the wire string), `Serialize` (the wire string),
/// strict `Deserialize`, and `Deserialize` for `Inbound<Self>` under the inbound policy.
///
/// ```ignore
/// wire_enum! {
///     /// Order side.
///     pub enum TransactionType {
///         /// Buy.
///         Buy => "BUY",
///         /// Sell.
///         Sell => "SELL",
///     }
/// }
/// ```
macro_rules! wire_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident => $wire:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[non_exhaustive]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        $vis enum $name {
            $( $(#[$vmeta])* $variant, )+
        }

        impl $crate::types::WireEnum for $name {
            const ALL: &'static [Self] = &[$(Self::$variant),+];

            fn as_wire(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire,)+
                }
            }

            fn from_wire(s: &str) -> ::std::option::Option<Self> {
                match s {
                    $($wire => ::std::option::Option::Some(Self::$variant),)+
                    _ => ::std::option::Option::None,
                }
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str($crate::types::WireEnum::as_wire(*self))
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = $crate::types::UnknownValue;

            fn from_str(s: &str) -> ::std::result::Result<Self, Self::Err> {
                <Self as $crate::types::WireEnum>::from_wire(s)
                    .ok_or_else(|| $crate::types::UnknownValue::new(s))
            }
        }

        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(&self, s: S) -> ::std::result::Result<S::Ok, S::Error> {
                s.serialize_str($crate::types::WireEnum::as_wire(*self))
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(d: D) -> ::std::result::Result<Self, D::Error> {
                $crate::types::deserialize_strict(d, stringify!($name))
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $crate::types::Inbound<$name> {
            fn deserialize<D: ::serde::Deserializer<'de>>(d: D) -> ::std::result::Result<Self, D::Error> {
                $crate::types::deserialize_inbound(d)
            }
        }
    };
}
pub(crate) use wire_enum;

#[cfg(test)]
mod tests {
    use super::*;

    wire_enum! {
        /// A test enum covering several wire spellings.
        pub(crate) enum Probe {
            /// First.
            Alpha => "ALPHA",
            /// Second, with a lowercase and punctuated wire string.
            BetaGamma => "beta_gamma",
            /// Third.
            Delta => "DELTA-1",
        }
    }

    fn inbound(json: &str) -> Result<Inbound<Probe>, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn exact_string_is_known() {
        assert_eq!(inbound(r#""ALPHA""#).unwrap(), Inbound::Known(Probe::Alpha));
        assert_eq!(
            inbound(r#""beta_gamma""#).unwrap(),
            Inbound::Known(Probe::BetaGamma)
        );
        assert_eq!(inbound(r#""ALPHA""#).unwrap().known(), Some(Probe::Alpha));
    }

    #[test]
    fn other_string_is_unknown_with_its_text() {
        let v = inbound(r#""alpha""#).unwrap();
        assert_eq!(v.known(), None);
        assert_eq!(v.as_wire(), "alpha");
        let Inbound::Unknown(u) = v else {
            panic!("expected Unknown")
        };
        assert_eq!(u.as_str(), "alpha");
        assert!(!u.is_truncated());
    }

    #[test]
    fn long_string_is_truncated_on_a_char_boundary() {
        // 1 KiB of text whose 64-byte cut falls inside a 3-byte character: 63 ASCII bytes, then
        // U+20AC (bytes 63..66), then filler.
        let long = format!("{}\u{20ac}{}", "a".repeat(63), "b".repeat(1024 - 66));
        assert_eq!(long.len(), 1024);
        let Inbound::Unknown(u) = inbound(&format!("\"{long}\"")).unwrap() else {
            panic!("expected Unknown")
        };
        assert_eq!(u.as_str(), "a".repeat(63));
        assert!(u.is_truncated());

        let exact = "c".repeat(64);
        let Inbound::Unknown(u) = inbound(&format!("\"{exact}\"")).unwrap() else {
            panic!("expected Unknown")
        };
        assert_eq!(u.as_str(), exact);
        assert!(!u.is_truncated());

        let over = "d".repeat(65);
        let Inbound::Unknown(u) = inbound(&format!("\"{over}\"")).unwrap() else {
            panic!("expected Unknown")
        };
        assert_eq!(u.as_str(), "d".repeat(64));
        assert!(u.is_truncated());
    }

    #[test]
    fn numbers_and_bools_are_unknown_holding_their_json_text() {
        for (json, text) in [
            ("7", "7"),
            ("-3", "-3"),
            ("1.0", "1.0"),
            ("2.5", "2.5"),
            ("true", "true"),
            ("false", "false"),
        ] {
            let v = inbound(json).unwrap();
            assert_eq!(v.known(), None, "{json}");
            assert_eq!(v.as_wire(), text, "{json}");
        }
    }

    #[test]
    fn arrays_objects_and_bare_null_are_errors() {
        assert!(inbound(r#"["ALPHA"]"#).is_err());
        assert!(inbound(r#"{"v":"ALPHA"}"#).is_err());
        assert!(inbound("null").is_err());
    }

    #[test]
    fn option_of_inbound_sees_null_as_none() {
        #[derive(serde::Deserialize)]
        struct Row {
            #[serde(default)]
            side: Option<Inbound<Probe>>,
        }
        let row: Row = serde_json::from_str(r#"{"side":null}"#).unwrap();
        assert_eq!(row.side, None);
        let row: Row = serde_json::from_str("{}").unwrap();
        assert_eq!(row.side, None);
        let row: Row = serde_json::from_str(r#"{"side":"DELTA-1"}"#).unwrap();
        assert_eq!(row.side, Some(Inbound::Known(Probe::Delta)));
    }

    #[test]
    fn every_variant_serialises_to_its_wire_string_and_round_trips() {
        let expected = [
            (Probe::Alpha, "ALPHA"),
            (Probe::BetaGamma, "beta_gamma"),
            (Probe::Delta, "DELTA-1"),
        ];
        assert_eq!(Probe::ALL, expected.map(|(v, _)| v));
        for (v, wire) in expected {
            assert_eq!(serde_json::to_string(&v).unwrap(), format!("\"{wire}\""));
            assert_eq!(v.to_string(), wire);
            assert_eq!(v.as_wire(), wire);
            assert_eq!(wire.parse::<Probe>(), Ok(v));
            assert_eq!(
                serde_json::from_str::<Probe>(&format!("\"{wire}\"")).unwrap(),
                v
            );
        }
    }

    #[test]
    fn strict_parsing_rejects_unknown_text() {
        let err = "Alpha".parse::<Probe>().unwrap_err();
        assert_eq!(err.as_str(), "Alpha");
        let err = serde_json::from_str::<Probe>(r#""SECRET-ish""#)
            .unwrap_err()
            .to_string();
        assert!(err.starts_with("unknown Probe value"), "{err}");
        assert!(!err.contains("SECRET"), "{err}");
        for json in ["12345", "-7", "1.5", "true"] {
            let err = serde_json::from_str::<Probe>(json).unwrap_err().to_string();
            assert!(err.starts_with("unknown Probe value"), "{json}: {err}");
            assert!(!err.contains(json), "{json}: {err}");
        }
    }

    #[test]
    fn macro_generates_no_conversion_out_of_inbound() {
        // Compiles only if `Probe: TryFrom<Inbound<Probe>>` does not hold: with such an impl the
        // second blanket impl would also apply and the inferred parameter would be ambiguous.
        trait NoFromInbound<A> {
            fn check() {}
        }
        impl<T: ?Sized> NoFromInbound<()> for T {}
        // `TryFrom` also covers `From`, through the standard blanket impl.
        impl<T: TryFrom<Inbound<Probe>>> NoFromInbound<u8> for T {}
        <Probe as NoFromInbound<_>>::check();
    }
}
