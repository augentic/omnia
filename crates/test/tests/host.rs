//! The component rung: real guest components from `crates/test-programs`
//! driven through `Deployment` over `Backends`.

#![cfg(not(target_arch = "wasm32"))]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, bail};
use axum::Router;
use axum::routing::get;
use omnia::wasmtime::component::Val;
use omnia::{DeploymentBuilder, Digest, ExitStatus, GuestEntry, GuestId, Runtime};
use omnia_test::host::{Backends, Deployment, ScriptedModel, ScriptedVcs, scratch};
use omnia_test::{Exchange, SeenFormat};
use omnia_wasi_blobstore::{BlobStoreClient as _, WasiBlobStore};
use omnia_wasi_config::WasiConfig;
use omnia_wasi_docstore::WasiDocStore;
use omnia_wasi_http::WasiHttp;
use omnia_wasi_identity::WasiIdentity;
use omnia_wasi_keyvalue::{
    BUCKET, Bucket, FutureResult, KeyValueClient, KeyValueDefault, WasiKeyValue,
};
use omnia_wasi_messaging::WasiMessaging;
use omnia_wasi_model::WasiModel;
use omnia_wasi_otel::WasiOtel;
use omnia_wasi_sql::WasiSql;
use omnia_wasi_vault::WasiVault;
use omnia_wasi_vcs::WasiVcs;
use omnia_wasi_websocket::WasiWebSocket;
use tokio::net::TcpListener;

// notes every bucket opened, then hands the call to the default it wraps
#[derive(Clone, Debug)]
struct RecordingKeyValue {
    inner: KeyValueDefault,
    opened: Arc<Mutex<Vec<String>>>,
}

impl KeyValueClient for RecordingKeyValue {
    fn open_bucket(&self, identifier: String) -> FutureResult<Arc<dyn Bucket>> {
        self.opened.lock().expect("opened buckets").push(identifier.clone());
        self.inner.open_bucket(identifier)
    }
}

test_programs::foreach_conformance!();

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

// Two backends reached through paths that end in the same name: the bundle's
// fields are positional, so neither shadows the other.
mod same_named {
    use omnia_wasi_keyvalue::WasiKeyValue;
    use omnia_wasi_model::WasiModel;
    use omnia_wasi_otel::{OtelDefault, WasiOtel};

    mod keyvalue {
        pub use omnia_wasi_keyvalue::KeyValueDefault as Client;
    }
    mod model {
        pub use omnia_wasi_model::ModelDefault as Client;
    }

    omnia::runtime!({
        mode: command,
        hosts: {
            WasiKeyValue: keyvalue::Client,
            WasiModel: model::Client,
            WasiOtel: OtelDefault,
        },
    });
}

// `production` in the list form: each backend names the hosts it serves, and
// the keyvalue entry carries its connect options and narrows its hosts.
mod production_list {
    use omnia_wasi_keyvalue::{KeyValueDefault, WasiKeyValue};
    use omnia_wasi_model::ModelDefault;
    use omnia_wasi_otel::OtelDefault;

