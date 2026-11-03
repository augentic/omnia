//! Shared provider conformance scenarios across `omnia_test::guest::Provider`,
//! `omnia_sdk::WasiProvider`, and `omnia_dapr::Dapr`.
//!
//! Each function exercises one row of the portable semantic contract over the
//! `omnia_sdk` capability trait alone, panicking on any deviation. Use the
//! [`conformance!`](crate::conformance!) macro in a test module to require a
//! paired test for every scenario in the selected capabilities.

use std::time::Duration;

use bytes::Bytes;
use http::Request;
use http_body_util::Full;
use omnia_sdk::docstore::{Document, Filter, QueryOptions, SortField};
use omnia_sdk::model::{self, Message as ModelMessage, Model, Request as ModelRequest, Role};
use omnia_sdk::sql::{DataType, Row};
use omnia_sdk::vcs::{self, Vcs};
use omnia_sdk::{
    BlobStore, Broadcast, CasError, Config, DocumentStore, HttpRequest, Identity, Message, Publish,
    StateStore, TableStore, Vault,
};

/// Asserts that `StateStore` round-trips opaque binary bytes.
///
/// Covers `set`, `get`, overwrite, and idempotent `delete`.
///
/// # Panics
///
/// Panics if any `StateStore` operation fails or returns unexpected bytes.
pub async fn state_get_set_delete<P: StateStore>(provider: &P) {
    let key = "conformance/get-set-delete";
    let payload: &[u8] = &[0x00, 0xff, 0xfe, b'o', b'm', b'n', b'i', b'a'];

    provider.delete(key).await.expect("delete before test");
    assert_eq!(provider.get(key).await.expect("get absent"), None);

    provider.set(key, payload, None).await.expect("set binary payload");
    assert_eq!(provider.get(key).await.expect("get binary"), Some(payload.to_vec()));

    provider.set(key, b"overwritten", None).await.expect("overwrite");
    assert_eq!(provider.get(key).await.expect("get overwritten"), Some(b"overwritten".to_vec()));

    provider.delete(key).await.expect("delete present");
    assert_eq!(provider.get(key).await.expect("get after delete"), None);
    provider.delete(key).await.expect("delete absent is idempotent");
}

/// Asserts that a TTL-bearing key expires (including across a CAS swap) while
/// a TTL-less key persists.
///
/// # Panics
///
/// Panics if the TTL keys do not expire or the persistent key is lost.
pub async fn state_ttl_expires<P: StateStore>(provider: &P) {
    let ephemeral = "conformance/ttl-ephemeral";
    let cas_ttl = "conformance/cas-ttl";
    let persistent = "conformance/ttl-persistent";

    provider.set(ephemeral, b"short-lived", Some(1)).await.expect("set ephemeral");
    provider.set(cas_ttl, b"t1", Some(1)).await.expect("set cas_ttl");
    provider.cas(cas_ttl, Some(b"t1"), b"t2").await.expect("cas preserves ttl");
    provider.set(persistent, b"long-lived", None).await.expect("set persistent");

    assert_eq!(
        provider.get(ephemeral).await.expect("get ephemeral"),
        Some(b"short-lived".to_vec())
    );
    assert_eq!(provider.get(cas_ttl).await.expect("get cas_ttl"), Some(b"t2".to_vec()));
    assert_eq!(
        provider.get(persistent).await.expect("get persistent"),
        Some(b"long-lived".to_vec())
    );

    sleep(Duration::from_millis(1100)).await;

    assert_eq!(provider.get(ephemeral).await.expect("ephemeral expired"), None);
    assert_eq!(provider.get(cas_ttl).await.expect("cas-swapped ttl_key expired"), None);
    assert_eq!(
        provider.get(persistent).await.expect("persistent survives"),
        Some(b"long-lived".to_vec())
    );
    provider.delete(persistent).await.expect("cleanup persistent");
}

/// Asserts atomic compare-and-swap over absent and present keys.
///
/// Covers conflict reporting with the observed value.
///
/// # Panics
///
/// Panics if CAS semantics deviate from the contract.
pub async fn state_cas<P: StateStore>(provider: &P) {
    let key = "conformance/cas";
    provider.delete(key).await.expect("cleanup cas key");

    provider.cas(key, None, b"v1").await.expect("cas from absent");
    assert_eq!(
        provider.cas(key, None, b"stale").await,
        Err(CasError::Conflict(Some(b"v1".to_vec())))
    );
    provider.cas(key, Some(b"v1"), b"v2").await.expect("cas matching v1");
    assert_eq!(provider.get(key).await.expect("get v2"), Some(b"v2".to_vec()));
    provider.delete(key).await.expect("cleanup cas key");
}

