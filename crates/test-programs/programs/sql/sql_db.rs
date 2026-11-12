//! The `omnia_sdk::contract::sql` rows over `WasiProvider`, all at once, so
//! the host's `omnia:sql` and its backend meet the `SqlDb` contract.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::WasiProvider;
use omnia_sdk::contract::sql;

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let db = WasiProvider;

    futures::join!(
        sql::batch(&db),
        sql::batch_rollback(&db),
        sql::placeholders(&db),
        sql::values(&db),
    );
}
