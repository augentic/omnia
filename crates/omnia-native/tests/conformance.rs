//! Conformance suite for `Direct<*Default>`, `WasiProvider` (through
//! `Deployment`), and byte-identity interoperability on a shared
//! `KeyValueDefault`.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use futures::FutureExt as _;
use omnia_backend::http::ConnectOptions as HttpOptions;
use omnia_backend::messaging::{
    Handler as _, Message as BackendMessage, Metadata as BackendMetadata,
};
use omnia_backend::sql::ConnectOptions as SqlOptions;
use omnia_backend::vcs::{Change, CloneOptions, Entry, Merged, Place, Rule, VcsClient};
use omnia_backend::{
    BlobStoreDefault, ConfigDefault, Connect as _, DocStoreDefault, FutureResult, HttpDefault,
    IdentityStub, KeyValueDefault, MessagingDefault, ModelDefault, MountRegistry, NoOptions,
    OtelDefault, ResolvedPreopen, SqlDefault, VaultDefault, WebSocketDefault,
};
use omnia_native::{Direct, RouterHandler};
use omnia_sdk::api::messaging::{Router as MessagingRouter, consume};
use omnia_sdk::api::{Client, Context};
use omnia_sdk::{Error, StateStore as _};
use omnia_test::conformance;
use omnia_test::host::{Backends, Deployment, scratch};
use omnia_wasi_blobstore::WasiBlobStore;
use omnia_wasi_config::WasiConfig;
use omnia_wasi_docstore::WasiDocStore;
use omnia_wasi_http::WasiHttp;
use omnia_wasi_identity::WasiIdentity;
use omnia_wasi_keyvalue::WasiKeyValue;
use omnia_wasi_messaging::WasiMessaging;
use omnia_wasi_model::WasiModel;
use omnia_wasi_otel::WasiOtel;
use omnia_wasi_sql::WasiSql;
use omnia_wasi_vault::WasiVault;
use omnia_wasi_vcs::WasiVcs;
use omnia_wasi_websocket::WasiWebSocket;
use tokio::net::TcpListener;

omnia_test::conformance!();
test_programs::foreach_conformance!();

#[derive(Clone, Debug, Default)]
struct StubVcsClient;

impl VcsClient for StubVcsClient {
    fn resolve(&self, _repo: Place, revision: String) -> FutureResult<String> {
        async move { Ok(format!("sha:{revision}")) }.boxed()
    }

    fn descends(&self, _repo: Place, _ancestor: String, _descendant: String) -> FutureResult<bool> {
        async { Ok(true) }.boxed()
    }

    fn head(&self, _at: Place) -> FutureResult<String> {
        async { Ok("sha:head".to_owned()) }.boxed()
    }

    fn commit(&self, _at: Place, _message: String) -> FutureResult<Option<String>> {
        async { Ok(None) }.boxed()
    }

    fn merge(
        &self, _at: Place, _revision: String, _message: String, _policy: Vec<Rule>,
    ) -> FutureResult<Merged> {
        async { Ok(Merged::default()) }.boxed()
    }

    fn log(&self, _repo: Place, _revision: String, _base: String) -> FutureResult<Vec<Entry>> {
        async { Ok(vec![]) }.boxed()
    }

    fn init(&self, _at: Place) -> FutureResult<()> {
        async { Ok(()) }.boxed()
    }

    fn add(&self, _repo: Place, _at: Place, _revision: String) -> FutureResult<()> {
        async { Ok(()) }.boxed()
    }

    fn remove(&self, _at: Place) -> FutureResult<()> {
        async { Ok(()) }.boxed()
    }

    fn pending(&self, _at: Place) -> FutureResult<Vec<Change>> {
        async { Ok(vec![]) }.boxed()
    }

    fn clone_repo(&self, _url: String, _at: Place, _options: CloneOptions) -> FutureResult<()> {
        async { Ok(()) }.boxed()
    }

    fn fetch(&self, _repo: Place, _remote: String) -> FutureResult<()> {
        async { Ok(()) }.boxed()
    }

    fn label(&self, _repo: Place, _name: String, _revision: String) -> FutureResult<()> {
        async { Ok(()) }.boxed()
    }

    fn labelled(&self, _repo: Place, _name: String) -> FutureResult<String> {
        async { Ok(String::new()) }.boxed()
    }

    fn fetched(&self, _repo: Place, _remote: String, _name: String) -> FutureResult<String> {
        async { Ok(String::new()) }.boxed()
    }