/// Asserts atomic increment starting from zero on an absent key.
///
/// Applies signed deltas over an 8-byte big-endian integer.
///
/// # Panics
///
/// Panics if increment results or stored bytes do not match.
pub async fn state_increment<P: StateStore>(provider: &P) {
    let key = "conformance/increment";
    provider.delete(key).await.expect("cleanup increment key");

    assert_eq!(provider.increment(key, 5).await.expect("increment from absent"), 5);
    assert_eq!(provider.increment(key, -2).await.expect("increment negative delta"), 3);
    assert_eq!(
        provider.get(key).await.expect("get raw counter bytes"),
        Some(3_i64.to_be_bytes().to_vec())
    );
    provider.delete(key).await.expect("cleanup increment key");
}

/// Asserts parameterized SQL execution and querying through `TableStore`.
///
/// # Panics
///
/// Panics if `exec` or `query` fails or returns unexpected rows.
pub async fn table_query_exec<P: TableStore>(provider: &P) {
    provider
        .exec(
            "conformance".to_owned(),
            "CREATE TABLE IF NOT EXISTS conformance_items (id INTEGER PRIMARY KEY, label TEXT NOT NULL)"
                .to_owned(),
            vec![],
        )
        .await
        .expect("create table");

    let affected = provider
        .exec(
            "conformance".to_owned(),
            "INSERT INTO conformance_items (id, label) VALUES ($1, $2)".to_owned(),
            vec![DataType::Int64(Some(1)), DataType::Str(Some("alpha".to_owned()))],
        )
        .await
        .expect("insert row");
    assert_eq!(affected, 1);

    let rows: Vec<Row> = provider
        .query(
            "conformance".to_owned(),
            "SELECT id, label FROM conformance_items WHERE id = $1".to_owned(),
            vec![DataType::Int64(Some(1))],
        )
        .await
        .expect("select row");
    assert_eq!(rows.len(), 1);
    let label = rows[0].fields.iter().find(|f| f.name == "label").expect("label column");
    assert_eq!(label.value, DataType::Str(Some("alpha".to_owned())));
}

/// Asserts `DocumentStore` CRUD, duplicate-insert refusal, and filtered queries.
///
/// Also exercises sorting and continuation-token pagination.
///
/// # Panics
///
/// Panics if document operations deviate from the contract.
pub async fn docstore_crud_query<P: DocumentStore>(provider: &P) {
    let store = "conformance_docs";
    let d1 = Document {
        id: "1".to_owned(),
        data: br#"{"name":"ann","age":30}"#.to_vec(),
    };
    provider.insert(store, &d1).await.expect("insert d1");
    provider.insert(store, &d1).await.expect_err("duplicate insert fails");

    let d1_updated = Document {
        id: "1".to_owned(),
        data: br#"{"name":"anne","age":30}"#.to_vec(),
    };
    let d2 = Document {
        id: "2".to_owned(),
        data: br#"{"name":"bob","age":20}"#.to_vec(),
    };
    let d3 = Document {
        id: "3".to_owned(),
        data: br#"{"name":"cara","age":40}"#.to_vec(),
    };
    provider.put(store, &d1_updated).await.expect("put d1");
    provider.put(store, &d2).await.expect("put d2");
    provider.put(store, &d3).await.expect("put d3");

    let fetched = provider.get(store, "1").await.expect("get d1").expect("d1 exists");
    assert_eq!(fetched.id, "1");

    let first_page = provider
        .query(
            store,
            QueryOptions {
                filter: Some(Filter::gte("age", 25)),
                order_by: vec![SortField {
                    field: "age".to_owned(),
                    descending: true,
                }],
                limit: Some(1),
                ..QueryOptions::default()
            },
        )
        .await
        .expect("query page 1");
    assert_eq!(first_page.documents.len(), 1);
    assert_eq!(first_page.documents[0].id, "3");
    assert!(first_page.continuation.is_some());

    let second_page = provider
        .query(
            store,
            QueryOptions {
                filter: Some(Filter::gte("age", 25)),
                order_by: vec![SortField {
                    field: "age".to_owned(),
                    descending: true,
                }],
                limit: Some(1),
                continuation: first_page.continuation,
                ..QueryOptions::default()
            },
        )
        .await
        .expect("query page 2");
    assert_eq!(second_page.documents.len(), 1);
    assert_eq!(second_page.documents[0].id, "1");
    assert!(second_page.continuation.is_none());

    assert!(provider.delete(store, "1").await.expect("delete d1"));
    assert!(!provider.delete(store, "1").await.expect("delete d1 again"));
}

