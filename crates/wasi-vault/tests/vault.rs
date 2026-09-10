//! End-to-end tests for `omnia:vault`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime against
//! the in-memory default. The guest asserts what it observes across the
//! boundary (and traps on failure); the host side seeds the locker and
//! asserts what persisted after the run.

#![cfg(not(target_arch = "wasm32"))]

use omnia::ExitStatus;
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_vault::{WasiVault, WasiVaultCtx as _};

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_vault!();

/// Run one guest program against `backends`, requiring a clean exit.
async fn run_guest(wasm: &str, backends: Backends) {
    let status = Deployment::new()
        .guest("guest", wasm)
        .run_host::<WasiVault, _>(backends)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
}

#[tokio::test]
async fn vault_locker() {
    let backends = Backends::defaults().await;
    let locker = backends.vault.open_locker("locker".to_owned()).await.expect("locker");
    locker.set("seeded".to_owned(), b"from-host".to_vec()).await.expect("seed");

    run_guest(test_programs::VAULT_LOCKER, backends.clone()).await;

    // Persisted state: the overwrite won, the delete took, the seed survived.
    let locker = backends.vault.open_locker("locker".to_owned()).await.expect("locker");
    let mut ids = locker.list_ids().await.expect("list-ids");
    ids.sort();
    assert_eq!(ids, ["seeded", "token"]);
    assert_eq!(locker.get("token".to_owned()).await.expect("get"), Some(b"v2".to_vec()));
    assert_eq!(locker.get("seeded".to_owned()).await.expect("get"), Some(b"from-host".to_vec()));
    assert!(!locker.exists("doomed".to_owned()).await.expect("exists"));
}
