//! The `omnia_test::contract::sql` rows over the `default` database, all at
//! once, so the host's `omnia:sql` and its backend meet the contract.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::sql::{DEFAULT, store};
use omnia_test::contract::sql;

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let db = store::open(DEFAULT.to_owned()).await.expect("open the default database");

    futures::join!(
        sql::batch(&db),
        sql::batch_rollback(&db),
        sql::conversions(&db),
        sql::placeholders(&db),
        sql::transaction(&db),
        sql::values(&db),
    );
}