/// Asserts `BlobStore` container and object operations.
///
/// Covers missing-object `Ok(None)`, object read/write, inclusive byte-range read, metadata, copy/move, and deletion.
///
/// # Panics
///
/// Panics if blob operations deviate from the contract.
pub async fn blobstore_objects<P: BlobStore>(provider: &P) {
    let container = "conformance-blobs";
    provider.create_container(container).await.expect("create container");
    assert!(provider.container_exists(container).await.expect("container exists"));
    assert_eq!(provider.container_info(container).await.expect("container info").name, container);

    assert_eq!(provider.get(container, "missing").await.expect("missing is None"), None);

    provider.put(container, "digits", b"0123456789").await.expect("put digits");
    assert_eq!(
        provider.get(container, "digits").await.expect("get digits"),
        Some(b"0123456789".to_vec())
    );
    assert_eq!(provider.get_range(container, "digits", 2, 5).await.expect("get_range"), b"2345");

    let info = provider.object_info(container, "digits").await.expect("object info");
    assert_eq!(info.name, "digits");
    assert_eq!(info.size, 10);

    provider.copy_object(container, "digits", container, "copy").await.expect("copy_object");
    provider.move_object(container, "copy", container, "moved").await.expect("move_object");
    let mut names = provider.list(container).await.expect("list");
    names.sort();
    assert_eq!(names, ["digits", "moved"]);

    provider.delete(container, "moved").await.expect("delete moved");
    provider.delete_container(container).await.expect("delete container");
    assert!(!provider.container_exists(container).await.expect("container gone"));
}

/// Asserts `Publish::send` delivers payload and headers to the topic.
///
/// # Panics
///
/// Panics if publishing fails.
pub async fn publish_send<P: Publish>(provider: &P) {
    let mut msg = Message::new(b"conformance-event");
    msg.headers.insert("x-trace".to_owned(), "1".to_owned());
    provider.send("conformance.events", &msg).await.expect("publish send");
}

/// Asserts `Config::get` returns a seeded value and errors on an absent key.
///
/// Expects `"CONFORMANCE_KEY"` to be seeded with `"conformance-val"`.
///
/// # Panics
///
/// Panics if the known key fails or the absent key succeeds.
pub async fn config_get<P: Config>(provider: &P) {
    let val = provider.get("CONFORMANCE_KEY").await.expect("known config key");
    assert_eq!(val, "conformance-val");
    provider.get("CONFORMANCE_ABSENT_KEY").await.expect_err("absent config key must fail");
}

/// Asserts `Identity::token` and `Identity::access_token` return a valid token.
///
/// # Panics
///
/// Panics if token acquisition fails or returns an empty token.
pub async fn identity_token<P: Identity>(provider: &P) {
    let scoped = provider.token("default", &["read".to_owned()]).await.expect("identity token");
    assert_ne!(scoped.token, "");
    assert!(scoped.expires_in > 0);

    let plain = provider.access_token("default".to_owned()).await.expect("access_token");
    assert_eq!(plain, scoped.token);
}

/// Asserts `Vault` secret CRUD, `exists`, and `list_ids` over the locker.
///
/// # Panics
///
/// Panics if any vault operation fails or returns unexpected values.
pub async fn vault_locker<P: Vault>(provider: &P) {
    let key = "conformance-secret";
    provider.delete(key).await.expect("cleanup secret");
    assert_eq!(provider.get(key).await.expect("missing secret is None"), None);
    assert!(!provider.exists(key).await.expect("missing secret does not exist"));

    provider.set(key, b"s1").await.expect("set secret");
    assert_eq!(provider.get(key).await.expect("get s1"), Some(b"s1".to_vec()));
    provider.set(key, b"s2").await.expect("overwrite secret");
    assert_eq!(provider.get(key).await.expect("get s2"), Some(b"s2".to_vec()));
    assert!(provider.exists(key).await.expect("secret exists"));
    assert!(provider.list_ids().await.expect("list_ids").iter().any(|id| id == key));

    provider.delete(key).await.expect("delete secret");
    assert!(!provider.exists(key).await.expect("secret deleted"));
}

/// Asserts `HttpRequest::fetch` sends a request and returns the response body.
///
/// Expects `url` (e.g. `http://127.0.0.1:port`) to answer `GET /conformance`
/// with `"conformance-ok"`.
///
/// # Panics
///
/// Panics if the HTTP request fails or the response does not match.
pub async fn http_fetch<P: HttpRequest>(provider: &P, url: &str) {
    let req = Request::get(format!("{url}/conformance"))
        .header("x-probe", "1")
        .body(Full::new(Bytes::new()))
        .expect("build request");
    let resp = provider.fetch(req).await.expect("http fetch");
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.into_body().as_ref(), b"conformance-ok");
}

