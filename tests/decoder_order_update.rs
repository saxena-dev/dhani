//! The order-update decoder against the documentation's verbatim sample message.

mod support;

use dhani::decoder::{
    DecodeErrorKind, OrderUpdate, OrderUpdateEvent, parse_order_update, parse_order_update_frame,
};
use dhani::types::{Inbound, OrderStatus, ProductType};

const SAMPLE: &str = "tests/fixtures/synth/order_update.json";

fn sample_order() -> OrderUpdate {
    let bytes = support::fixtures::raw_bytes(SAMPLE);
    match parse_order_update(&bytes).expect("the sample parses") {
        OrderUpdateEvent::Order(order) => *order,
        other => panic!("expected an order alert, got {other:?}"),
    }
}

#[test]
fn the_documentation_sample_decodes() {
    let o = sample_order();
    assert_eq!(o.status, Some(Inbound::Known(OrderStatus::Cancelled)));
    assert_eq!(o.price, Some(13.0));
    // The sample repeats Remarks; the last occurrence wins.
    assert_eq!(o.remarks.as_deref(), Some("Super Order"));
    assert_eq!(
        (
            o.exchange.as_deref(),
            o.segment.as_deref(),
            o.security_id.as_deref()
        ),
        (Some("NSE"), Some("E"), Some("14366"))
    );
    assert_eq!(
        (o.order_no.as_deref(), o.exch_order_no.as_deref()),
        (Some("1124091136546"), Some("1400000000404591"))
    );
    assert_eq!(
        (
            o.product.as_deref(),
            o.product(),
            o.txn_type.as_deref(),
            o.order_type.as_deref()
        ),
        (Some("C"), Some(ProductType::Cnc), Some("B"), Some("LMT"))
    );
    assert_eq!(
        (
            o.quantity,
            o.traded_qty,
            o.remaining_quantity,
            o.disc_quantity,
            o.disc_qty_rem
        ),
        (Some(1), Some(0), Some(1), Some(1), Some(1))
    );
    assert_eq!(
        (
            o.trigger_price,
            o.traded_price,
            o.avg_traded_price,
            o.ref_ltp,
            o.tick_size
        ),
        (Some(0.0), Some(0.0), Some(0.0), Some(13.21), Some(0.01))
    );
    assert_eq!(
        (o.leg_no, o.lot_size, o.multiplier),
        (Some(1), Some(1), Some(1))
    );
    assert_eq!(
        o.order_date_time.as_ref().map(|t| t.as_str()),
        Some("2024-09-11 14:39:29")
    );
    assert_eq!(
        o.good_till_days_date.as_ref().map(|t| t.as_str()),
        Some("2024-09-11")
    );
    assert_eq!(
        (
            o.symbol.as_deref(),
            o.display_name.as_deref(),
            o.isin.as_deref()
        ),
        (Some("IDEA"), Some("Vodafone Idea"), Some("INE669E01016"))
    );
    assert_eq!(
        (
            o.off_mkt_flag.as_deref(),
            o.opt_type.as_deref(),
            o.correlation_id.as_deref()
        ),
        (Some("0"), Some("XX"), Some(""))
    );
    assert_eq!((o.strike_price, o.algo_ord_no), (None, None));
}

#[test]
fn the_client_id_is_decoded_and_redacted() {
    let o = sample_order();
    assert!(o.client_id.is_some());
    let debug = format!("{:?}", o.client_id);
    assert_eq!(debug, "Some(ClientId(<redacted>))");
    let whole = format!("{o:?}");
    assert!(!whole.contains("<your-client-id>"), "{whole}");
}

#[test]
fn other_message_types_are_kept_whole() {
    match parse_order_update(br#"{"Type":"x","Data":{}}"#).unwrap() {
        OrderUpdateEvent::Other { kind, raw } => {
            assert_eq!(kind.as_deref(), Some("x"));
            assert_eq!(raw.0, serde_json::json!({"Type": "x", "Data": {}}));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_binary_frame_is_unexpected() {
    let bytes = support::fixtures::raw_bytes(SAMPLE);
    let err = parse_order_update_frame(true, &bytes).unwrap_err();
    assert_eq!(
        (err.kind, err.offset, err.packet_code),
        (DecodeErrorKind::UnexpectedBinary, 0, None)
    );
    assert!(parse_order_update_frame(false, &bytes).is_ok());
}
