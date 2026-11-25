//! The component rung: real guest components from `crates/test-programs`
//! driven through `Deployment` over `Backends`.

#![cfg(not(target_arch = "wasm32"))]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use bytes::Bytes;
use http::header::CONTENT_TYPE;
use http::{Method, Request, Response, StatusCode};
use omnia::wasmtime::component::{ResourceTable, Val};
use omnia::{Digest, ExitStatus, Grants, GuestEntry, GuestId, HostCtx, Runtime, SourceSpec};
use omnia_test::host::sql::{Field, Row, Value};
use omnia_test::host::{
    Asked, Backends, Deployment, Ended, FixedIdentity, HandleError, MatchedHttp, STATE_BUCKET,
    ScriptedModel, ScriptedTables, Sink, deliver, precompiled, scratch, send,
};
use omnia_wasi_blobstore::BlobStoreClient as _;
use omnia_wasi_http::WasiHttp;
use omnia_wasi_identity::WasiIdentity;
use omnia_wasi_keyvalue::{KeyValueClient, Lifetime, WasiKeyValue};
use omnia_wasi_messaging::WasiMessaging;
use omnia_wasi_model::WasiModel;
use omnia_wasi_otel::WasiOtel;
use omnia_wasi_sql::WasiSql;
use omnia_wasi_websocket::WasiWebSocket;
use serde_json::json;

test_programs::foreach_harness!();

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

// every model test here boots it, so it is compiled once
fn echo_text() -> SourceSpec {
    precompiled(test_programs::MODEL_ECHO_TEXT).expect("the echo guest precompiles")
}

#[tokio::test]
async fn runtime_overlay() {
    // The generated `main` and `run` stay untouched; the overlay reaches the
    // same `Hooks` through `run_with`.
    let _ = (production::main, production::run);
    let backends = Backends::defaults().await.model(ScriptedModel::answering(["second"]));
    let status = Deployment::from(production::manifest())
        .guest("echo", echo_text())
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
    assert!(!manifest.mounts[0].writable, "a mount without `writable:` is read-only");

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
        .guest("echo", echo_text())
        .run_with::<same_named::Hooks, _>(backends)
        .await
        .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS);
}

#[tokio::test]
async fn exhausted_script() {
    // an overrun fails the guest's request and the test's final check
    let backends = Backends::defaults().await.model(ScriptedModel::default());
    let outcome = Deployment::new()
        .guest("echo", echo_text())
        .run_host::<WasiModel, _>(backends.clone())
        .await;
    assert!(!matches!(outcome, Ok(ExitStatus::SUCCESS)), "the guest's expect fails: {outcome:?}");
    assert_eq!(backends.model.seen().len(), 1, "the request was still recorded");
    // The soft answer to the guest is not a soft answer to the test: the
    // overrun fails the assertion the scenario would normally end with.
    let asserted = catch_unwind(AssertUnwindSafe(|| backends.model.assert_exhausted()));
    assert!(asserted.is_err(), "the overrun fails assert_exhausted()");

    // a `then` fallback answers past the script's end instead
    let model = ScriptedModel::answering::<String>([]).then(|| "second".to_owned());
    let backends = Backends::defaults().await.model(model);
    let status = Deployment::new()
        .guest("echo", echo_text())
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
async fn host_handles() {
    let backends = Backends::defaults().await;
    let bucket = backends.keyvalue.open_bucket(STATE_BUCKET.to_owned()).await.expect("bucket");
    let set = bucket.set("k".to_owned(), b"v".to_vec(), Lifetime::Replace(None), None);
    set.await.expect("set").expect("no precondition");
    assert_eq!(backends.state("k").await, Some(b"v".to_vec()));
    assert_eq!(backends.state("missing").await, None);

    let container = backends.blobstore.create_container("c".to_owned()).await.expect("container");
    container.write_data("o".to_owned(), b"bytes".to_vec().into()).await.expect("write");
    assert_eq!(backends.object("c", "o").await, Some(b"bytes".to_vec()));
    assert_eq!(backends.object("absent", "o").await, None);
}

trait EchoClient {
    fn name(&self) -> &'static str;
}

struct Named;

impl EchoClient for Named {
    fn name(&self) -> &'static str {
        "named"
    }
}

