//! A clone, a fetch, and a push from a guest under a grants table. Each
//! succeeds, since the suite's backend refuses no address.

#![cfg(target_arch = "wasm32")]

use omnia_sdk::vcs;

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let clone = vcs::clone("https://example.test/repo.git", "./clone", None)
        .await
        .expect("clone")
        .expect("a new workspace");
    clone.fetch("origin").await.expect("fetch");
    clone.push("origin", "main").await.expect("push").expect("a forward push");
}
