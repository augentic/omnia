//! End-to-end tests for `omnia:identity`: every scenario runs a real guest
//! component from `crates/test-programs` through the omnia runtime against
//! an inline recording backend. The guest asserts what it observes across
//! the boundary (and traps on failure); the host side asserts what the
//! backend was asked for. `IdentityStub` records nothing, so it is not the
//! scenario backend here.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::{Arc, Mutex};

use futures::FutureExt as _;
use omnia::{ExitStatus, FutureResult, Provides};
use omnia_test::host::Deployment;
use omnia_wasi_identity::{AccessToken, Identity, WasiIdentity, WasiIdentityCtx};
use omnia_wasi_otel::{OtelDefault, WasiOtel, WasiOtelCtx};

// Every guest program in `crates/test-programs` must have a matching test
// here; a new program without one fails to compile.
test_programs::foreach_identity!();

/// The store's backend bundle: the recording identity backend under test
/// plus the no-op otel every `command!` guest imports.
#[derive(Clone, Debug)]
struct Backends {
    identity: RecordingIdentity,
    otel: OtelDefault,
}

impl Backends {
    fn new() -> Self {
        Self {
            identity: RecordingIdentity::default(),
            otel: OtelDefault,
        }
    }
}

impl Provides<WasiIdentity> for Backends {
    fn borrow(&mut self) -> &mut dyn WasiIdentityCtx {
        &mut self.identity
    }
}

impl Provides<WasiOtel> for Backends {
    fn borrow(&mut self) -> &mut dyn WasiOtelCtx {
        &mut self.otel
    }
}

/// Every `get-token` call the backend saw, as `(identity name, scopes)`.
type Requests = Arc<Mutex<Vec<(String, Vec<String>)>>>;

/// Issues a fixed token, records every `(name, scopes)` request, and refuses
/// the name `missing`.
#[derive(Clone, Debug, Default)]
struct RecordingIdentity {
    requests: Requests,
}

impl RecordingIdentity {
    fn requests(&self) -> Vec<(String, Vec<String>)> {
        self.requests.lock().expect("requests lock").clone()
    }
}

impl WasiIdentityCtx for RecordingIdentity {
    fn get_identity(&self, name: String) -> FutureResult<Arc<dyn Identity>> {
        let requests = Arc::clone(&self.requests);
        async move {
            if name == "missing" {
                anyhow::bail!("no identity named `{name}`");
            }
            Ok(Arc::new(Recorded { name, requests }) as Arc<dyn Identity>)
        }
        .boxed()
    }
}

/// One resolved identity, sharing the backend's request log.
#[derive(Debug)]
struct Recorded {
    name: String,
    requests: Requests,
}

impl Identity for Recorded {
    fn get_token(&self, scopes: Vec<String>) -> FutureResult<AccessToken> {
        self.requests.lock().expect("requests lock").push((self.name.clone(), scopes));
        async {
            Ok(AccessToken {
                token: "recorded-token".to_owned(),
                expires_in: 900,
            })
        }
        .boxed()
    }
}

/// Run one guest program against `backends`, requiring a clean exit.
async fn run_guest(wasm: &str, backends: Backends) {
    let status = Deployment::new()
        .guest("guest", wasm)
        .run_host::<WasiIdentity, _>(backends)
        .await
        .expect("guest runs");
    assert_eq!(status, ExitStatus::SUCCESS, "guest `{wasm}` failed");
}

#[tokio::test]
async fn identity_stub_token() {
    let backends = Backends::new();

    run_guest(test_programs::IDENTITY_STUB_TOKEN, backends.clone()).await;

    // Wire fidelity: the scopes reached the backend in order, under the
    // resolved name, and the refused name never issued a token.
    assert_eq!(
        backends.identity.requests(),
        [("default".to_owned(), vec!["read".to_owned(), "write".to_owned()])]
    );
}
