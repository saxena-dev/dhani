//! Observability overhead benchmark and allocation-budget smoke check.
//!
//! "Off means off": with no `tracing` subscriber and no metrics recorder, one mocked REST call
//! must allocate no more than the committed baseline in `benches/budgets.toml`, and exactly as
//! much as the same call with `tracing::subscriber::NoSubscriber` set as the default.
//! `tracing` cannot be compiled out, so this baseline is the guard against instrumentation that
//! allocates while nothing is listening.
//!
//! Runs under `cargo test --benches` (and plain `cargo test`) as well as `cargo bench`. There are
//! no timed benchmarks.
//!
//! Re-baselining: run `cargo test --no-default-features --features rest --bench overhead --
//! --nocapture` and the same with `--all-features`, read the printed `allocations per call`, and
//! set `rest_call_allocs_max` to the larger count plus 10 % (rounded up). Only do so for a change
//! that is expected to allocate more.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use dhani::config::{Environment, Urls};
use dhani::labels::EndpointId;
use dhani::rest::RateLimiter;
use dhani::{AccessToken, ClientId, Credentials, DhanClient};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Counts allocations made on the current thread while counting is switched on. The mock server
/// runs on its own threads and is not counted.
struct CountingAllocator;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static COUNT: Cell<u64> = const { Cell::new(0) };
}

fn note_allocation() {
    // `try_with`: the thread-locals may already be gone during thread teardown.
    let _ = COUNTING.try_with(|counting| {
        if counting.get() {
            let _ = COUNT.try_with(|n| n.set(n.get() + 1));
        }
    });
}

// SAFETY: every method forwards to the system allocator unchanged; the bookkeeping touches only
// const-initialised thread-locals without destructors, so it never allocates or recurses.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note_allocation();
        // SAFETY: forwarded with the caller's layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note_allocation();
        // SAFETY: forwarded with the caller's layout.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note_allocation();
        // SAFETY: forwarded with the caller's pointer, layout and size.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded with the caller's pointer and layout.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// The committed allocation budget.
fn budget() -> u64 {
    let text = include_str!("budgets.toml");
    let table: toml::Table = text.parse().expect("budgets.toml parses");
    let value = table
        .get("rest_call_allocs_max")
        .and_then(toml::Value::as_integer)
        .expect("budgets.toml has an integer rest_call_allocs_max");
    u64::try_from(value).expect("a non-negative budget")
}

fn client(server: &MockServer) -> DhanClient {
    let mut urls = Urls::for_env(Environment::Live);
    urls.rest = url::Url::parse(&format!("{}/v2", server.uri())).unwrap();
    DhanClient::builder()
        .urls(urls)
        .credentials(Credentials::new(
            ClientId::new("9999888877").unwrap(),
            AccessToken::new("ZZSENTINELZZ").unwrap(),
        ))
        .rate_limiter(RateLimiter::disabled())
        .build()
        .unwrap()
}

async fn one_call(client: &DhanClient) {
    let body = client
        .__execute_for_tests(EndpointId::OrdersList, &[], &[], None, None)
        .await
        .expect("the mocked call succeeds");
    assert!(body.is_some());
}

/// Allocations made on this thread by one call.
async fn allocations_of_one_call(client: &DhanClient) -> u64 {
    COUNT.with(|n| n.set(0));
    COUNTING.with(|c| c.set(true));
    one_call(client).await;
    COUNTING.with(|c| c.set(false));
    COUNT.with(Cell::get)
}

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/orders"))
            .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
            .mount(&server)
            .await;
        let client = client(&server);
        // Warm up: connection pool, callsite registration and lazily initialised state.
        for _ in 0..3 {
            one_call(&client).await;
        }
        let bare = allocations_of_one_call(&client).await;
        let with_no_subscriber = {
            let _guard = tracing::subscriber::set_default(tracing::subscriber::NoSubscriber::default());
            one_call(&client).await;
            allocations_of_one_call(&client).await
        };
        let max = budget();
        println!("overhead: allocations per call {bare} (budget {max})");
        assert_eq!(
            bare, with_no_subscriber,
            "a call allocates differently with NoSubscriber set as the default"
        );
        assert!(
            bare <= max,
            "a mocked REST call allocated {bare} times, over the budget of {max} in benches/budgets.toml"
        );
    });
}
