use serde_json::json;

use super::*;

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn daily() -> DailyRequest {
    DailyRequest::new(
        ExchangeSegment::NseEq,
        SecurityId::new("1333").unwrap(),
        InstrumentKind::Equity,
        date(2024, 1, 1),
        date(2024, 2, 1),
    )
}

fn intraday(interval: IntradayInterval) -> IntradayRequest {
    IntradayRequest::new(
        ExchangeSegment::NseFno,
        SecurityId::new("52175").unwrap(),
        InstrumentKind::Optidx,
        interval,
        date(2024, 9, 11),
        date(2024, 9, 15),
    )
}

fn invalid(field: &'static str, reason: ValidationReason) -> Result<(), ValidationError> {
    Err(ValidationError { field, reason })
}

#[test]
fn a_daily_body_omits_an_unset_expiry_and_sends_oi_as_a_bool() {
    assert_eq!(
        serde_json::to_value(daily()).unwrap(),
        json!({
            "securityId": "1333",
            "exchangeSegment": "NSE_EQ",
            "instrument": "EQUITY",
            "oi": false,
            "fromDate": "2024-01-01",
            "toDate": "2024-02-01"
        })
    );
    let body =
        serde_json::to_value(daily().with_expiry_code(ExpiryCode::Next).with_oi(true)).unwrap();
    assert_eq!(
        (&body["expiryCode"], &body["oi"]),
        (&json!(2), &json!(true))
    );
}

#[test]
fn an_intraday_body_sends_the_interval_as_an_integer() {
    assert_eq!(
        serde_json::to_value(intraday(IntradayInterval::Min5)).unwrap(),
        json!({
            "securityId": "52175",
            "exchangeSegment": "NSE_FNO",
            "instrument": "OPTIDX",
            "interval": 5,
            "oi": false,
            "fromDate": "2024-09-11",
            "toDate": "2024-09-15"
        })
    );
    let minutes: Vec<_> = [
        IntradayInterval::Min1,
        IntradayInterval::Min5,
        IntradayInterval::Min15,
        IntradayInterval::Min25,
        IntradayInterval::Min60,
    ]
    .map(|i| serde_json::to_value(intraday(i)).unwrap()["interval"].clone())
    .to_vec();
    assert_eq!(
        minutes,
        [json!(1), json!(5), json!(15), json!(25), json!(60)]
    );
}

#[test]
fn date_ranges() {
    assert_eq!(daily().validate(), Ok(()));
    let mut same_day = daily();
    same_day.to_date = same_day.from_date;
    let expected = invalid(
        "from_date",
        ValidationReason::Inconsistent("must be before to_date (to_date is not inclusive)"),
    );
    assert_eq!(same_day.validate(), expected);
    let mut reversed = daily();
    reversed.from_date = date(2024, 3, 1);
    assert_eq!(reversed.validate(), expected);
    // Intraday allows a single day but not a reversed range.
    let mut one_day = intraday(IntradayInterval::Min1);
    one_day.to_date = one_day.from_date;
    assert_eq!(one_day.validate(), Ok(()));
    let mut backwards = intraday(IntradayInterval::Min1);
    backwards.from_date = date(2024, 9, 16);
    assert_eq!(
        backwards.validate(),
        invalid(
            "from_date",
            ValidationReason::Inconsistent("must not be after to_date")
        )
    );
    let mut far = daily();
    far.to_date = date(10_000, 1, 1);
    assert_eq!(
        far.validate(),
        invalid("to_date", ValidationReason::OutOfRange)
    );
}

#[test]
fn only_the_chart_segments_are_accepted() {
    for segment in [
        ExchangeSegment::NseCurrency,
        ExchangeSegment::NseComm,
        ExchangeSegment::InxEq,
    ] {
        let mut req = daily();
        req.exchange_segment = segment;
        assert_eq!(
            req.validate(),
            invalid("exchange_segment", ValidationReason::UnknownEnumValue)
        );
    }
    let mut index = daily();
    index.exchange_segment = ExchangeSegment::IdxI;
    assert_eq!(index.validate(), Ok(()));
}

#[test]
fn candles_decode_columns_and_iterate() {
    let candles: Candles = serde_json::from_value(json!({
        "open": [1.5, 2.5],
        "high": [2.5, 3.5],
        "low": [1.0, 2.0],
        "close": [2.0, 3.0],
        "volume": [100, "200"],
        "open_interest": [10.0, 20],
        "timestamp": [1756698300, 1756699200]
    }))
    .unwrap();
    assert_eq!(candles.volume, [100, 200]);
    assert_eq!(candles.open_interest, [10, 20]);
    let all: Vec<Candle> = candles.iter().collect();
    assert_eq!(
        all[0],
        Candle {
            ts: 1756698300,
            open: 1.5,
            high: 2.5,
            low: 1.0,
            close: 2.0,
            volume: 100,
            open_interest: Some(10)
        }
    );
    assert_eq!(
        (all[1].ts, all[1].close, all[1].volume),
        (1756699200, 3.0, 200)
    );
    // 2025-09-01 09:15:00 IST.
    assert_eq!(
        all[0].time_ist().unwrap().to_rfc3339(),
        "2025-09-01T09:15:00+05:30"
    );
}

#[test]
fn empty_and_null_columns_are_empty() {
    let empty: Candles = serde_json::from_value(json!({})).unwrap();
    assert!(empty.is_empty() && empty.iter().next().is_none());
    let nulls: Candles = serde_json::from_value(json!({"open": null, "timestamp": null})).unwrap();
    assert_eq!(nulls, Candles::default());
    // Open interest may be absent while the rest is present.
    let no_oi: Candles = serde_json::from_value(json!({
        "open": [1.0], "high": [1.0], "low": [1.0], "close": [1.0],
        "volume": [1], "timestamp": [1]
    }))
    .unwrap();
    assert_eq!(no_oi.iter().next().unwrap().open_interest, None);
}

#[test]
fn malformed_candles_fail_to_decode() {
    for body in [
        // A scalar where an array belongs.
        json!({"open": 1.5}),
        // Unequal lengths.
        json!({
            "open": [1.0, 2.0], "high": [1.0], "low": [1.0], "close": [1.0],
            "volume": [1], "timestamp": [1]
        }),
        // Timestamps without prices.
        json!({"volume": [1], "timestamp": [1]}),
        // Prices without timestamps.
        json!({"open": [1.0]}),
        // An open-interest column of another length.
        json!({
            "open": [1.0], "high": [1.0], "low": [1.0], "close": [1.0],
            "volume": [1], "open_interest": [1, 2], "timestamp": [1]
        }),
        // A scalar in an integer column.
        json!({"volume": 5}),
        // A non-integral volume.
        json!({
            "open": [1.0], "high": [1.0], "low": [1.0], "close": [1.0],
            "volume": [1.5], "timestamp": [1]
        }),
    ] {
        assert!(
            serde_json::from_value::<Candles>(body.clone()).is_err(),
            "{body}"
        );
    }
}

#[test]
fn iteration_stops_at_a_short_column_instead_of_panicking() {
    let mut candles: Candles = serde_json::from_value(json!({
        "open": [1.0, 2.0], "high": [1.0, 2.0], "low": [1.0, 2.0], "close": [1.0, 2.0],
        "volume": [1, 2], "timestamp": [1, 2]
    }))
    .unwrap();
    candles.close.pop();
    assert_eq!(candles.iter().count(), 1);
    assert_eq!(Candles::default().iter().count(), 0);
}
