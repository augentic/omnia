//! Named identities through the `omnia_sdk` identity helpers: two declared
//! names issue distinct tokens, a token's lifetime arrives as the `Duration`
//! it has left, and an undeclared name fails with the code `not-found`.

#![cfg(target_arch = "wasm32")]

use std::time::Duration;

use omnia_sdk::identity::{AccessToken, DEFAULT, credentials};
use omnia_sdk::{Error, Result};

omnia_sdk::command::export!(scenario);

async fn scenario() {
    let default = token(DEFAULT, &["read"]).await.expect("default token");
    let reports = token("reports", &["read"]).await.expect("reports token");

    assert_ne!(default.token, reports.token, "each identity has its own token");
    // the stub's hour, less what passed before the guest read the clock
    let lifetime = Duration::from_secs(59 * 60)..=Duration::from_secs(3600);
    assert!(lifetime.contains(&reports.expires_in), "lifetime {:?}", reports.expires_in);

    let error = token("undeclared", &[]).await.expect_err("undeclared");
    assert!(
        matches!(&error, Error::ServerError { code, .. } if code == "not-found"),
        "unexpected error: {error}"
    );
}

async fn token(name: &str, scopes: &[&str]) -> Result<AccessToken> {
    credentials::open(name.to_owned()).await?.token(scopes).await
}
