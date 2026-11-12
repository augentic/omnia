//! Every `omnia:sql` error kind reaches the guest as its own `ErrorKind`.
//! The default backend fails an undeclared database and a malformed
//! statement itself. The suite's faulting backend fails the rest by
//! database name. The `wasi:sql` draft's error carries only the message.

#![cfg(target_arch = "wasm32")]

use std::fmt::Debug;

use omnia_sdk::capability::{ErrorKind, Result};
use omnia_sdk::{SqlDb as _, WasiProvider};
use omnia_wasi_sql::types::Connection;

omnia_sdk::api::command::export!(scenario);

async fn scenario() {
    let provider = WasiProvider;

    // the default backend's own kinds
    expect_kind(provider.named("undeclared").query("SELECT 1", &[]).await, ErrorKind::NotFound);
    expect_kind(provider.execute("SELEKT nonsense", &[]).await, ErrorKind::Invalid);

    // the backend's chosen kinds
    for (database, kind) in [
        ("denied", ErrorKind::Denied),
        ("unsupported", ErrorKind::Unsupported),
        ("unavailable", ErrorKind::Unavailable),
        ("other", ErrorKind::Other),
    ] {
        expect_kind(provider.named(database).query("SELECT 1", &[]).await, kind);
    }

    // the draft's error has no kind
    let Err(error) = Connection::open("denied".to_owned()).await else {
        panic!("the draft opened a denied database");
    };
    assert_eq!(error.trace(), "denied by the suite");
}

fn expect_kind<T: Debug>(result: Result<T>, kind: ErrorKind) {
    match result {
        Ok(value) => panic!("returned {value:?}, not a `{kind}` failure"),
        Err(err) => assert_eq!(err.kind(), kind, "{err}"),
    }
}
