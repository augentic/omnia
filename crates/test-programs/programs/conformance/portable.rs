//! Sandboxed conformance guest: runs every portable scenario in
//! `omnia_test::conformance` through `omnia_sdk::WasiProvider`, then exchanges
//! keys and atomic counters with the host on a shared bucket.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::{Config, StateStore, WasiProvider};
use omnia_test::conformance;

omnia_sdk::command!(scenario);

async fn scenario() {
    conformance::state_get_set_delete(&WasiProvider).await;
    conformance::state_ttl_expires(&WasiProvider).await;
    conformance::state_cas(&WasiProvider).await;
    conformance::state_increment(&WasiProvider).await;
    conformance::table_query_exec(&WasiProvider).await;
    conformance::docstore_crud_query(&WasiProvider).await;
    conformance::blobstore_objects(&WasiProvider).await;
    conformance::publish_send(&WasiProvider).await;
    conformance::config_get(&WasiProvider).await;
    conformance::identity_token(&WasiProvider).await;
    conformance::vault_locker(&WasiProvider).await;

    let origin =
        Config::get(&WasiProvider, "CONFORMANCE_ORIGIN").await.expect("CONFORMANCE_ORIGIN");
    conformance::http_fetch(&WasiProvider, &origin).await;

    conformance::broadcast_send(&WasiProvider).await;
    conformance::model_complete(&WasiProvider).await;
    conformance::vcs_flow(&WasiProvider).await;

    // Byte identity with `Direct<KeyValueDefault>`: read what `Direct` wrote,
    // advance the shared counter, and write a key for `Direct` to read back.
    if Config::get(&WasiProvider, "CONFORMANCE_SHARED_BUCKET").await.is_ok() {
        assert_eq!(
            StateStore::get(&WasiProvider, "shared/from-direct").await.expect("read from direct"),
            Some(b"written-by-direct".to_vec())
        );
        assert_eq!(
            StateStore::increment(&WasiProvider, "shared/counter", 5)
                .await
                .expect("increment shared"),
            15
        );
        StateStore::set(&WasiProvider, "shared/from-wasi", b"written-by-wasi", None)
            .await
            .expect("write wasi");
    }
}
