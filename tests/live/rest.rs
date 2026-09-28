//! The REST lanes: the sandbox suite (`sandbox_*`, one test per sandbox-enabled endpoint), the
//! live read-only suite (`live_ro_*`) and the single live mutation (`live_mut_*`).
//!
//! Every order placed here carries a correlation ID unique to the run and is cancelled before
//! the test asserts, so a failing check never leaves an order behind. A placement that fails
//! after it may have reached Dhan is looked up by that correlation ID and cancelled too.

use std::future::Future;
use std::time::Duration;

use chrono::{Days, Utc};
use dhani::config::Environment;
use dhani::rest::{
    Candles, ConvertPositionRequest, DailyRequest, IntradayInterval, IntradayRequest,
    MarginRequest, ModifyOrderRequest, OptionChainRequest, PlaceOrderRequest, QuoteRequest,
    UnderlyingRef,
};
use dhani::types::{
    AmoTime, CorrelationId, ExchangeSegment, InstrumentKind, OrderId, OrderStatus, OrderType,
    PositionType, ProductType, RawJson, SecurityId, TransactionType, Validity,
};
use dhani::{Credentials, DhanClient, ErrorKind};
use tokio::sync::MutexGuard;

use crate::lane::{capture_dir, env, live_credentials, sandbox_credentials, serial, today};

/// A LIMIT price far below HDFC Bank's market price, so a sandbox order stays pending. The
/// sandbox has no market quote to derive it from.
const SANDBOX_LIMIT_PRICE: f64 = 500.0;

/// A client and the lock that keeps other tests from calling Dhan meanwhile.
type Lane = (DhanClient, MutexGuard<'static, ()>);

async fn lane(env: Environment, credentials: Option<Credentials>) -> Option<Lane> {
    let credentials = credentials?;
    let guard = serial().await;
    let client = DhanClient::builder()
        .environment(env)
        .credentials(credentials)
        .build()
        .expect("a client");
    Some((client, guard))
}

async fn sandbox() -> Option<Lane> {
    lane(Environment::Sandbox, sandbox_credentials()).await
}

async fn live() -> Option<Lane> {
    lane(Environment::Live, live_credentials()).await
}

/// HDFC Bank on NSE (`NSE_EQ:1333`).
fn hdfc_bank() -> SecurityId {
    SecurityId::new("1333").expect("a security ID")
}

/// One HDFC Bank share, bought with a LIMIT order for delivery, tagged `<tag>-<unix seconds>`.
fn limit_buy(price: f64, tag: &str) -> PlaceOrderRequest {
    let correlation_id =
        CorrelationId::new(format!("{tag}-{}", Utc::now().timestamp())).expect("an ID");
    PlaceOrderRequest::new(
        ExchangeSegment::NseEq,
        hdfc_bank(),
        TransactionType::Buy,
        1,
        OrderType::Limit,
        ProductType::Cnc,
        Validity::Day,
    )
    .with_price(price)
    .with_correlation_id(correlation_id)
}

/// How hard to make sure an order is gone.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Cleanup {
    /// Cancel once and print a failure: the sandbox holds no real money.
    BestEffort,
    /// Retry the cancel, then read the order back and panic unless it is closed.
    Strict,
}

/// Places `order`, runs `body` with its ID, and cancels the order before returning the body's
/// result. Callers assert on the result.
async fn with_placed_order<T, F, Fut>(
    client: &DhanClient,
    order: PlaceOrderRequest,
    cleanup: Cleanup,
    body: F,
) -> T
where
    F: FnOnce(OrderId) -> Fut,
    Fut: Future<Output = T>,
{
    let ack = match client.orders().place(&order).await {
        Ok(ack) => ack,
        Err(e) => {
            if e.may_have_reached_server() {
                cancel_tagged(client, &order, cleanup).await;
            }
            panic!("place the order: {e}");
        }
    };
    println!("placed order {} ({:?})", ack.order_id, ack.order_status);
    let result = body(ack.order_id.clone()).await;
    cancel(client, &ack.order_id, cleanup).await;
    result
}

