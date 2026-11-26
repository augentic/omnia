//! A clone, a fetch, and a push from a guest under a grants table. Each
//! succeeds, since the suite's backend refuses no host. A fetch from a
//! remote the backend locates beneath a mount the grants leave out is
//! `denied`.

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

    let hidden = clone.fetch("hidden").await.expect_err("a remote beneath a mount left out");
    assert!(
        hidden.code() == "denied" && hidden.description().contains("outside the guest's mounts"),
        "expected `denied`, got {hidden:?}"
    );
}
