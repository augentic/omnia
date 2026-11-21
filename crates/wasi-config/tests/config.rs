//! End-to-end tests for `omnia:config`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime. The guest
//! asserts what it observes across the boundary (and traps on failure). The
//! seeded scenarios run over the map-backed default seeded by
//! `Backends::config`, the served row over the default connected to the test
//! process's environment, and the error row over a faulting backend that
//! records each key it is asked for.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::{Arc, Mutex};

use futures::FutureExt as _;
use omnia::{Connect as _, ExitStatus, FutureResult, Provides};
use omnia_test::host::Deployment;
use omnia_wasi_config::{ConfigClient, ConfigDefault, ConnectOptions, Error, WasiConfig};
use omnia_wasi_otel::{OtelClient, OtelDefault, WasiOtel};

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_config!();

async fn run_guest<B: Provides<WasiConfig> + Provides<WasiOtel>>(wasm: &str, backends: B) {
    run_with_args(wasm, backends, Vec::new()).await;
}

async fn run_with_args<B>(wasm: &str, backends: B, args: Vec<String>)
where
    B: Provides<WasiConfig> + Provides<WasiOtel>,
{
    let status = Deployment::new()
        .guest("guest", wasm)
        .args(args)
        .run_host::<WasiConfig, _>(backends)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
}

#[tokio::test]
async fn config_seeded() {
    let backends = omnia_test::host::Backends::defaults().await.config([
        ("GREETING", "hello"),
        ("EMPTY", ""),
        ("ORDERS_CURRENCY", "nzd"),
        ("ORDERS_REGION", "eu"),
    ]);
    run_guest(test_programs::CONFIG_SEEDED, backends).await;
}

#[tokio::test]
async fn config_settings() {
    let backends = omnia_test::host::Backends::defaults().await.config([
        ("ORDERS_REGION", "eu"),
        ("RETRIES", "3"),
        ("ORDERS_CURRENCY", "nzd"),
    ]);
    run_guest(test_programs::CONFIG_SETTINGS, backends).await;
}

// Cargo sets `CARGO_PKG_NAME` and `CARGO_MANIFEST_DIR` on every test process,
// and a key named but unset is served as nothing.
#[tokio::test]
async fn config_served() {
    let keys = ["CARGO_PKG_NAME", "OMNIA_CONFIG_SERVED_UNSET"];
    let options = ConnectOptions {
        keys: keys.map(str::to_owned).to_vec(),
    };
    let mut backends = omnia_test::host::Backends::defaults().await;
    backends.config = ConfigDefault::connect_with(options).await.expect("default connects");

    let name = std::env::var("CARGO_PKG_NAME").expect("cargo sets `CARGO_PKG_NAME`");
    assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some(), "cargo sets `CARGO_MANIFEST_DIR`");
    let args = vec![format!("CARGO_PKG_NAME={name}"), "CARGO_MANIFEST_DIR".to_owned()];
    run_with_args(test_programs::CONFIG_SERVED, backends, args).await;
}

// `omnia_test`'s bundle holds the default backend alone, and the error row
// runs over a faulting one
#[derive(Clone, Debug)]
struct Backends {
    config: Faulting,
    otel: OtelDefault,
}

impl Provides<WasiConfig> for Backends {
    fn borrow(&mut self) -> &dyn ConfigClient {
        &self.config
    }
}

impl Provides<WasiOtel> for Backends {
    fn borrow(&mut self) -> &dyn OtelClient {
        &self.otel
    }
}

// fails `get` with the kind a key is named for, records each key so the run's
// exchanges are pinned, and lists nothing
#[derive(Clone, Debug, Default)]
struct Faulting {
    keys: Arc<Mutex<Vec<String>>>,
}

impl ConfigClient for Faulting {
    fn get(&self, key: String) -> FutureResult<Option<String>> {
        let fault: anyhow::Error = match key.as_str() {
            "NOT_FOUND" => Error::NotFound("not found by the suite".to_owned()).into(),
            "DENIED" => Error::Denied("denied by the suite".to_owned()).into(),
            "INVALID" => Error::Invalid("invalid in the suite".to_owned()).into(),
            "UNSUPPORTED" => Error::Unsupported("unsupported by the suite".to_owned()).into(),
            "UNAVAILABLE" => Error::Unavailable("unavailable in the suite".to_owned()).into(),
            _ => anyhow::anyhow!("an untyped backend failure"),
        };
        self.keys.lock().expect("keys asked for").push(key);
        async move { Err(fault) }.boxed()
    }
}

#[tokio::test]
async fn config_error_codes() {
    let backends = Backends {
        config: Faulting::default(),
        otel: OtelDefault,
    };

    run_guest(test_programs::CONFIG_ERROR_CODES, backends.clone()).await;

    let keys = backends.config.keys.lock().expect("keys asked for").clone();
    assert_eq!(keys, ["NOT_FOUND", "DENIED", "INVALID", "UNSUPPORTED", "UNAVAILABLE", "OTHER"]);
}
