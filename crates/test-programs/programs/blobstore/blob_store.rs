//! The `omnia_test::contract::blob` rows over `WasiProvider`, all at once,
//! so the host's `wasi:blobstore` meets the contract the native double does.
//! One object of the program's own is left for the host to read back.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::{BlobStore as _, WasiProvider};
use omnia_test::contract::blob;

omnia_sdk::api::command::export!(scenario);

const SCOPE: &str = "contract";
const HOST: &str = "contract-host";

async fn scenario() {
    let store = WasiProvider;

    store.create_container(HOST).await.expect("create the host's container");
    store.put(HOST, "raw", b"raw").await.expect("put the host's object");

    futures::join!(
        blob::objects(&store, SCOPE),
        blob::absent(&store, SCOPE),
        blob::delete_absent(&store, SCOPE),
        blob::uncreated(&store, SCOPE),
        blob::containers(&store, SCOPE),
        blob::range(&store, SCOPE),
        blob::listing(&store, SCOPE),
        blob::copy_rename(&store, SCOPE),
        blob::clear(&store, SCOPE),
    );
}
