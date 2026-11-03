//! Conformance suite for `Direct<*Default>` and `RouterHandler`.

#![cfg(not(target_arch = "wasm32"))]

use axum::Router;
use axum::routing::get;
use omnia_backend::http::ConnectOptions as HttpOptions;
use omnia_backend::messaging::{
    Handler as _, Message as BackendMessage, Metadata as BackendMetadata,
};
use omnia_backend::sql::ConnectOptions as SqlOptions;
use omnia_backend::{
    BlobStoreDefault, ConfigDefault, Connect as _, DocStoreDefault, HttpDefault, IdentityStub,
    KeyValueDefault, MessagingDefault, ModelDefault, MountRegistry, NoOptions, ResolvedPreopen,
    SqlDefault, VaultDefault, WebSocketDefault,
};
use omnia_native::{Direct, RouterHandler};
use omnia_sdk::api::messaging::{Router as MessagingRouter, consume};
use omnia_sdk::api::{Client, Context};
use omnia_sdk::{Error, StateStore as _};
use omnia_test::backend::ScriptedVcs;
use omnia_test::conformance;
use tokio::net::TcpListener;

omnia_test::conformance!();

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
    vcs: Direct<ScriptedVcs>,
}

async fn native_provider(mounts: MountRegistry, vcs: ScriptedVcs) -> NativeDefaults {
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
        model: Direct::with_mounts(ModelDefault, mounts.clone()),
        vcs: Direct::with_mounts(vcs, mounts),
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
    let provider = native_provider(MountRegistry::new(), ScriptedVcs::default()).await;
    conformance::state_get_set_delete(&provider).await;
}

#[tokio::test]
async fn state_ttl_expires() {
    let provider = native_provider(MountRegistry::new(), ScriptedVcs::default()).await;
    conformance::state_ttl_expires(&provider).await;
}

#[tokio::test]
async fn state_cas() {
    let provider = native_provider(MountRegistry::new(), ScriptedVcs::default()).await;
    conformance::state_cas(&provider).await;
}

#[tokio::test]
async fn state_increment() {
    let provider = native_provider(MountRegistry::new(), ScriptedVcs::default()).await;
    conformance::state_increment(&provider).await;
}

#[tokio::test]
async fn table_query_exec() {
    let provider = native_provider(MountRegistry::new(), ScriptedVcs::default()).await;
    conformance::table_query_exec(&provider).await;
}

#[tokio::test]
async fn docstore_crud_query() {
    let provider = native_provider(MountRegistry::new(), ScriptedVcs::default()).await;
    conformance::docstore_crud_query(&provider).await;
}

#[tokio::test]
async fn blobstore_objects() {
    let provider = native_provider(MountRegistry::new(), ScriptedVcs::default()).await;
    conformance::blobstore_objects(&provider).await;
}

#[tokio::test]
async fn publish_send() {
    let provider = native_provider(MountRegistry::new(), ScriptedVcs::default()).await;
    conformance::publish_send(&provider).await;
}

#[tokio::test]
async fn config_get() {
    let provider = native_provider(MountRegistry::new(), ScriptedVcs::default()).await;
    conformance::config_get(&provider).await;
}

#[tokio::test]
async fn identity_token() {
    let provider = native_provider(MountRegistry::new(), ScriptedVcs::default()).await;
    conformance::identity_token(&provider).await;
}

#[tokio::test]
async fn vault_locker() {
    let provider = native_provider(MountRegistry::new(), ScriptedVcs::default()).await;
    conformance::vault_locker(&provider).await;
}

#[tokio::test]
async fn http_fetch() {
    let url = serve_origin().await;
    let provider = native_provider(MountRegistry::new(), ScriptedVcs::default()).await;
    conformance::http_fetch(&provider, &url).await;
}

#[tokio::test]
async fn broadcast_send() {
    let provider = native_provider(MountRegistry::new(), ScriptedVcs::default()).await;
    conformance::broadcast_send(&provider).await;
}

#[tokio::test]
async fn model_complete() {
    let provider = native_provider(MountRegistry::new(), ScriptedVcs::default()).await;
    conformance::model_complete(&provider).await;
}

#[tokio::test]
async fn vcs_flow() {
    let scratch = tempfile::tempdir().expect("tempdir");
    let mounts = MountRegistry::open(vec![ResolvedPreopen::new(
        ".".to_owned(),
        scratch.path().to_path_buf(),
        false,
    )])
    .expect("open mount registry");
    let vcs = ScriptedVcs::resolving(["sha:main"]);
    let provider = native_provider(mounts, vcs.clone()).await;
    conformance::vcs_flow(&provider).await;
    vcs.assert_exhausted();
}

#[tokio::test]
async fn router_handler_unrouted_topic() {
    let provider = native_provider(MountRegistry::new(), ScriptedVcs::default()).await;
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
