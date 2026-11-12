//! The `omnia_sdk::contract::document` rows over `WasiProvider`, all at
//! once, so the host's `omnia:docstore` and its backend meet the
//! `DocumentDb` contract the native doubles do.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::WasiProvider;
use omnia_sdk::contract::document;

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let db = WasiProvider;

    futures::join!(
        document::insert_exists(&db),
        document::portable_query(&db),
        document::optional_query(&db),
    );
}
