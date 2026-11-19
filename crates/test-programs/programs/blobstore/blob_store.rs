//! The `omnia_test::contract::blob` rows, all at once, so the host's
//! `omnia:blobstore` and its backend meet the contract. The program reads an
//! object the host seeded before the run, and leaves one of its own for the
//! host to read back.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::blobstore::{admin, store};
use omnia_test::contract::blob;

omnia_sdk::api::command::export!(scenario);

const SCOPE: &str = "contract";
const HOST: &str = "contract-host";
const SEEDED: &str = "seeded";

async fn scenario() {
    let seeded = store::open(SEEDED.to_owned()).await.expect("open the host's container");
    let read = seeded.read("from-host").await.expect("read the host's object");
    assert_eq!(read.as_deref(), Some(&b"hello"[..]), "the host's object reads other bytes");
    let host = admin::create_container(HOST.to_owned()).await.expect("create the host's container");
    host.write("raw", b"raw").await.expect("write the host's object");

    futures::join!(
        blob::objects(SCOPE),
        blob::absent(SCOPE),
        blob::delete_absent(SCOPE),
        blob::uncreated(SCOPE),
        blob::containers(SCOPE),
        blob::range(SCOPE),
        blob::listing(SCOPE),
        blob::copy_rename(SCOPE),
        blob::clear(SCOPE),
    );
}
