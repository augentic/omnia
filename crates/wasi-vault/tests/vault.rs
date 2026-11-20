//! End-to-end tests for `omnia:vault`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime against
//! the in-memory default. The guest asserts what it observes across the
//! boundary (and traps on failure); the host side seeds the locker and
//! asserts what persisted after the run.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::{Arc, Mutex};

use omnia::{Connect as _, ExitStatus};
use omnia_test::host::{Backends, Deployment};
use omnia_wasi_vault::{
    ConnectOptions, Error, FutureResult, Locker, VaultClient, VaultDefault, WasiVault,
};

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_vault!();

async fn run_guest(wasm: &str, backends: Backends) {
    let status = Deployment::new()
        .guest("guest", wasm)
        .run_host::<WasiVault, _>(backends)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
}

async fn declaring(lockers: &[&str]) -> VaultDefault {
    let options = ConnectOptions {
        lockers: lockers.iter().map(|&locker| locker.to_owned()).collect(),
    };
    VaultDefault::connect_with(options).await.expect("in-memory vault")
}

#[tokio::test]
async fn vault_locker() {
    let backends = Backends::defaults().await.vault(declaring(&["locker"]).await);
    let locker = backends.vault.open_locker("locker".to_owned()).await.expect("locker");
    locker.set("seeded".to_owned(), b"from-host".to_vec()).await.expect("seed");

    run_guest(test_programs::VAULT_LOCKER, backends.clone()).await;

    // Persisted state: the overwrite won, the deleted name was set again,
    // and the seed survived.
    let locker = backends.vault.open_locker("locker".to_owned()).await.expect("locker");
    let mut names = locker.list_ids().await.expect("list-ids");
    names.sort();
    assert_eq!(names, ["doomed", "seeded", "tls-cert", "tls-key", "token"]);
    assert_eq!(locker.get("token".to_owned()).await.expect("get"), Some(b"v2".to_vec()));
    assert_eq!(locker.get("doomed".to_owned()).await.expect("get"), Some(b"back".to_vec()));
}

#[tokio::test]
async fn vault_lockers() {
    let backends = Backends::defaults().await.vault(declaring(&["default", "payments"]).await);

    run_guest(test_programs::VAULT_LOCKERS, backends.clone()).await;

    // Persisted state: the unnamed handle wrote to the locker named `default`.
    let locker = backends.vault.open_locker("default".to_owned()).await.expect("locker");
    assert_eq!(locker.list_ids().await.expect("list-ids"), ["api-key"]);
    assert_eq!(locker.get("api-key".to_owned()).await.expect("get"), Some(b"default-key".to_vec()));
}

#[tokio::test]
async fn vault_default_locker() {
    let backends = Backends::defaults().await.vault(declaring(&["default"]).await);
    let locker = backends.vault.open_locker("default".to_owned()).await.expect("locker");
    for n in 0..=1_000 {
        locker.set(format!("bulk-{n:04}"), b"x".to_vec()).await.expect("seed");
    }

    run_guest(test_programs::VAULT_DEFAULT_LOCKER, backends.clone()).await;

    // Persisted state: the secret the guest left.
    assert_eq!(locker.get("kept".to_owned()).await.expect("get"), Some(b"v".to_vec()));
}

// fails `get` with the kind a locker is named for, delegating any other
// name, and records each name so the run's exchanges are pinned
#[derive(Clone, Debug)]
struct Faulting {
    inner: VaultDefault,
    opened: Arc<Mutex<Vec<String>>>,
}

impl VaultClient for Faulting {
    fn open_locker(&self, identifier: String) -> FutureResult<Arc<dyn Locker>> {
        self.opened.lock().expect("opened lockers").push(identifier.clone());
        let fault: fn() -> anyhow::Error = match identifier.as_str() {
            "denied" => || Error::Denied("denied by the suite".to_owned()).into(),
            "invalid" => || Error::Invalid("invalid in the suite".to_owned()).into(),
            "unsupported" => || Error::Unsupported("unsupported by the suite".to_owned()).into(),
            "unavailable" => || Error::Unavailable("unavailable in the suite".to_owned()).into(),
            "other" => || anyhow::anyhow!("an untyped backend failure"),
            _ => return self.inner.open_locker(identifier),
        };
        Box::pin(async move { Ok(Arc::new(Faulty(fault)) as Arc<dyn Locker>) })
    }
}

#[derive(Debug)]
struct Faulty(fn() -> anyhow::Error);

impl Locker for Faulty {
    fn get(&self, _name: String) -> FutureResult<Option<Vec<u8>>> {
        let fault = (self.0)();
        Box::pin(async move { Err(fault) })
    }

    fn set(&self, _name: String, _value: Vec<u8>) -> FutureResult<()> {
        unreachable!("the scenario sets no secret")
    }

    fn delete(&self, _name: String) -> FutureResult<()> {
        unreachable!("the scenario deletes no secret")
    }

    fn exists(&self, _name: String) -> FutureResult<bool> {
        unreachable!("the scenario checks no secret")
    }

    fn list_ids(&self) -> FutureResult<Vec<String>> {
        unreachable!("the scenario lists no locker")
    }
}

#[tokio::test]
async fn vault_error_codes() {
    let faulting = Faulting {
        inner: declaring(&["default"]).await,
        opened: Arc::default(),
    };
    let backends = Backends::defaults().await.vault(faulting);

    let status = Deployment::new()
        .guest("guest", test_programs::VAULT_ERROR_CODES)
        .run_host::<WasiVault, _>(backends.clone())
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest failed");

    let opened = backends.vault.opened.lock().expect("opened lockers").clone();
    assert_eq!(opened, ["undeclared", "denied", "invalid", "unsupported", "unavailable", "other"]);
}
