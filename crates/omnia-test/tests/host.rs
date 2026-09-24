//! The component rung: real guest components from `crates/test-programs`
//! driven through `Deployment` over `Backends`.

#![cfg(not(target_arch = "wasm32"))]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, bail};
use omnia::wasmtime::component::Val;
use omnia::{Digest, ExitStatus, GuestEntry, GuestId, Runtime, StoreCtx};
use omnia_test::host::{Backends, Deployment, ScriptedModel, scratch};
use omnia_test::{Exchange, SeenFormat};
use omnia_wasi_blobstore::WasiBlobstoreCtx as _;
use omnia_wasi_keyvalue::{Bucket, FutureResult, KeyValueDefault, WasiKeyValue, WasiKeyValueCtx};
use omnia_wasi_model::WasiModel;
use omnia_wasi_otel::WasiOtel;

// A production `runtime!` as an embedder would write it: the compiled-in
// hosts, connected from the environment under `main`. The macro embeds a
// `guests:` entry with `include_bytes!`, which takes a literal path, so the
// guest a suite names by its `test_programs` path constant joins through
// the overlay instead.
mod production {
    use omnia_wasi_keyvalue::{KeyValueDefault, WasiKeyValue};
    use omnia_wasi_model::{ModelDefault, WasiModel};
    use omnia_wasi_otel::{OtelDefault, WasiOtel};

    omnia::runtime!({
        mode: command,
        hosts: {
            WasiModel: ModelDefault,
            WasiKeyValue: KeyValueDefault,
            WasiOtel: OtelDefault,
        },
    });
}

// The same shape with a `.` mount the binary would preopen — a directory
// that does not exist under test.
mod production_plugins {
    use omnia_wasi_otel::{OtelDefault, WasiOtel};

    omnia::runtime!({
        mode: command,
        mounts: [{ name: ".", path: "/nonexistent/adapters" }],
        hosts: { WasiOtel: OtelDefault },
    });
}

#[tokio::test]
async fn runtime_overlay() {
    // The generated `main` and `run` stay untouched; the overlay reaches the
    // same `Hooks` through `run_with`.
    let _ = (production::main, production::run);
    let backends = Backends::defaults().await.model(ScriptedModel::answering(["second"]));
    let status = Deployment::from(production::manifest())
        .guest("echo", test_programs::MODEL_ECHO_TEXT)
        .run_with::<production::Hooks, _>(backends.clone())
        .await
        .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS);

    let seen = backends.model.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].messages, ["hi", "second"]);
    backends.model.assert_exhausted();
}

// The overlay's `.` mount stands in for the binary's: mounts dedup by name,
// last wins, before any directory is opened, so the nonexistent production
// root is never touched (opening it would fail the boot) and the overlaid
// deployment — an on-demand guest included — runs through the binary's hooks.
#[tokio::test]
async fn overlay_mount() {
    let _ = (production_plugins::main, production_plugins::run);
    let scratch = scratch();

    let deployment = Deployment::from(production_plugins::manifest())
        .guest("requester", test_programs::PLUGINS_LOAD)
        .on_demand("plugin", test_programs::LINK_ECHOER)
        .args(["plugin"])
        .mount(scratch.mount(false));
    let manifest = deployment.manifest().expect("inline base resolves");
    assert_eq!(
        manifest.mounts.iter().map(|mount| mount.path.as_path()).collect::<Vec<_>>(),
        [std::path::Path::new("/nonexistent/adapters"), scratch.path()],
        "the overlay's `.` mount follows the binary's, so it wins the dedup"
    );

    let status = deployment
        .run_with::<production_plugins::Hooks, _>(Backends::defaults().await)
        .await
        .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}

#[tokio::test]
async fn scripted_model() {
    let backends = Backends::defaults().await.model(ScriptedModel::answering(["second"]));
    let status = Deployment::new()
        .guest("echo", test_programs::MODEL_ECHO_TEXT)
        .run_host::<WasiModel, _>(backends.clone())
        .await
        .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS);

    let seen = backends.model.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].system.as_deref(), Some("be terse"));
    assert_eq!(seen[0].messages, ["hi", "second"]);
    assert_eq!(seen[0].format, SeenFormat::Text);
    assert!(seen[0].workspace.is_none());
    backends.model.assert_exhausted();
}

