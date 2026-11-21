//! `#[derive(Settings)]` and the typed reads beneath it, over the host's
//! seeded map. A struct loads every field from its key, an unset key fails
//! with the code `not-found`, and a value that does not parse with `invalid`.
//! An `Option` field loads as `None` when its key is unset.

#![cfg(target_arch = "wasm32")]

use std::fmt::Debug;

use omnia_sdk::{ErrorKind, Result, Settings, config};

omnia_sdk::command::export!(scenario);

#[derive(Debug, Settings)]
#[setting(prefix = "ORDERS_")]
struct Orders {
    region: String,
    #[setting(key = "RETRIES")]
    retries: u8,
    currency: Option<String>,
    discount: Option<u8>,
}

#[derive(Debug, Settings)]
#[expect(dead_code, reason = "it only ever fails to load")]
struct Unset {
    missing: String,
}

#[derive(Debug, Settings)]
#[expect(dead_code, reason = "it only ever fails to load")]
struct Mistyped {
    #[setting(key = "ORDERS_REGION")]
    region: u16,
}

async fn scenario() {
    // a struct loads each field from its key, and an unset optional one as `None`
    assert_eq!(Orders::KEYS, ["ORDERS_REGION", "RETRIES", "ORDERS_CURRENCY", "ORDERS_DISCOUNT"]);
    let orders = Orders::load().await.expect("every required key is seeded");
    assert_eq!((orders.region.as_str(), orders.retries), ("eu", 3));
    assert_eq!((orders.currency.as_deref(), orders.discount), (Some("nzd"), None));
    expect_code(Unset::load().await, "not-found");
    expect_code(Mistyped::load().await, "invalid");

    // the typed reads it loads through
    assert_eq!(config::parse::<u8>("RETRIES").await.expect("a seeded key"), Some(3));
    assert_eq!(config::parse::<u8>("MISSING").await.expect("an unset key"), None);
    expect_code(config::require::<u8>("MISSING").await, "not-found");
    expect_code(config::parse::<u8>("ORDERS_REGION").await, "invalid");
}

fn expect_code<T: Debug>(result: Result<T>, code: &str) {
    match result {
        Ok(value) => panic!("returned {value:?}, not a `{code}` failure"),
        Err(err) => assert!(
            err.kind() == ErrorKind::ServerError && err.code() == code,
            "failed with `{err}`, not `{code}`"
        ),
    }
}
