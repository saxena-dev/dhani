//! `WireTime`, the IST offset constant and epoch helpers.
//!
//! DhanHQ timestamps arrive in several formats and include sentinels, so a response keeps the
//! raw text in [`WireTime`] and converts on demand with [`WireTime::to_ist`], which never fails a
//! decode.

use std::fmt;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
use serde::Deserialize;
use serde::de::{self, Deserializer, Visitor};

/// Indian Standard Time, UTC+05:30, the zone of every wall-clock time DhanHQ sends.
pub const IST: FixedOffset = match FixedOffset::east_opt(5 * 3600 + 30 * 60) {
    Some(offset) => offset,
    None => panic!("UTC+05:30 is a valid offset"),
};

/// Values that mean "no time" rather than a malformed time.
const SENTINELS: [&str; 3] = [
    "",
    // DOC:6979-6980 (for example "drvExpiryDate": "NA")
    "NA",
    // DOC:6155 ("ExpiryDate": "0001-01-01 00:00:00")
    "0001-01-01 00:00:00",
];

/// A timestamp exactly as received. A JSON number is kept as its decimal text.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WireTime(String);

impl WireTime {
    /// The raw text as received.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The time in IST, parsing exactly these formats:
    ///
    /// - `YYYY-MM-DD HH:MM:SS`, an IST wall-clock time (DOC:6286);
    /// - RFC 3339, for example `2019-08-24T14:15:22Z` (DOC:5048-5082), converted to IST;
    /// - `YYYY-MM-DD`, taken as midnight IST.
    ///
    /// Returns `None` for the sentinels `""`, `"NA"` and `"0001-01-01 00:00:00"`, and for
    /// anything else (for example the placeholder `"string"`).
    pub fn to_ist(&self) -> Option<DateTime<FixedOffset>> {
        let s = self.0.as_str();
        if SENTINELS.contains(&s) {
            return None;
        }
        if let Ok(local) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
            return local.and_local_timezone(IST).single();
        }
        if let Ok(t) = DateTime::parse_from_rfc3339(s) {
            return Some(t.with_timezone(&IST));
        }
        if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
            return date
                .and_time(NaiveTime::MIN)
                .and_local_timezone(IST)
                .single();
        }
        None
    }

    /// The IST calendar date of [`to_ist`](WireTime::to_ist).
    pub fn to_date(&self) -> Option<NaiveDate> {
        self.to_ist().map(|t| t.date_naive())
    }
}

impl<'de> Deserialize<'de> for WireTime {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct WireTimeVisitor;

        impl Visitor<'_> for WireTimeVisitor {
            type Value = WireTime;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a timestamp string or number")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<WireTime, E> {
                Ok(WireTime(v.to_owned()))
            }

            fn visit_string<E: de::Error>(self, v: String) -> Result<WireTime, E> {
                Ok(WireTime(v))
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<WireTime, E> {
                Ok(WireTime(v.to_string()))
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<WireTime, E> {
                Ok(WireTime(v.to_string()))
            }

            fn visit_f64<E: de::Error>(self, v: f64) -> Result<WireTime, E> {
                let text = serde_json::Number::from_f64(v)
                    .map_or_else(|| v.to_string(), |n| n.to_string());
                Ok(WireTime(text))
            }
        }

        d.deserialize_any(WireTimeVisitor)
    }
}

/// Converts Unix epoch seconds to IST; `None` if out of chrono's range.
pub fn epoch_to_ist(secs: i64) -> Option<DateTime<FixedOffset>> {
    DateTime::from_timestamp(secs, 0).map(|t| t.with_timezone(&IST))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wt(s: &str) -> WireTime {
        WireTime(s.to_owned())
    }

    fn ist(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<FixedOffset> {
        NaiveDate::from_ymd_opt(y, mo, d)
            .unwrap()
            .and_hms_opt(h, mi, s)
            .unwrap()
            .and_local_timezone(IST)
            .unwrap()
    }

    #[test]
    fn ist_is_plus_five_thirty() {
        assert_eq!(IST.local_minus_utc(), 19_800);
    }

    #[test]
    fn wall_clock_format_is_ist() {
        let t = wt("2024-09-11 14:39:29").to_ist().unwrap();
        assert_eq!(t, ist(2024, 9, 11, 14, 39, 29));
        assert_eq!(t.to_rfc3339(), "2024-09-11T14:39:29+05:30");
    }

    #[test]
    fn rfc3339_is_converted_to_ist() {
        let t = wt("2019-08-24T14:15:22Z").to_ist().unwrap();
        assert_eq!(t.to_rfc3339(), "2019-08-24T19:45:22+05:30");
        assert_eq!(t.offset(), &IST);
        let t = wt("2019-08-24T14:15:22+05:30").to_ist().unwrap();
        assert_eq!(t, ist(2019, 8, 24, 14, 15, 22));
    }

    #[test]
    fn date_only_is_midnight_ist() {
        let t = wt("2024-09-11").to_ist().unwrap();
        assert_eq!(t.to_rfc3339(), "2024-09-11T00:00:00+05:30");
        assert_eq!(
            wt("2024-09-11").to_date(),
            NaiveDate::from_ymd_opt(2024, 9, 11)
        );
        // An RFC 3339 time late in the UTC day falls on the next IST date.
        assert_eq!(
            wt("2024-09-11T20:00:00Z").to_date(),
            NaiveDate::from_ymd_opt(2024, 9, 12)
        );
    }

    #[test]
    fn sentinels_and_other_text_are_none() {
        for s in [
            "",
            "NA",
            "0001-01-01 00:00:00",
            "string",
            "2024-13-01",
            "11/09/2024",
            "1756698300",
        ] {
            assert_eq!(wt(s).to_ist(), None, "{s:?}");
            assert_eq!(wt(s).to_date(), None, "{s:?}");
        }
    }

    #[test]
    fn deserialises_strings_and_stringifies_numbers() {
        let t: WireTime = serde_json::from_str("1756698300").unwrap();
        assert_eq!(t, wt("1756698300"));
        assert_eq!(t.as_str(), "1756698300");
        let t: WireTime = serde_json::from_str("-5").unwrap();
        assert_eq!(t.as_str(), "-5");
        let t: WireTime = serde_json::from_str("1.5").unwrap();
        assert_eq!(t.as_str(), "1.5");
        let t: WireTime = serde_json::from_str(r#""2024-09-11 14:39:29""#).unwrap();
        assert_eq!(t.as_str(), "2024-09-11 14:39:29");
        assert!(serde_json::from_str::<WireTime>("true").is_err());
        assert!(serde_json::from_str::<WireTime>("[]").is_err());
        let o: Option<WireTime> = serde_json::from_str("null").unwrap();
        assert_eq!(o, None);
    }

    #[test]
    fn epoch_seconds_convert_to_ist() {
        let t = epoch_to_ist(0).unwrap();
        assert_eq!(t.to_rfc3339(), "1970-01-01T05:30:00+05:30");
        assert_eq!(
            epoch_to_ist(1_756_698_300).unwrap().to_rfc3339(),
            "2025-09-01T09:15:00+05:30"
        );
        assert_eq!(epoch_to_ist(i64::MAX), None);
    }
}
