use super::*;

const COMPACT: &str = "\
SEM_EXM_EXCH_ID,SEM_SMST_SECURITY_ID,SEM_SEGMENT,SEM_INSTRUMENT_NAME,SEM_EXPIRY_CODE,SM_SYMBOL_NAME,SEM_TRADING_SYMBOL,SEM_CUSTOM_SYMBOL,SEM_EXCH_INSTRUMENT_TYPE,SEM_SERIES,SEM_LOT_UNITS,SEM_EXPIRY_DATE,SEM_STRIKE_PRICE,SEM_OPTION_TYPE,SEM_TICK_SIZE,SEM_EXPIRY_FLAG,SEM_NEW_COLUMN
NSE,1333,E,EQUITY,0,HDFC BANK LTD,HDFCBANK,HDFC Bank,ES,EQ,1.0,,,XX,5.0,NA,z
";

#[test]
fn a_compact_row_maps_every_typed_column() {
    let rows = parse_scrip_master(COMPACT).unwrap();
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(r.exchange.as_deref(), Some("NSE"));
    assert_eq!(r.security_id, Some(SecurityId::new("1333").unwrap()));
    assert_eq!(r.segment.as_deref(), Some("E"));
    assert_eq!(r.instrument.as_deref(), Some("EQUITY"));
    assert_eq!(r.expiry_code.as_deref(), Some("0"));
    assert_eq!(r.symbol_name.as_deref(), Some("HDFC BANK LTD"));
    assert_eq!(r.trading_symbol.as_deref(), Some("HDFCBANK"));
    assert_eq!(r.display_name.as_deref(), Some("HDFC Bank"));
    assert_eq!(r.instrument_type.as_deref(), Some("ES"));
    assert_eq!(r.series.as_deref(), Some("EQ"));
    assert_eq!((r.lot_size, r.tick_size), (Some(1.0), Some(5.0)));
    // Empty cells are None.
    assert_eq!((r.expiry_date.clone(), r.strike_price), (None, None));
    assert_eq!(
        (r.option_type.as_deref(), r.expiry_flag.as_deref()),
        (Some("XX"), Some("NA"))
    );
    // Columns the record does not type are kept by header.
    assert_eq!(r.extra.get("SEM_NEW_COLUMN").map(String::as_str), Some("z"));
    assert_eq!(r.extra.len(), 1);
    assert_eq!((r.isin.clone(), r.underlying_symbol.clone()), (None, None));
}

#[test]
fn a_detailed_row_maps_its_column_names() {
    let csv = "\
EXCH_ID,SEGMENT,SECURITY_ID,ISIN,INSTRUMENT,UNDERLYING_SECURITY_ID,UNDERLYING_SYMBOL,SYMBOL_NAME,DISPLAY_NAME,INSTRUMENT_TYPE,SERIES,LOT_SIZE,SM_EXPIRY_DATE,STRIKE_PRICE,OPTION_TYPE,TICK_SIZE,EXPIRY_FLAG,BRACKET_FLAG,SELL_BO_SL_MIN_RANGE
NSE,D,52175,NA,OPTIDX,13,NIFTY,NIFTY-Oct2024-25000-CE,NIFTY 31 OCT 25000 CALL,OP,NA,25,2024-10-31 14:30:00,25000.00,CE,0.05,W,N,1.5
";
    let r = &parse_scrip_master(csv).unwrap()[0];
    assert_eq!(r.security_id, Some(SecurityId::new("52175").unwrap()));
    assert_eq!(r.isin.as_ref().map(|i| i.as_ref()), Some("NA"));
    assert_eq!(
        (
            r.underlying_security_id.as_deref(),
            r.underlying_symbol.as_deref()
        ),
        (Some("13"), Some("NIFTY"))
    );
    assert_eq!(r.display_name.as_deref(), Some("NIFTY 31 OCT 25000 CALL"));
    assert_eq!(
        r.expiry_date.as_ref().map(WireTime::as_str),
        Some("2024-10-31 14:30:00")
    );
    assert_eq!(
        (r.lot_size, r.strike_price, r.tick_size),
        (Some(25.0), Some(25000.0), Some(0.05))
    );
    assert_eq!(r.extra.get("BRACKET_FLAG").map(String::as_str), Some("N"));
    assert_eq!(
        r.extra.get("SELL_BO_SL_MIN_RANGE").map(String::as_str),
        Some("1.5")
    );
}

#[test]
fn a_malformed_row_reports_only_its_index() {
    let wrong_count = "SEM_EXM_EXCH_ID,SEM_SERIES\n1,2\n3\n";
    assert_eq!(
        parse_scrip_master(wrong_count),
        Err(MalformedRow { row: 2 })
    );
    let short = "SEM_EXM_EXCH_ID,SEM_SERIES\n1\n";
    assert_eq!(parse_scrip_master(short), Err(MalformedRow { row: 1 }));
    let bad_id = "SEM_SMST_SECURITY_ID\n1\n2\n".to_owned() + &"x".repeat(200) + "\n";
    assert_eq!(parse_scrip_master(&bad_id), Err(MalformedRow { row: 3 }));
}

#[test]
fn headers_are_trimmed_and_a_bom_is_ignored() {
    let csv = "\u{feff}SEM_EXM_EXCH_ID , SEM_SMST_SECURITY_ID\n NSE , 1333 \n";
    let r = &parse_scrip_master(csv).unwrap()[0];
    assert_eq!(
        (
            r.exchange.as_deref(),
            r.security_id.as_ref().map(|s| s.as_ref())
        ),
        (Some("NSE"), Some("1333"))
    );
    assert!(r.extra.is_empty());
}

#[test]
fn an_empty_file_has_no_records() {
    // No known column: an empty body or an HTML error page is not a scrip master.
    assert_eq!(parse_scrip_master(""), Err(MalformedRow { row: 0 }));
    assert_eq!(
        parse_scrip_master("<html>\n<body>Error</body>\n</html>\n"),
        Err(MalformedRow { row: 0 })
    );
    assert_eq!(parse_scrip_master("SEM_EXM_EXCH_ID\n"), Ok(Vec::new()));
}

#[test]
fn a_non_numeric_number_cell_is_none_with_its_text_in_extra() {
    let csv = "SEM_EXM_EXCH_ID,SEM_LOT_UNITS,SEM_STRIKE_PRICE,SEM_TICK_SIZE\nNSE,NA,-0.01000,x\n";
    let rows = parse_scrip_master(csv).unwrap();
    let r = &rows[0];
    assert_eq!(
        (r.lot_size, r.strike_price, r.tick_size),
        (None, Some(-0.01), None)
    );
    assert_eq!(r.extra.get("SEM_LOT_UNITS").map(String::as_str), Some("NA"));
    assert_eq!(r.extra.get("SEM_TICK_SIZE").map(String::as_str), Some("x"));
    assert_eq!(r.extra.len(), 2);
}

#[test]
fn records_share_one_allocation_per_header() {
    let csv = "SEM_EXM_EXCH_ID,BRACKET_FLAG\nNSE,N\nBSE,Y\n";
    let rows = parse_scrip_master(csv).unwrap();
    let key = |i: usize| rows[i].extra.keys().next().unwrap().clone();
    assert!(std::sync::Arc::ptr_eq(&key(0), &key(1)));
}
