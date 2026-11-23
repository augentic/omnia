//! End-to-end tests for `omnia:config`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime. The guest
//! asserts what it observes across the boundary (and traps on failure). The
//! seeded scenarios run over the map-backed default seeded by
//! `Backends::config`, the served row over the default connected to the test
//! process's environment, and the error row over a faulting backend that
//! records each key it is asked for. A malformed `CONFIG_KEYS` is refused
//! before any guest runs.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::{Arc, Mutex};

use futures::FutureExt as _;
use omnia::{Connect as _, ExitStatus, FutureResult, Provides};
use omnia_test::host::Deployment;
use omnia_wasi_config::{ConfigClient, ConfigDefault, ConnectOptions, Error, WasiConfig};
use omnia_wasi_otel::{OtelClient, OtelDefault, WasiOtel};
use tracing::field::{Field, Visit};
use tracing::subscriber::DefaultGuard;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Registry;
use tracing_subscriber::layer::{Context as LayerContext, Layer, SubscriberExt as _};

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

// records the `key` of each warning `omnia_wasi_config` logs on the test's
// thread, for as long as the guard lives
#[derive(Clone, Default)]
struct Warnings(Arc<Mutex<Vec<String>>>);

impl Warnings {
    fn install() -> (Self, DefaultGuard) {
        let warnings = Self::default();
        let guard = tracing::subscriber::set_default(Registry::default().with(warnings.clone()));
        (warnings, guard)
    }

    fn keys(&self) -> Vec<String> {
        self.0.lock().expect("warnings recorded").clone()
    }
}

impl<S: Subscriber> Layer<S> for Warnings {
    fn on_event(&self, event: &Event<'_>, _: LayerContext<'_, S>) {
        struct Key(Option<String>);

        impl Visit for Key {
            fn record_str(&mut self, field: &Field, value: &str) {
                if field.name() == "key" {
                    self.0 = Some(value.to_owned());
                }
            }

            fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
        }

        let metadata = event.metadata();
        if *metadata.level() != Level::WARN || !metadata.target().starts_with("omnia_wasi_config") {
            return;
        }
        let mut key = Key(None);
        event.record(&mut key);
        self.0.lock().expect("warnings recorded").extend(key.0);
    }
}

// Cargo sets `CARGO_PKG_*` and `CARGO_MANIFEST_*` on every test process. The
// keys name a whole variable, a prefix, and a variable the process leaves
// unset, and `CARGO_PKG_VERSION` is withheld. The command guest runs on the
// test's own task, so the recorder sees the host's warnings.
#[tokio::test]
async fn config_served() {
    const KEYS: &str = "CARGO_PKG_NAME, CARGO_MANIFEST_*, OMNIA_CONFIG_SERVED_UNSET";
    let unset = "OMNIA_CONFIG_SERVED_UNSET";
    assert!(std::env::var_os(unset).is_none(), "`{unset}` is unset");
    for name in ["CARGO_PKG_NAME", "CARGO_MANIFEST_DIR", "CARGO_PKG_VERSION"] {
        assert!(std::env::var_os(name).is_some(), "cargo sets `{name}`");
    }

    let options =
        ConnectOptions::load_from(|name| (name == "CONFIG_KEYS").then(|| KEYS.to_owned()))
            .expect("every entry is a pattern");
    let mut backends = omnia_test::host::Backends::defaults().await;
    backends.config = ConfigDefault::connect_with(options).await.expect("default connects");

    let served = std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .filter(|(name, _)| name == "CARGO_PKG_NAME" || name.starts_with("CARGO_MANIFEST_"))
        .map(|(name, value)| format!("{name}={value}"));
    let args = served.chain(["CARGO_PKG_VERSION".to_owned(), unset.to_owned()]).collect();

    let (warnings, _guard) = Warnings::install();
    run_with_args(test_programs::CONFIG_SERVED, backends, args).await;
    assert_eq!(warnings.keys(), ["CARGO_PKG_VERSION"]);
}

#[test]
fn malformed_keys() {
    let lookup = |name: &str| (name == "CONFIG_KEYS").then(|| "ORDERS_URL, ORDERS-URL".to_owned());
    let err = ConnectOptions::load_from(lookup).expect_err("`ORDERS-URL` is not a pattern");
    assert_eq!(
        err.to_string(),
        "`CONFIG_KEYS` is invalid: `ORDERS-URL` is not a variable name, or a prefix of one that \
         ends in `*`"
    );
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