    omnia::runtime!({
        mode: command,
        hosts: [
            ModelDefault,
            KeyValueDefault(omnia::NoOptions) for [WasiKeyValue],
            OtelDefault,
        ],
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
// deployment — a guest loaded at first use included — runs through the
// binary's hooks.
#[tokio::test]
async fn overlay_mount() {
    let _ = (production_plugins::main, production_plugins::run);
    let scratch = scratch();

    let deployment = Deployment::from(production_plugins::manifest())
        .guest("requester", test_programs::PLUGINS_LOAD)
        .entry(GuestEntry::new("plugin", test_programs::LINK_ECHOER))
        .args(["declared", "plugin"])
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
async fn same_named_backends() {
    let _ = (same_named::main, same_named::run);
    let backends = Backends::defaults().await.model(ScriptedModel::answering(["second"]));
    let status = Deployment::from(same_named::manifest())
        .guest("echo", test_programs::MODEL_ECHO_TEXT)
        .run_with::<same_named::Hooks, _>(backends)
        .await
        .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS);
}

#[tokio::test]
async fn list_overlay() {
    let _ = production_list::main;
    let backends = Backends::defaults().await.model(ScriptedModel::answering(["second"]));
    let status = Deployment::from(production_list::manifest())
        .guest("echo", test_programs::MODEL_ECHO_TEXT)
        .run_with::<production_list::Hooks, _>(backends.clone())
        .await
        .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS);

    // the list's wiring linked `omnia:model` over the bundle handed in
    assert_eq!(backends.model.seen().len(), 1);
    backends.model.assert_exhausted();
}

// The generated `run` owns its tokio runtime, connects the list's backends,
// and fills their slots; the guest's own assertions are the read-back.
#[test]
fn list_connected() {
    let manifest = Deployment::new()
        .guest("guest", test_programs::KEYVALUE_TTL_EXPIRES)
        .manifest()
        .expect("manifest");

    let status =
        production_list::run(DeploymentBuilder::new().manifest(manifest)).expect("deployment runs");

    assert_eq!(status, ExitStatus::SUCCESS);
}

// Two list entries serving one host reach `Deployment::hosts` as two groups.
#[tokio::test]
async fn linked_twice() {
    let error = Deployment::new()
        .guest("guest", test_programs::KEYVALUE_TTL_EXPIRES)
        .run(Backends::defaults().await, |deployment| {
            deployment.hosts::<(WasiKeyValue, WasiOtel)>()?;
            deployment.hosts::<(WasiKeyValue,)>()?;
            Ok(())
        })
        .await
        .expect_err("should refuse the second group");

    assert!(format!("{error:#}").contains("two backends serve `wasi:keyvalue`"), "{error:#}");
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

// A declared path guest loads at its first use — here the requester's
// `load` of its name; nothing else opts the deployment into the loader.
#[tokio::test]
async fn declared_guest() {
    let status = Deployment::new()
        .guest("requester", test_programs::PLUGINS_LOAD)
        .entry(GuestEntry::new("plugin", test_programs::LINK_ECHOER))
        .args(["declared", "plugin"])
        .run(Backends::defaults().await, |deployment| {
            deployment.host::<WasiOtel>()?;
            Ok(())
        })
        .await
        .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the requester's assertions all held");
}

// A path guest's `digest` pin is checked at its first use — here the command
// drive, which names it as the marked command guest: the bytes' own digest
// runs, any other fails the run before the guest loads.
#[tokio::test]
async fn pinned_path_guest() {
    fn otel_only(deployment: &mut omnia::Deployment<Backends>) -> Result<()> {
        deployment.host::<WasiOtel>()?;
        Ok(())
    }
    let bytes = std::fs::read(test_programs::COMMAND_EXIT_MAP).expect("reading the guest");
    let pinned = |digest: Digest| {
        Deployment::new()
            .entry(GuestEntry::new("cli", test_programs::COMMAND_EXIT_MAP).digest(digest).command())
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

#[tokio::test]
async fn swapped_keyvalue() {
    let defaults = Backends::defaults().await;
    let recording = RecordingKeyValue {
        inner: defaults.keyvalue.clone(),
        opened: Arc::default(),
    };
    // The guest's atomics start from a host-seeded counter.
    let bucket = recording.inner.open_bucket("bucket".to_owned()).await.expect("bucket");
    bucket.set("counter".to_owned(), 37_i64.to_be_bytes().to_vec(), None).await.expect("seed");

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
    let bucket = backends.keyvalue.open_bucket(BUCKET.to_owned()).await.expect("bucket");
    bucket.set("k".to_owned(), b"v".to_vec(), None).await.expect("set");
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

async fn serve_origin() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let app = Router::new().route("/conformance", get(|| async { "conformance-ok" }));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

// The conformance scenarios the handler rung runs over `guest::Provider`,
// inside a real WASI guest over `WasiProvider` and every default host.
#[tokio::test]
async fn conformance_portable() {
    let url = serve_origin().await;
    let scratch = scratch();
    let vcs = ScriptedVcs::resolving(["sha:main"]);

    let backends = Backends::defaults()
        .await
        .config([("CONFORMANCE_KEY", "conformance-val".to_owned()), ("CONFORMANCE_ORIGIN", url)])
        .vcs(vcs.clone());

    let status = Deployment::new()
        .guest("conformance", test_programs::CONFORMANCE_PORTABLE)
        .mount(scratch.mount(false))
        .run(backends, |deployment| {
            deployment.host::<WasiBlobStore>()?;
            deployment.host::<WasiConfig>()?;
            deployment.host::<WasiDocStore>()?;
            deployment.host::<WasiHttp>()?;
            deployment.host::<WasiIdentity>()?;
            deployment.host::<WasiKeyValue>()?;
            deployment.host::<WasiMessaging>()?;
            deployment.host::<WasiModel>()?;
            deployment.host::<WasiOtel>()?;
            deployment.host::<WasiSql>()?;
            deployment.host::<WasiVault>()?;
            deployment.host::<WasiVcs>()?;
            deployment.host::<WasiWebSocket>()?;
            Ok(())
        })
        .await
        .expect("conformance guest runs");
    assert_eq!(status, ExitStatus::SUCCESS);
    vcs.assert_exhausted();
}

async fn call<B: omnia::Backends>(
    runtime: &Runtime<B>, guest: &str, func: &str, message: &str,
) -> Result<String> {
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
