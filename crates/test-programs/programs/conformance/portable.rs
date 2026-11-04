//! Sandboxed conformance guest: runs every portable scenario in
//! `omnia_test::conformance` through `omnia_sdk::WasiProvider`.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::{Config, WasiProvider};
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
}