/// Asserts `Broadcast::send` dispatches an event payload.
///
/// # Panics
///
/// Panics if broadcasting fails.
pub async fn broadcast_send<P: Broadcast>(provider: &P) {
    provider.send("conformance", b"ping", None).await.expect("broadcast send");
}

/// Asserts `Model::complete` and `Model::complete_with` with a `check` callback.
///
/// # Panics
///
/// Panics if completion or check handling fails.
pub async fn model_complete<P: Model>(provider: &P) {
    let request = ModelRequest::builder()
        .messages(vec![ModelMessage {
            role: Role::User,
            content: "conformance".to_owned(),
        }])
        .build();
    let reply = provider.complete(request).await.expect("model complete");
    assert_eq!(reply.answer, "conformance");

    let checked_req = ModelRequest::builder()
        .messages(vec![ModelMessage {
            role: Role::User,
            content: "checked".to_owned(),
        }])
        .check(true)
        .build();
    let checked = provider
        .complete_with(checked_req, |call| async move {
            assert_eq!(call.name, model::CHECK_TOOL);
            Ok(String::new())
        })
        .await
        .expect("model complete_with check");
    assert_eq!(checked.answer, "checked");
}

/// Asserts `Vcs` refuses an unmounted path and resolves a revision on `.`.
///
/// # Panics
///
/// Panics if mount confinement or revision resolution fails.
pub async fn vcs_flow<P: Vcs>(provider: &P) {
    let unmounted = provider.head("/unmounted").await.expect_err("unmounted path refused");
    assert!(matches!(unmounted, vcs::Error::Other(_)), "unexpected error: {unmounted:?}");

    let resolved = provider.resolve(".", "main").await.expect("resolve main");
    assert_eq!(resolved, "sha:main");
}

async fn sleep(duration: Duration) {
    cfg_select! {
        all(target_arch = "wasm32", target_os = "wasi") => {
            let nanos = u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
            omnia_sdk::wasip3::clocks::monotonic_clock::wait_for(nanos).await;
        }
        _ => {
            tokio::time::sleep(duration).await;
        }
    }
}

/// Asserts that the invoking test module defines a test for every conformance scenario.
///
/// Covers the selected capabilities, or all 12 portable capabilities when invoked with no arguments.
///
/// # Examples
///
/// An unpaired scenario fails to compile:
///
/// ```compile_fail
/// omnia_test::conformance!(state);
/// ```
#[macro_export]
macro_rules! conformance {
    () => {
        $crate::conformance!(
            state,
            table,
            docstore,
            blobstore,
            publish,
            config,
            identity,
            vault,
            http,
            broadcast,
            model,
            vcs
        );
    };
    ($($cap:ident),+ $(,)?) => {
        $(
            $crate::__conformance_cap!($cap);
        )+
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __conformance_cap {
    (state) => {
        #[expect(unused_imports, reason = "asserts the conformance test exists")]
        use self::{
            state_cas as _, state_get_set_delete as _, state_increment as _, state_ttl_expires as _,
        };
    };
    (table) => {
        #[expect(unused_imports, reason = "asserts the conformance test exists")]
        use self::table_query_exec as _;
    };
    (docstore) => {
        #[expect(unused_imports, reason = "asserts the conformance test exists")]
        use self::docstore_crud_query as _;
    };
    (blobstore) => {
        #[expect(unused_imports, reason = "asserts the conformance test exists")]
        use self::blobstore_objects as _;
    };
    (publish) => {
        #[expect(unused_imports, reason = "asserts the conformance test exists")]
        use self::publish_send as _;
    };
    (config) => {
        #[expect(unused_imports, reason = "asserts the conformance test exists")]
        use self::config_get as _;
    };
    (identity) => {
        #[expect(unused_imports, reason = "asserts the conformance test exists")]
        use self::identity_token as _;
    };
    (vault) => {
        #[expect(unused_imports, reason = "asserts the conformance test exists")]
        use self::vault_locker as _;
    };
    (http) => {
        #[expect(unused_imports, reason = "asserts the conformance test exists")]
        use self::http_fetch as _;
    };
    (broadcast) => {
        #[expect(unused_imports, reason = "asserts the conformance test exists")]
        use self::broadcast_send as _;
    };
    (model) => {
        #[expect(unused_imports, reason = "asserts the conformance test exists")]
        use self::model_complete as _;
    };
    (vcs) => {
        #[expect(unused_imports, reason = "asserts the conformance test exists")]
        use self::vcs_flow as _;
    };
}