#[tokio::test]
async fn scripted_calls() {
    let model = ScriptedModel::answering(["42"]).calling(0, [("lookup", "{}")]);
    let backends = Backends::defaults().await.model(model);
    let status = Deployment::new()
        .guest("tools", test_programs::MODEL_TOOL_ROUNDTRIP)
        .command("tools")
        .run_host::<WasiModel, _>(backends.clone())
        .await
        .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS);

    assert_eq!(backends.model.seen()[0].tools, ["lookup"]);
    assert_eq!(
        backends.model.exchanges(),
        [Exchange {
            tool: "lookup".into(),
            arguments: "{}".into(),
            outcome: Ok("42".into()),
        }]
    );
}

#[tokio::test]
async fn exhausted_script() {
    let backends = Backends::defaults().await.model(ScriptedModel::default());
    let outcome = Deployment::new()
        .guest("echo", test_programs::MODEL_ECHO_TEXT)
        .run_host::<WasiModel, _>(backends.clone())
        .await;
    assert!(!matches!(outcome, Ok(ExitStatus::SUCCESS)), "the guest's expect fails: {outcome:?}");
    assert_eq!(backends.model.seen().len(), 1, "the request was still recorded");
    // The soft answer to the guest is not a soft answer to the test: the
    // overrun fails the assertion the scenario would normally end with.
    let asserted = catch_unwind(AssertUnwindSafe(|| backends.model.assert_exhausted()));
    assert!(asserted.is_err(), "the overrun fails assert_exhausted()");
}

#[tokio::test]
async fn then_answers() {
    let model = ScriptedModel::answering::<String>([]).then(|| "second".to_owned());
    let backends = Backends::defaults().await.model(model);
    let status = Deployment::new()
        .guest("echo", test_programs::MODEL_ECHO_TEXT)
        .run_host::<WasiModel, _>(backends)
        .await
        .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS);
}

#[tokio::test]
async fn link_pair() {
    let runtime = Deployment::new()
        .guest("echoer", test_programs::LINK_ECHOER)
        .guest("full", test_programs::LINK_FULL)
        .boot(Backends::defaults().await, |_| Ok(()))
        .await
        .expect("deployment boots");

    let answer = call(&runtime, "full", "poke", "hi").await.expect("dispatch");
    assert_eq!(answer, "echoer pong: hi");
    runtime.shutdown();
}

// An on-demand guest is admitted on its first `load`; nothing else opts the
// deployment into the loader.
#[tokio::test]
async fn on_demand_guest() {
    let status = Deployment::new()
        .guest("requester", test_programs::PLUGINS_LOAD)
        .on_demand("plugin", test_programs::LINK_ECHOER)
        .args(["plugin"])
        .run(Backends::defaults().await, |deployment| {
            deployment.host::<WasiOtel, Backends>()?;
            Ok(())
        })
        .await
        .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}

// A boot guest's `digest` pin is checked as the deployment builds: the
// bytes' own digest runs, any other fails startup before the guest loads.
#[tokio::test]
async fn pinned_boot_guest() {
    fn otel_only(deployment: &mut omnia::Deployment<StoreCtx<Backends>>) -> Result<()> {
        deployment.host::<WasiOtel, Backends>()?;
        Ok(())
    }
    let bytes = std::fs::read(test_programs::COMMAND_EXIT_MAP).expect("reading the guest");
    let pinned = |digest: Digest| {
        Deployment::new()
            .entry(GuestEntry::new("cli", test_programs::COMMAND_EXIT_MAP).digest(digest))
            .args(["ok"])
    };

    let status = pinned(Digest::of(&bytes))
        .run(Backends::defaults().await, otel_only)
        .await
        .expect("the pin matches the guest's bytes");
    assert_eq!(status, ExitStatus::SUCCESS);

    let error = pinned(Digest::of(b"some other component"))
        .run(Backends::defaults().await, otel_only)
        .await
        .expect_err("the pin names other bytes");
    assert!(format!("{error:#}").contains("not its declared digest"), "{error:#}");
}