#[derive(HostCtx)]
#[host_ctx(client = EchoClient, crate = omnia)]
struct WasiEcho;

// A host a deployment crate defines for itself. This crate reaches
// `omnia-core` only through `omnia`, as a deployment does, so the derive
// resolves here only under `crate = omnia`.
#[test]
fn facade_host() {
    let (mut table, grants) = (ResourceTable::new(), Arc::new(Grants::all()));
    let view = WasiEcho::view(&Named, &mut table, &grants);
    assert_eq!(view.client.name(), "named");
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

#[tokio::test]
async fn harness_flow() {
    let tables = ScriptedTables::default()
        .on_query(
            |sql, params| sql.starts_with("SELECT name") && params == [Value::Int64(Some(7))],
            vec![Row {
                fields: vec![Field {
                    name: "name".to_owned(),
                    value: Value::Str(Some("widget".to_owned())),
                }],
            }],
        )
        .on_exec(|sql, _| sql.starts_with("UPDATE items"), 1);
    let price = Response::new(Bytes::from_static(br#"{"price":12}"#));
    let http = MatchedHttp::default().on(Method::GET, "https://prices.test/items/7", price);
    let sink = Sink::default();
    let identity = FixedIdentity::new("secret");
    let backends = Backends::defaults()
        .await
        .sql(tables.clone())
        .http(http.clone())
        .messaging(sink.clone())
        .websocket(sink.clone())
        .identity(identity.clone());
    let cache = backends.keyvalue.open_bucket(STATE_BUCKET.to_owned()).await.expect("bucket");
    let cached = cache.set("names.7".to_owned(), b"widget".to_vec(), Lifetime::Replace(None), None);
    cached.await.expect("set").expect("no precondition");
    let flow = precompiled(test_programs::HARNESS_FLOW).expect("flow precompiles");
    let runtime = Deployment::new()
        .guest("flow", flow)
        .boot(backends.clone(), |deployment| {
            deployment.host::<WasiHttp>()?;
            deployment.host::<WasiIdentity>()?;
            deployment.host::<WasiKeyValue>()?;
            deployment.host::<WasiMessaging>()?;
            deployment.host::<WasiOtel>()?;
            deployment.host::<WasiSql>()?;
            deployment.host::<WasiWebSocket>()?;
            Ok(())
        })
        .await
        .expect("runtime boots");

    // a rename reaches every backend
    let response = send(&runtime, rename(7)).await.expect("rename served");
    assert_eq!(response.status(), StatusCode::CREATED, "{:?}", response.body());
    let reply: serde_json::Value = serde_json::from_slice(response.body()).expect("json reply");
    assert_eq!(reply, json!({"previous": "widget", "price": 12, "token": "secret"}));

    let transactions = tables.transactions();
    let [transaction] = transactions.as_slice() else {
        panic!("one transaction, not {transactions:?}");
    };
    assert_eq!(transaction.ended, Some(Ended::Committed));
    let updated: Vec<_> = transaction.statements.iter().map(|executed| &executed.params).collect();
    assert_eq!(updated, [&[Value::Str(Some("gadget".to_owned())), Value::Int64(Some(7))]]);
    let [priced] = http.requests().try_into().expect("one upstream request");
    assert_eq!(priced.headers["authorization"], "Bearer secret");
    let asked = Asked {
        identity: "default".to_owned(),
        scopes: vec!["prices.read".to_owned()],
    };
    assert_eq!(identity.asked(), [asked]);
    assert_eq!(backends.state("prices.7").await, Some(b"12".to_vec()));
    assert_eq!(backends.state("names.7").await, None);
    let [renamed] = sink.sent().try_into().expect("one message sent");
    assert_eq!(
        (renamed.topic.as_str(), renamed.payload.as_slice()),
        ("items.renamed", &b"gadget"[..])
    );
    assert_eq!(renamed.metadata.expect("metadata").inner["key"], "7");
    let [announced] = sink.broadcasts().try_into().expect("one broadcast");
    assert_eq!((announced.data, announced.sockets), (b"gadget".to_vec(), None));

    // an unscripted statement fails in the reply
    let response = send(&runtime, rename(8)).await.expect("rename served");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let reply = String::from_utf8_lossy(response.body());
    assert!(reply.contains("no rule matches `SELECT name FROM items WHERE id = $1`"), "{reply}");

    // one delivery handled, and one rejected
    deliver(&runtime, "items.refresh", "7").await.expect("refresh handled");
    let refreshed = sink.sent().pop().expect("a message sent");
    assert_eq!(
        (refreshed.topic.as_str(), refreshed.payload.as_slice()),
        ("items.refreshed", &b"7"[..])
    );
    let rejected = deliver(&runtime, "items.refresh", "").await.expect_err("empty refresh");
    assert!(matches!(&rejected, HandleError::Rejected(reason) if reason == "an empty refresh"));

    runtime.shutdown();
}

fn rename(id: i64) -> Request<Bytes> {
    Request::post(format!("/items/{id}"))
        .header(CONTENT_TYPE, "application/json")
        .body(Bytes::from_static(br#"{"name":"gadget"}"#))
        .expect("a valid request")
}

#[tokio::test]
async fn harness_http_hosts() {
    let price = Response::new(Bytes::from_static(br#"{"price":12}"#));
    let http = MatchedHttp::default().on(Method::GET, "https://prices.test/items/7", price);
    let grants = Grants::none().hosts(["prices.test"]).interfaces(["*"]);
    let guest = GuestEntry::new("hosts", test_programs::HARNESS_HTTP_HOSTS).command();

    let status = Deployment::new()
        .entry(guest.grants(grants))
        .run(Backends::defaults().await.http(http.clone()), |deployment| {
            deployment.host::<WasiHttp>()?;
            deployment.host::<WasiOtel>()?;
            Ok(())
        })
        .await
        .expect("deployment runs");
    assert_eq!(status, ExitStatus::SUCCESS, "the guest's assertions held");

    let [priced] = http.requests().try_into().expect("one request recorded");
    assert_eq!(priced.uri, "https://prices.test/items/7");
}

#[test]
fn rebuilt_component() {
    let scratch = scratch();
    let wasm = scratch.path().join("guest.wasm");
    let artifacts = || {
        std::fs::read_dir(scratch.path().join("precompiled"))
            .expect("the artifacts' directory")
            .map(|entry| entry.expect("an artifact").file_name())
            .filter(|name| name.to_string_lossy().ends_with(".bin"))
            .collect::<Vec<_>>()
    };

    std::fs::copy(test_programs::LINK_HANDLE, &wasm).expect("staging the first build");
    precompiled(&wasm).expect("the first build precompiles");
    let first = artifacts();
    std::fs::copy(test_programs::LINK_PARTIAL, &wasm).expect("staging the second build");
    precompiled(&wasm).expect("the second build precompiles");
    let second = artifacts();

    assert_eq!(first.len(), 1, "{first:?}");
    assert_eq!(second.len(), 1, "the second build's artifact replaces the first's: {second:?}");
    assert_ne!(first, second);
}

async fn call<B: omnia::Backends>(
    runtime: &Runtime<B>, guest: &str, func: &str, message: &str,
) -> Result<String> {
    let entry = runtime
        .registry()
        .get(&GuestId::from(guest))
        .with_context(|| format!("guest `{guest}` is not registered"))?;
    let results = runtime
        .lease(entry)
        .await
        .invoke(async |mut entered, instance, _bound| {
            let export = instance
                .get_func(&mut entered, func)
                .with_context(|| format!("guest `{guest}` exports `{func}`"))?;
            let mut results = vec![Val::Bool(false)];
            export
                .call_async(&mut entered, &[Val::String(message.to_owned())], &mut results)
                .await
                .map_err(anyhow::Error::from)?;
            anyhow::Ok(results)
        })
        .await??;
    match results.into_iter().next() {
        Some(Val::String(answer)) => Ok(answer),
        other => bail!("`{guest}`'s `{func}` returned a non-string result: {other:?}"),
    }
}
