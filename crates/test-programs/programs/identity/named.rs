//! Named identities through the `omnia_sdk` `Identity`: the unnamed handle is
//! `default`, two declared names issue distinct tokens, a token's lifetime
//! arrives as the `Duration` it has left, and an undeclared name fails with
//! the code `not-found`.

#![cfg(target_arch = "wasm32")]

use std::time::Duration;

use omnia_sdk::{Error, Identity, WasiProvider};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let unnamed = WasiProvider.token(&["read"]).await.expect("unnamed token");
    let default = WasiProvider.named("default").token(&["read"]).await.expect("default token");
    let reports = WasiProvider.named("reports").token(&["read"]).await.expect("reports token");

    assert_eq!(unnamed.token, default.token, "the unnamed handle is `default`");
    assert_ne!(default.token, reports.token, "each identity has its own token");
    // the stub's hour, less what passed before the guest read the clock
    let lifetime = Duration::from_secs(59 * 60)..=Duration::from_secs(3600);
    assert!(lifetime.contains(&reports.expires_in), "lifetime {:?}", reports.expires_in);

    let error = WasiProvider.named("undeclared").token(&[]).await.expect_err("undeclared");
    assert!(
        matches!(&error, Error::ServerError { code, .. } if code == "not-found"),
        "unexpected error: {error}"
    );
}