/// A keyvalue backend that notes every bucket opened through it, then hands
/// the call to the in-memory default it wraps.
#[derive(Clone, Debug)]
struct RecordingKeyValue {
    inner: KeyValueDefault,
    opened: Arc<Mutex<Vec<String>>>,
}

impl WasiKeyValueCtx for RecordingKeyValue {
    fn open_bucket(&self, identifier: String) -> FutureResult<Arc<dyn Bucket>> {
        self.opened.lock().expect("opened buckets").push(identifier.clone());
        self.inner.open_bucket(identifier)
    }
}

#[tokio::test]
async fn swapped_keyvalue() {
    let defaults = Backends::defaults().await;
    let recording = RecordingKeyValue {
        inner: defaults.keyvalue.clone(),
        opened: Arc::default(),
    };
    // The guest's atomics start from a host-seeded counter.
    let bucket = recording.inner.open_bucket("bucket".to_owned()).await.expect("bucket");
    bucket.set("counter".to_owned(), 37_i64.to_be_bytes().to_vec()).await.expect("seed");

    let backends = defaults.keyvalue(recording);
    let status = Deployment::new()
        .guest("guest", test_programs::KEYVALUE_BUCKET)
        .run_host::<WasiKeyValue, _>(backends.clone())
        .await
        .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS);

    // The guest reached the swapped-in backend, and its writes landed in
    // the store behind it.
    let opened = backends.keyvalue.opened.lock().expect("opened buckets").clone();
    assert_eq!(opened, ["bucket"]);
    let bucket = backends.keyvalue.inner.open_bucket("bucket".to_owned()).await.expect("bucket");
    assert_eq!(
        bucket.get("counter".to_owned()).await.expect("get"),
        Some(42_i64.to_be_bytes().to_vec())
    );
}

#[tokio::test]
async fn host_handles() {
    let backends = Backends::defaults().await;
    let bucket = backends.keyvalue.open_bucket("cache".to_owned()).await.expect("bucket");
    bucket.set("k".to_owned(), b"v".to_vec()).await.expect("set");
    assert_eq!(backends.state("k").await, Some(b"v".to_vec()));
    assert_eq!(backends.state("missing").await, None);

    let container = backends.blobstore.create_container("c".to_owned()).await.expect("container");
    container.write_data("o".to_owned(), b"bytes".to_vec().into()).await.expect("write");
    assert_eq!(backends.object("c", "o").await, Some(b"bytes".to_vec()));
    assert_eq!(backends.object("absent", "o").await, None);
}

#[test]
fn scratch_mounts() {
    let scratch = scratch();
    scratch.write("nested/file.txt", "hello");
    assert_eq!(scratch.read("nested/file.txt"), Some(b"hello".to_vec()));
    let mount = scratch.mount(true);
    assert_eq!((mount.name.as_str(), mount.writable), (".", true));
    assert_eq!(scratch.mount_as("project", false).name, "project");
    assert_eq!(mount.path, scratch.path());
}

/// Instantiate `guest` fresh and drive its exported `func` with one string
/// argument, returning the string result.
async fn call<B>(runtime: &Runtime<B>, guest: &str, func: &str, message: &str) -> Result<String>
where
    B: Clone + Send + Sync + 'static,
{
    let entry = runtime
        .registry()
        .get(&GuestId::from(guest))
        .with_context(|| format!("guest `{guest}` is not registered"))?;
    let mut store = runtime.build_store(runtime.store());
    let instance = runtime.instantiate(entry.instance_pre(), &mut store).await?;
    let export = instance
        .get_func(&mut store, func)
        .with_context(|| format!("guest `{guest}` exports `{func}`"))?;
    let mut results = vec![Val::Bool(false)];
    export
        .call_async(&mut store, &[Val::String(message.to_owned())], &mut results)
        .await
        .map_err(anyhow::Error::from)?;
    match results.into_iter().next() {
        Some(Val::String(answer)) => Ok(answer),
        other => bail!("`{guest}`'s `{func}` returned a non-string result: {other:?}"),
    }
}
