//! End-to-end tests for `omnia:config`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime. The guest
//! asserts what it observes across the boundary (and traps on failure). The
//! seeded scenario runs over the map-backed default seeded by
//! `Backends::config`, and the error row over a faulting backend that records
//! each key it is asked for.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::{Arc, Mutex};

use futures::FutureExt as _;
use omnia::{ExitStatus, FutureResult, Provides};
use omnia_test::host::Deployment;
use omnia_wasi_config::{ConfigClient, Error, WasiConfig};
use omnia_wasi_otel::{OtelClient, OtelDefault, WasiOtel};

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_config!();

async fn run_guest<B: Provides<WasiConfig> + Provides<WasiOtel>>(wasm: &str, backends: B) {
    let status = Deployment::new()
        .guest("guest", wasm)
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
    let backends = omnia_test::host::Backends::defaults()
        .await
        .config([("ORDERS_REGION", "eu"), ("RETRIES", "3")]);
    run_guest(test_programs::CONFIG_SETTINGS, backends).await;
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