    fn push(&self, _repo: Place, _remote: String, _label: String) -> FutureResult<()> {
        async { Ok(()) }.boxed()
    }
}

#[derive(Clone, Debug, omnia_sdk::Provider)]
struct NativeDefaults {
    #[provider(Config)]
    config: Direct<ConfigDefault>,
    #[provider(HttpRequest)]
    http: Direct<HttpDefault>,
    #[provider(Identity)]
    identity: Direct<IdentityStub>,
    #[provider(Publish)]
    publish: Direct<MessagingDefault>,
    #[provider(Broadcast)]
    broadcast: Direct<WebSocketDefault>,
    #[provider(StateStore)]
    state: Direct<KeyValueDefault>,
    #[provider(BlobStore)]
    blob: Direct<BlobStoreDefault>,
    #[provider(DocumentStore)]
    docs: Direct<DocStoreDefault>,
    #[provider(TableStore)]
    tables: Direct<SqlDefault>,
    #[provider(Vault)]
    vault: Direct<VaultDefault>,
    #[provider(Model)]
    model: Direct<ModelDefault>,
    #[provider(Vcs)]
    vcs: Direct<StubVcsClient>,
}

async fn native_provider(mounts: Arc<MountRegistry>) -> NativeDefaults {
    NativeDefaults {
        config: Direct::new(ConfigDefault::from_iter([("CONFORMANCE_KEY", "conformance-val")])),
        http: Direct::new(
            HttpDefault::connect_with(HttpOptions { connect_timeout: 5 }).await.expect("http"),
        ),
        identity: Direct::new(IdentityStub),
        publish: Direct::new(MessagingDefault::connect_with(NoOptions).await.expect("messaging")),
        broadcast: Direct::new(WebSocketDefault::new()),
        state: Direct::new(KeyValueDefault::connect_with(NoOptions).await.expect("keyvalue")),
        blob: Direct::new(BlobStoreDefault::connect_with(NoOptions).await.expect("blobstore")),
        docs: Direct::new(DocStoreDefault::connect_with(NoOptions).await.expect("docstore")),
        tables: Direct::new(
            SqlDefault::connect_with(SqlOptions {
                database: ":memory:".to_owned(),
            })
            .await
            .expect("sqlite"),
        ),
        vault: Direct::new(VaultDefault::connect_with(NoOptions).await.expect("vault")),
        model: Direct::with_mounts(ModelDefault, Arc::clone(&mounts)),
        vcs: Direct::with_mounts(StubVcsClient, mounts),
    }
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

#[tokio::test]
async fn state_get_set_delete() {
    let provider = native_provider(Arc::default()).await;
    conformance::state_get_set_delete(&provider).await;
}

#[tokio::test]
async fn state_ttl_expires() {
    let provider = native_provider(Arc::default()).await;
    conformance::state_ttl_expires(&provider).await;
}

#[tokio::test]
async fn state_cas() {
    let provider = native_provider(Arc::default()).await;
    conformance::state_cas(&provider).await;
}

#[tokio::test]
async fn state_increment() {
    let provider = native_provider(Arc::default()).await;
    conformance::state_increment(&provider).await;
}

#[tokio::test]
async fn table_query_exec() {
    let provider = native_provider(Arc::default()).await;
    conformance::table_query_exec(&provider).await;
}

#[tokio::test]
async fn docstore_crud_query() {
    let provider = native_provider(Arc::default()).await;
    conformance::docstore_crud_query(&provider).await;
}

#[tokio::test]
async fn blobstore_objects() {
    let provider = native_provider(Arc::default()).await;
    conformance::blobstore_objects(&provider).await;
}

#[tokio::test]
async fn publish_send() {
    let provider = native_provider(Arc::default()).await;
    conformance::publish_send(&provider).await;
}

#[tokio::test]
async fn config_get() {
    let provider = native_provider(Arc::default()).await;
    conformance::config_get(&provider).await;
}

#[tokio::test]
async fn identity_token() {
    let provider = native_provider(Arc::default()).await;
    conformance::identity_token(&provider).await;
}

#[tokio::test]
async fn vault_locker() {
    let provider = native_provider(Arc::default()).await;
    conformance::vault_locker(&provider).await;
}

#[tokio::test]
async fn http_fetch() {
    let url = serve_origin().await;
    let provider = native_provider(Arc::default()).await;
    conformance::http_fetch(&provider, &url).await;
}

#[tokio::test]
async fn broadcast_send() {
    let provider = native_provider(Arc::default()).await;
    conformance::broadcast_send(&provider).await;
}

#[tokio::test]
async fn model_complete() {
    let provider = native_provider(Arc::default()).await;
    conformance::model_complete(&provider).await;
}

#[tokio::test]
async fn vcs_flow() {
    let scratch = scratch();
    let mounts = Arc::new(
        MountRegistry::open(vec![ResolvedPreopen::new(
            ".".to_owned(),
            scratch.path().to_path_buf(),
            false,
        )])
        .expect("open mount registry"),
    );
    let provider = native_provider(mounts).await;
    conformance::vcs_flow(&provider).await;
}

type WasiBackends = Backends<
    ModelDefault,
    KeyValueDefault,
    BlobStoreDefault,
    DocStoreDefault,
    SqlDefault,
    VaultDefault,
    MessagingDefault,
    IdentityStub,
    OtelDefault,
    StubVcsClient,
>;

// Runs the same conformance scenarios inside a real WASI guest over `WasiProvider`,
// and verifies byte-for-byte interoperability with `Direct<KeyValueDefault>` on
// one shared `KeyValueDefault` instance.
#[tokio::test]
async fn conformance_portable() {
    let url = serve_origin().await;
    let scratch = scratch();

    let backends = Backends::defaults()
        .await
        .config([
            ("CONFORMANCE_KEY", "conformance-val".to_owned()),
            ("CONFORMANCE_ORIGIN", url),
            ("CONFORMANCE_SHARED_BUCKET", "1".to_owned()),
        ])
        .vcs(StubVcsClient);

    let direct_kv = Direct::new(backends.keyvalue.clone());
    direct_kv.set("shared/from-direct", b"written-by-direct", None).await.expect("direct set");
    assert_eq!(direct_kv.increment("shared/counter", 10).await.expect("direct increment"), 10);

    let status = Deployment::new()
        .guest("conformance", test_programs::CONFORMANCE_PORTABLE)
        .mount(scratch.mount(false))
        .run(backends.clone(), |deployment| {
            deployment.host::<WasiBlobStore, WasiBackends>()?;
            deployment.host::<WasiConfig, WasiBackends>()?;
            deployment.host::<WasiDocStore, WasiBackends>()?;
            deployment.host::<WasiHttp, WasiBackends>()?;
            deployment.host::<WasiIdentity, WasiBackends>()?;
            deployment.host::<WasiKeyValue, WasiBackends>()?;
            deployment.host::<WasiMessaging, WasiBackends>()?;
            deployment.host::<WasiModel, WasiBackends>()?;
            deployment.host::<WasiOtel, WasiBackends>()?;
            deployment.host::<WasiSql, WasiBackends>()?;
            deployment.host::<WasiVault, WasiBackends>()?;
            deployment.host::<WasiVcs, WasiBackends>()?;
            deployment.host::<WasiWebSocket, WasiBackends>()?;
            Ok(())
        })
        .await
        .expect("conformance guest runs");
    assert_eq!(status, omnia::ExitStatus::SUCCESS);

    assert_eq!(
        direct_kv.get("shared/from-wasi").await.expect("direct reads wasi key"),
        Some(b"written-by-wasi".to_vec())
    );
    assert_eq!(direct_kv.increment("shared/counter", 0).await.expect("direct reads counter"), 15);
}

#[tokio::test]
async fn router_handler_dispatches_and_ignores_unrouted() {
    let provider = native_provider(Arc::default()).await;
    let router = MessagingRouter::new(Client::new("native", provider.clone())).route(
        "orders.created",
        consume(|order: String, cx: Context<NativeDefaults>| async move {
            cx.provider().set("orders/last", order.as_bytes(), None).await.map_err(Error::from)?;
            Ok::<_, Error>(())
        }),
    );
    let handler = RouterHandler::new(router);

    let mut msg = BackendMessage::new(br#""ord-1""#.to_vec());
    msg.topic = "orders.created".to_owned();
    msg.metadata = Some(BackendMetadata::new());
    handler.handle(msg).await.expect("routed topic handled");
    assert_eq!(provider.get("orders/last").await.expect("get"), Some(b"ord-1".to_vec()));

    let mut unrouted = BackendMessage::new(b"{}".to_vec());
    unrouted.topic = "orders.unknown".to_owned();
    handler.handle(unrouted).await.expect("unrouted topic dropped cleanly");
}
