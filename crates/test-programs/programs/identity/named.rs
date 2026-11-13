//! Named identities through the `omnia_sdk` `Identity`: the unnamed handle is
//! `default`, two declared names issue distinct tokens, a token's lifetime
//! arrives as a `Duration`, and an undeclared name fails with `NotFound`.

#![cfg(target_arch = "wasm32")]

use std::time::Duration;

use omnia_sdk::capability::ErrorKind;
use omnia_sdk::{Identity, WasiProvider};

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let unnamed = WasiProvider.token(&["read"]).await.expect("unnamed token");
    let default = WasiProvider.named("default").token(&["read"]).await.expect("default token");
    let reports = WasiProvider.named("reports").token(&["read"]).await.expect("reports token");

    assert_eq!(unnamed, default, "the unnamed handle is `default`");
    assert_ne!(default.token, reports.token, "each identity has its own token");
    assert_eq!(reports.expires_in, Duration::from_secs(3600));

    let error = WasiProvider.named("undeclared").token(&[]).await.expect_err("undeclared");
    assert_eq!(error.kind(), ErrorKind::NotFound, "unexpected error: {error}");
}
