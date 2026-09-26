//! Public-surface assertions: every public item is reachable at its documented path, and the
//! client types have the documented auto traits.

mod support;

#[cfg(feature = "rest")]
const _: () = {
    fn assert<T: Send + Sync + Clone + 'static>() {}
    let _ = assert::<dhani::DhanClient>;
};

#[cfg(feature = "rest")]
#[test]
fn the_client_is_send_sync_and_clone() {
    fn assert<T: Send + Sync + Clone + 'static>() {}
    assert::<dhani::DhanClient>();
    assert::<dhani::rest::DhanClient>();
}
