//! Instrument master contract tests (§9 rows I1–I2): the CSV is fetched from its absolute URL
//! without credentials and parsed by header name; a malformed row is a Decode error naming only
//! the row.

mod support;

use dhani::ErrorKind;
use dhani::rest::ScripMasterKind;
use support::fixtures::synth;
use support::mock::client_for;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn serve(route: &str, body: Vec<u8>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(route))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/csv")
                .set_body_bytes(body),
        )
        .expect(1)
        .mount(&server)
        .await;
    server
}

/// The one request carried no credential headers.
async fn assert_no_credentials(server: &MockServer) {
    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    for name in ["access-token", "client-id"] {
        assert!(received[0].headers.get(name).is_none(), "{name}");
    }
}

// row: I1
#[tokio::test]
async fn i1_compact_scrip_master() {
    let server = serve(
        "/csv/api-scrip-master.csv",
        synth("scrip_master_compact.csv"),
    )
    .await;
    let rows = client_for(&server)
        .instruments()
        .scrip_master(ScripMasterKind::Compact)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    let hdfc = &rows[1];
    assert_eq!(hdfc.security_id.as_ref().map(|s| s.as_ref()), Some("1333"));
    assert_eq!(hdfc.exchange.as_deref(), Some("NSE"));
    assert_eq!(hdfc.instrument.as_deref(), Some("EQUITY"));
    assert_eq!(hdfc.trading_symbol.as_deref(), Some("HDFCBANK"));
    assert_eq!(hdfc.display_name.as_deref(), Some("HDFC Bank Ltd"));
    assert_eq!(
        (hdfc.lot_size, hdfc.strike_price, hdfc.tick_size),
        (Some(1.0), Some(1.5), Some(1.5))
    );
    assert_eq!(
        hdfc.expiry_date.as_ref().map(|t| t.as_str()),
        Some("2024-09-11 14:39:29")
    );
    assert_eq!(
        (hdfc.option_type.as_deref(), hdfc.expiry_flag.as_deref()),
        (Some("CE"), Some("M"))
    );
    // The placeholder row decodes; its ID only fails if reused.
    assert_eq!(
        rows[0].security_id.as_ref().map(|s| s.as_ref()),
        Some("string")
    );
    assert!(hdfc.extra.is_empty());
    assert_no_credentials(&server).await;
}

// row: I2
#[tokio::test]
async fn i2_detailed_scrip_master_keeps_unknown_columns_in_extra() {
    let server = serve(
        "/csv/api-scrip-master-detailed.csv",
        synth("scrip_master_detailed.csv"),
    )
    .await;
    let rows = client_for(&server)
        .instruments()
        .scrip_master(ScripMasterKind::Detailed)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    let r = &rows[1];
    assert_eq!(
        (
            r.exchange.as_deref(),
            r.segment.as_deref(),
            r.instrument.as_deref()
        ),
        (Some("BSE"), Some("D"), Some("FUTIDX"))
    );
    assert_eq!(
        (r.option_type.as_deref(), r.expiry_flag.as_deref()),
        (Some("PE"), Some("W"))
    );
    // The detailed file has no security-ID column (OQ-14).
    assert_eq!(r.security_id, None);
    assert_eq!(r.isin.as_ref().map(|i| i.as_ref()), Some("string"));
    for (column, value) in [
        ("BRACKET_FLAG", "Y"),
        ("COVER_FLAG", "Y"),
        ("ASM_GSM_FLAG", "R"),
        ("SELL_BO_SL_MIN_RANGE", "1.5"),
        ("MTF_LEVERAGE", "1.5"),
    ] {
        assert_eq!(
            r.extra.get(column).map(String::as_str),
            Some(value),
            "{column}"
        );
    }
    assert_eq!(r.extra.len(), 22);
    assert_no_credentials(&server).await;
}

#[tokio::test]
async fn a_malformed_second_row_is_a_decode_error_naming_only_the_row() {
    let csv = "SEM_EXM_EXCH_ID,SEM_SMST_SECURITY_ID\nNSE,1333\nNSE,1334,SECRET-EXTRA\n";
    let server = serve("/csv/api-scrip-master.csv", csv.as_bytes().to_vec()).await;
    let err = client_for(&server)
        .instruments()
        .scrip_master(ScripMasterKind::Compact)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Decode);
    assert_eq!(err.detail(), Some("row 2"));
    // Parsed inside the call, so the error carries the response's status and attempt count.
    assert_eq!((err.http_status(), err.attempts()), (Some(200), 1));
    assert!(!format!("{err:?}").contains("SECRET-EXTRA"));
}