/// Cancels every order in the book that carries `order`'s correlation ID. Under
/// [`Cleanup::Strict`] the book is read up to three times, a second apart, since a just-placed
/// order may not be listed yet; finding none is then reported as a possibly open order.
async fn cancel_tagged(client: &DhanClient, order: &PlaceOrderRequest, cleanup: Cleanup) {
    let tag = order
        .correlation_id
        .as_ref()
        .expect("every order here is tagged")
        .to_string();
    let attempts = if cleanup == Cleanup::Strict { 3 } else { 1 };
    for attempt in 1..=attempts {
        let orders = match client.orders().list().await {
            Ok(orders) => orders,
            Err(e) if cleanup == Cleanup::Strict => {
                panic!("could not list orders to find {tag}: {e}; check the order book by hand")
            }
            Err(e) => return println!("could not list orders to find {tag}: {e}"),
        };
        let tagged: Vec<OrderId> = orders
            .into_iter()
            .filter(|o| o.correlation_id.as_deref() == Some(tag.as_str()))
            .map(|o| o.order_id)
            .collect();
        if !tagged.is_empty() {
            for id in &tagged {
                cancel(client, id, cleanup).await;
            }
            return;
        }
        if attempt < attempts {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    if cleanup == Cleanup::Strict {
        panic!(
            "no order tagged {tag} was found, but one may be open: check the order book by hand"
        );
    }
}

/// Cancels `order_id` as `cleanup` says.
async fn cancel(client: &DhanClient, order_id: &OrderId, cleanup: Cleanup) {
    if cleanup == Cleanup::BestEffort {
        match client.orders().cancel(order_id).await {
            Ok(ack) => println!("cancelled order {} ({:?})", ack.order_id, ack.order_status),
            Err(e) => println!("cancel of order {order_id} failed: {e}"),
        }
        return;
    }
    for attempt in 1..=3 {
        match client.orders().cancel(order_id).await {
            Ok(ack) => {
                println!("cancelled order {} ({:?})", ack.order_id, ack.order_status);
                break;
            }
            Err(e) => {
                println!("cancel attempt {attempt} of order {order_id} failed: {e}");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
    let status = client
        .orders()
        .get(order_id)
        .await
        .ok()
        .and_then(|o| o.order_status)
        .and_then(|s| s.known());
    assert!(
        matches!(status, Some(OrderStatus::Cancelled | OrderStatus::Rejected)),
        "order {order_id} may still be open ({status:?}): cancel it by hand"
    );
}

/// Writes `raw` to `target/captures/rest/<name>-<date>.json`.
fn dump(name: &str, raw: &RawJson) {
    let path = capture_dir("rest").join(format!("{name}-{}.json", today()));
    let body = serde_json::to_vec_pretty(&raw.0).expect("serialise the capture");
    std::fs::write(&path, body).expect("write the capture");
    println!("wrote {}", path.display());
}

// ---- Sandbox: one test per sandbox-enabled row. ----

#[tokio::test]
#[ignore = "needs sandbox credentials"]
async fn sandbox_o1_place_order() {
    let Some((client, _serial)) = sandbox().await else {
        return;
    };
    let order = limit_buy(SANDBOX_LIMIT_PRICE, "dhani-o1");
    let status = with_placed_order(&client, order, Cleanup::BestEffort, async |id| {
        client.orders().get(&id).await.map(|o| o.order_status)
    })
    .await;
    println!("order status: {:?}", status.expect("read back the order"));
}

/// One share never needs more than one slice, so this checks the call and the list-shaped
/// answer, not the multi-slice path.
#[tokio::test]
#[ignore = "needs sandbox credentials"]
async fn sandbox_o2_place_sliced_order() {
    let Some((client, _serial)) = sandbox().await else {
        return;
    };
    let order = limit_buy(SANDBOX_LIMIT_PRICE, "dhani-o2");
    let acks = match client.orders().place_sliced(&order).await {
        Ok(acks) => acks,
        Err(e) => {
            if e.may_have_reached_server() {
                cancel_tagged(&client, &order, Cleanup::BestEffort).await;
            }
            panic!("place the sliced order: {e}");
        }
    };
    for ack in &acks {
        cancel(&client, &ack.order_id, Cleanup::BestEffort).await;
    }
    assert!(
        !acks.is_empty(),
        "a sliced order returns at least one order ID"
    );
}

#[tokio::test]
#[ignore = "needs sandbox credentials"]
async fn sandbox_o3_modify_order() {
    let Some((client, _serial)) = sandbox().await else {
        return;
    };
    let order = limit_buy(SANDBOX_LIMIT_PRICE, "dhani-o3");
    let modified = with_placed_order(&client, order, Cleanup::BestEffort, async |id| {
        let req = ModifyOrderRequest::new(id, OrderType::Limit, Validity::Day)
            .with_quantity(1)
            .with_price(SANDBOX_LIMIT_PRICE - 1.0);
        client.orders().modify(&req).await
    })
    .await;
    let ack = modified.expect("modify the order");
    println!("modified order {} ({:?})", ack.order_id, ack.order_status);
}

#[tokio::test]
#[ignore = "needs sandbox credentials"]
async fn sandbox_o4_cancel_order() {
    let Some((client, _serial)) = sandbox().await else {
        return;
    };
    let order = limit_buy(SANDBOX_LIMIT_PRICE, "dhani-o4");
    // The body cancels; the cleanup's second cancel only prints its refusal.
    let cancelled = with_placed_order(&client, order, Cleanup::BestEffort, async |id| {
        (client.orders().cancel(&id).await, id)
    })
    .await;
    let (ack, id) = cancelled;
    let ack = ack.expect("cancel the order");
    assert_eq!(ack.order_id, id);
    println!("cancel status: {:?}", ack.order_status);
}

#[tokio::test]
#[ignore = "needs sandbox credentials"]
async fn sandbox_o5_order_list() {
    let Some((client, _serial)) = sandbox().await else {
        return;
    };
    let orders = client.orders().list().await.expect("list the orders");
    println!("{} orders", orders.len());
}

#[tokio::test]
#[ignore = "needs sandbox credentials"]
async fn sandbox_o6_order_by_id() {
    let Some((client, _serial)) = sandbox().await else {
        return;
    };
    let order = limit_buy(SANDBOX_LIMIT_PRICE, "dhani-o6");
    let (got, id) = with_placed_order(&client, order, Cleanup::BestEffort, async |id| {
        (client.orders().get(&id).await, id)
    })
    .await;
    assert_eq!(got.expect("get the order").order_id, id);
}

#[tokio::test]
#[ignore = "needs sandbox credentials"]
async fn sandbox_o7_order_by_correlation_id() {
    let Some((client, _serial)) = sandbox().await else {
        return;
    };
    let order = limit_buy(SANDBOX_LIMIT_PRICE, "dhani-o7");
    let tag = order.correlation_id.clone().expect("tagged");
    let (got, id) = with_placed_order(&client, order, Cleanup::BestEffort, async |id| {
        (client.orders().get_by_correlation_id(&tag).await, id)
    })
    .await;
    assert_eq!(got.expect("get the order").order_id, id);
}

#[tokio::test]
#[ignore = "needs sandbox credentials"]
async fn sandbox_o8_trade_book() {
    let Some((client, _serial)) = sandbox().await else {
        return;
    };
    let trades = client.orders().trades().await.expect("list the trades");
    println!("{} trades", trades.len());
}

#[tokio::test]
#[ignore = "needs sandbox credentials"]
async fn sandbox_o9_trades_for_order() {
    let Some((client, _serial)) = sandbox().await else {
        return;
    };
    let trades = client.orders().trades().await.expect("list the trades");
    let Some(order_id) = trades.into_iter().find_map(|t| t.order_id) else {
        println!("skipped: the sandbox account has no trades");
        return;
    };
    let trades = client
        .orders()
        .trades_for_order(&order_id)
        .await
        .expect("the order's trades");
    println!("{} trades for order {order_id}", trades.len());
}

#[tokio::test]
#[ignore = "needs sandbox credentials"]
async fn sandbox_p1_holdings() {
    let Some((client, _serial)) = sandbox().await else {
        return;
    };
    let holdings = client.portfolio().holdings().await.expect("the holdings");
    println!("{} holdings", holdings.len());
}

#[tokio::test]
#[ignore = "needs sandbox credentials"]
async fn sandbox_p2_positions() {
    let Some((client, _serial)) = sandbox().await else {
        return;
    };
    let positions = client.portfolio().positions().await.expect("the positions");
    println!("{} positions", positions.len());
}

/// Converts one unit of an open long intraday position to delivery if the sandbox shows one.
/// Otherwise converts a position that does not exist and expects a broker error. The
/// documentation names no error code for this, so the code is printed for the first run to pin
/// down; the test also fails if the sandbox accepts the conversion.
#[tokio::test]
#[ignore = "needs sandbox credentials"]
async fn sandbox_p3_convert_position() {
    let Some((client, _serial)) = sandbox().await else {
        return;
    };
    let positions = client.portfolio().positions().await.expect("the positions");
    let open = positions.iter().find_map(|p| {
        p.net_qty.filter(|q| *q > 0)?;
        let from = p.product_type.as_ref()?.known()?;
        (from == ProductType::Intraday).then_some(())?;
        Some(ConvertPositionRequest::new(
            ProductType::Intraday,
            ProductType::Cnc,
            p.exchange_segment.as_ref()?.known()?,
            PositionType::Long,
            p.security_id.clone()?,
            1,
        ))
    });
    match open {
        Some(req) => {
            client
                .portfolio()
                .convert_position(&req)
                .await
                .expect("convert one unit of the open position");
            println!("converted one unit");
        }
        None => {
            let req = ConvertPositionRequest::new(
                ProductType::Intraday,
                ProductType::Cnc,
                ExchangeSegment::NseEq,
                PositionType::Long,
                hdfc_bank(),
                1,
            );
            let err = client
                .portfolio()
                .convert_position(&req)
                .await
                .expect_err("no position to convert");
            let code = err.api().and_then(|a| a.error_code.clone());
            println!("refused with {code:?}: {err}");
            assert_eq!(err.kind(), ErrorKind::Api, "a broker error, not {err}");
        }
    }
}

#[tokio::test]
#[ignore = "needs sandbox credentials"]
async fn sandbox_m1_fund_limit() {
    let Some((client, _serial)) = sandbox().await else {
        return;
    };
    let limits = client.funds().limits().await.expect("the fund limit");
    println!("available balance: {:?}", limits.available_balance);
}

#[tokio::test]
#[ignore = "needs sandbox credentials"]
async fn sandbox_m2_margin() {
    let Some((client, _serial)) = sandbox().await else {
        return;
    };
    let req = MarginRequest::new(
        ExchangeSegment::NseEq,
        hdfc_bank(),
        TransactionType::Buy,
        1,
        ProductType::Cnc,
        SANDBOX_LIMIT_PRICE,
    );
    let margin = client.funds().margin(&req).await.expect("the margin");
    println!("total margin: {:?}", margin.total_margin);
}

#[tokio::test]
#[ignore = "needs sandbox credentials"]
async fn sandbox_t1_ledger() {
    let Some((client, _serial)) = sandbox().await else {
        return;
    };
    let to = today();
    let entries = client
        .statements()
        .ledger(to - Days::new(30), to)
        .await
        .expect("the ledger");
    println!("{} ledger entries", entries.len());
}

#[tokio::test]
#[ignore = "needs sandbox credentials"]
async fn sandbox_t2_trade_history() {
    let Some((client, _serial)) = sandbox().await else {
        return;
    };
    let to = today();
    let trades = client
        .statements()
        .trade_history(to - Days::new(30), to, 0)
        .await
        .expect("the first trade-history page");
    println!("{} trades on page 0", trades.len());
}

/// The daily candles arrive as parallel arrays of one length.
#[tokio::test]
#[ignore = "needs sandbox credentials"]
async fn sandbox_h1_daily_candles() {
    let Some((client, _serial)) = sandbox().await else {
        return;
    };
    let to = today();
    let req = DailyRequest::new(
        ExchangeSegment::NseEq,
        hdfc_bank(),
        InstrumentKind::Equity,
        to - Days::new(30),
        to,
    );
    let candles = client
        .historical()
        .daily(&req)
        .await
        .expect("daily candles");
    assert_columnar(&candles);
}

/// 25-minute candles: the API spec, the guide and the Python SDK give 25, while some endpoint
/// pages say 30. The endpoint accepts 25, and the candles are 1500 s apart.
#[tokio::test]
#[ignore = "needs sandbox credentials"]
async fn sandbox_h2_intraday_candles() {
    let Some((client, _serial)) = sandbox().await else {
        return;
    };
    let to = today();
    let req = IntradayRequest::new(
        ExchangeSegment::NseEq,
        hdfc_bank(),
        InstrumentKind::Equity,
        IntradayInterval::Min25,
        to - Days::new(5),
        to,
    );
    let candles = client
        .historical()
        .intraday(&req)
        .await
        .expect("25-minute candles");
    assert_columnar(&candles);
    // The smallest gap between candles is one interval; larger gaps span nights and holidays.
    let step = candles
        .timestamp
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|d| *d > 0)
        .min();
    match step {
        Some(step) => assert_eq!(step, 25 * 60, "candles {step} s apart"),
        None => println!("fewer than two candles: the interval is not confirmed"),
    }
}

fn assert_columnar(candles: &Candles) {
    let n = candles.timestamp.len();
    println!("{n} candles");
    for (name, len) in [
        ("open", candles.open.len()),
        ("high", candles.high.len()),
        ("low", candles.low.len()),
        ("close", candles.close.len()),
        ("volume", candles.volume.len()),
    ] {
        assert_eq!(len, n, "{name} has {len} values for {n} timestamps");
    }
    // Open interest is present only when asked for.
    let oi = candles.open_interest.len();
    assert!(oi == 0 || oi == n, "open_interest has {oi} values for {n}");
}

// ---- Live, read-only. ----

#[tokio::test]
#[ignore = "needs live credentials"]
async fn live_ro_profile() {
    let Some((client, _serial)) = live().await else {
        return;
    };
    // Debug lists the key names only.
    let profile = client.account().profile().await.expect("the profile");
    println!("profile: {profile:?}");
}

#[tokio::test]
#[ignore = "needs live credentials"]
async fn live_ro_fund_limit() {
    let Some((client, _serial)) = live().await else {
        return;
    };
    let limits = client.funds().limits().await.expect("the fund limit");
    println!(
        "available balance present: {}",
        limits.available_balance.is_some()
    );
}

#[tokio::test]
#[ignore = "needs live credentials"]
async fn live_ro_holdings() {
    let Some((client, _serial)) = live().await else {
        return;
    };
    let holdings = client.portfolio().holdings().await.expect("the holdings");
    println!("{} holdings", holdings.len());
}

#[tokio::test]
#[ignore = "needs live credentials"]
async fn live_ro_positions() {
    let Some((client, _serial)) = live().await else {
        return;
    };
    let positions = client.portfolio().positions().await.expect("the positions");
    println!("{} positions", positions.len());
}

#[tokio::test]
#[ignore = "needs live credentials"]
async fn live_ro_orders() {
    let Some((client, _serial)) = live().await else {
        return;
    };
    let orders = client.orders().list().await.expect("the order book");
    println!("{} orders", orders.len());
}

/// LTP, OHLC and full quote for `NSE_EQ:1333`, typed and raw; the raw bodies are captured. One
/// client, so its rate limiter spaces the six Quote-class calls.
#[tokio::test]
#[ignore = "needs live credentials and a Data API subscription"]
async fn live_ro_quotes_nse_eq_1333() {
    let Some((client, _serial)) = live().await else {
        return;
    };
    let mut req = QuoteRequest::new();
    req.add(ExchangeSegment::NseEq, hdfc_bank());
    let quotes = client.market_quote();

    let ltp = quotes.ltp(&req).await.expect("LTP");
    println!("ltp: {:?}", ltp.data);
    dump(
        "ltp-nse_eq-1333",
        &quotes.ltp_raw(&req).await.expect("raw LTP"),
    );

    let ohlc = quotes.ohlc(&req).await.expect("OHLC");
    println!("ohlc: {:?}", ohlc.data);
    dump(
        "ohlc-nse_eq-1333",
        &quotes.ohlc_raw(&req).await.expect("raw OHLC"),
    );

    let quote = quotes.quote(&req).await.expect("quote");
    let n: usize = quote.data.values().map(|m| m.len()).sum();
    println!("quote instruments: {n}");
    dump(
        "quote-nse_eq-1333",
        &quotes.quote_raw(&req).await.expect("raw quote"),
    );
}

/// The expiry list for NIFTY (`IDX_I:13`), then the chain for its nearest expiry.
#[tokio::test]
#[ignore = "needs live credentials and a Data API subscription"]
async fn live_ro_option_chain_idx_i_13() {
    let Some((client, _serial)) = live().await else {
        return;
    };
    let nifty = UnderlyingRef::new(13, ExchangeSegment::IdxI);
    let expiries = client
        .option_chain()
        .expiries(&nifty)
        .await
        .expect("the expiry list");
    println!("{} expiries", expiries.len());
    let nearest = *expiries.iter().min().expect("at least one expiry");
    // Dhan allows one option-chain call per three seconds, and may count the expiry list.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let req = OptionChainRequest::new(nifty, nearest);
    let chain = client.option_chain().chain(&req).await.expect("the chain");
    println!("{} strikes for {nearest}", chain.strikes().len());
}

// ---- Live mutation: gated separately, never run in CI. ----

/// Places an after-market LIMIT buy of one HDFC Bank share at 90% of the last price, then
/// cancels it at once and checks that it is closed.
#[tokio::test]
#[ignore = "places a real order; needs live credentials and DHANI_LIVE_ALLOW_ORDERS=1"]
async fn live_mut_amo_limit_place_then_cancel() {
    match env("DHANI_LIVE_ALLOW_ORDERS").as_deref() {
        Some("1") => {}
        Some(_) => {
            println!("skipped: DHANI_LIVE_ALLOW_ORDERS is not 1");
            return;
        }
        None => return,
    }
    let Some((client, _serial)) = live().await else {
        return;
    };
    let mut req = QuoteRequest::new();
    req.add(ExchangeSegment::NseEq, hdfc_bank());
    let ltp = client.market_quote().ltp(&req).await.expect("LTP");
    let last = ltp
        .data
        .values()
        .flat_map(|m| m.values())
        .find_map(|q| q.last_price)
        .expect("a last price");
    let order = limit_buy((last * 0.9).floor(), "dhani-amo").with_amo(AmoTime::Open);
    let status = with_placed_order(&client, order, Cleanup::Strict, async |id| {
        client.orders().get(&id).await.map(|o| o.order_status)
    })
    .await;
    println!(
        "order status before cancel: {:?}",
        status.expect("read back")
    );
}
