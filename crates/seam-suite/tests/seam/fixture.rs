//! Shared suite fixtures: one tokio runtime for every test and one
//! conformance runtime built once per process.
//!
//! Tests are ordinary `#[test]` functions that `RT.block_on` their async
//! bodies, so backends that spawn tasks (the WebSocket server, the messaging
//! broker) live on a runtime that outlasts any single test.

use std::net::TcpListener;
use std::sync::{Arc, LazyLock, Mutex};

use anyhow::{Context as _, Result};
use futures::FutureExt as _;
use omnia::wasmtime_wasi::ResourceTable;
use omnia::{Backend, FutureResult, HasHttp, Runtime};
use omnia_testkit::single_guest;
use omnia_wasi_blobstore::{BlobstoreDefault, HasBlobstore, WasiBlobstore, WasiBlobstoreCtx};
use omnia_wasi_config::{ConfigDefault, HasConfig, WasiConfig, WasiConfigCtx};
use omnia_wasi_docstore::{DocStoreDefault, HasDocStore, WasiDocStore, WasiDocStoreCtx};
use omnia_wasi_http::{HttpDefault, WasiHttp, WasiHttpCtxView};
use omnia_wasi_identity::{HasIdentity, IdentityStub, WasiIdentity, WasiIdentityCtx};
use omnia_wasi_keyvalue::{HasKeyValue, KeyValueDefault, WasiKeyValue, WasiKeyValueCtx};
use omnia_wasi_messaging::{HasMessaging, MessagingDefault, WasiMessaging, WasiMessagingCtx};
use omnia_wasi_otel::{HasOtel, WasiOtel, WasiOtelCtx};
use omnia_wasi_sql::{HasSql, SqlDefault, WasiSql, WasiSqlCtx};
use omnia_wasi_vault::{HasVault, VaultDefault, WasiVault, WasiVaultCtx};
use omnia_wasi_websocket::{
    ConnectOptions as WsConnectOptions, HasWebSocket, WasiWebSocket, WasiWebSocketCtx,
    WebSocketDefault,
};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use tokio::sync::OnceCell;

/// The one tokio runtime every test in the suite runs on.
pub static RT: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("building the shared tokio runtime")
});

/// Spans and metrics observed at the host export boundary.
#[derive(Debug, Default)]
pub struct Captured {
    pub spans: usize,
    pub metrics: usize,
}

/// A `wasi:otel` backend that counts exported spans and metrics instead of
/// discarding them, shared across bundle clones so tests read totals after
/// guests run.
#[derive(Debug, Clone, Default)]
pub struct CapturingOtel {
    pub captured: Arc<Mutex<Captured>>,
}

impl WasiOtelCtx for CapturingOtel {
    fn export_traces(&self, request: ExportTraceServiceRequest) -> FutureResult<()> {
        let spans = request
            .resource_spans
            .iter()
            .flat_map(|rs| &rs.scope_spans)
            .map(|ss| ss.spans.len())
            .sum::<usize>();
        let captured = Arc::clone(&self.captured);
        async move {
            captured.lock().expect("otel capture lock").spans += spans;
            Ok(())
        }
        .boxed()
    }

    fn export_metrics(&self, request: ExportMetricsServiceRequest) -> FutureResult<()> {
        let metrics = request
            .resource_metrics
            .iter()
            .flat_map(|rm| &rm.scope_metrics)
            .map(|sm| sm.metrics.len())
            .sum::<usize>();
        let captured = Arc::clone(&self.captured);
        async move {
            captured.lock().expect("otel capture lock").metrics += metrics;
            Ok(())
        }
        .boxed()
    }
}

/// The all-interface backend bundle behind the conformance guest.
#[derive(Clone)]
pub struct Bundle {
    http: HttpDefault,
    otel: CapturingOtel,
    keyvalue: KeyValueDefault,
    blobstore: BlobstoreDefault,
    config: ConfigDefault,
    identity: IdentityStub,
    sql: SqlDefault,
    vault: VaultDefault,
    docstore: DocStoreDefault,
    messaging: MessagingDefault,
    websocket: WebSocketDefault,
}

impl HasHttp for Bundle {
    fn http_view<'a>(&'a mut self, table: &'a mut ResourceTable) -> WasiHttpCtxView<'a> {
        self.http.as_view(table)
    }
}

impl HasOtel for Bundle {
    fn otel_ctx(&mut self) -> &mut dyn WasiOtelCtx {
        &mut self.otel
    }
}

impl HasKeyValue for Bundle {
    fn keyvalue_ctx(&mut self) -> &mut dyn WasiKeyValueCtx {
        &mut self.keyvalue
    }
}

impl HasBlobstore for Bundle {
    fn blobstore_ctx(&mut self) -> &mut dyn WasiBlobstoreCtx {
        &mut self.blobstore
    }
}

// `wasi:config` is read-only, so its accessor borrows `&self`.
impl HasConfig for Bundle {
    fn config_ctx(&self) -> &dyn WasiConfigCtx {
        &self.config
    }
}

impl HasIdentity for Bundle {
    fn identity_ctx(&mut self) -> &mut dyn WasiIdentityCtx {
        &mut self.identity
    }
}

impl HasSql for Bundle {
    fn sql_ctx(&mut self) -> &mut dyn WasiSqlCtx {
        &mut self.sql
    }
}

impl HasVault for Bundle {
    fn vault_ctx(&mut self) -> &mut dyn WasiVaultCtx {
        &mut self.vault
    }
}

impl HasDocStore for Bundle {
    fn docstore_ctx(&mut self) -> &mut dyn WasiDocStoreCtx {
        &mut self.docstore
    }
}

impl HasMessaging for Bundle {
    fn messaging_ctx(&mut self) -> &mut dyn WasiMessagingCtx {
        &mut self.messaging
    }
}

impl HasWebSocket for Bundle {
    fn websocket_ctx(&mut self) -> &mut dyn WasiWebSocketCtx {
        &mut self.websocket
    }
}

/// The shared conformance runtime plus probe handles onto every shared
/// backend (clones share state, so a probe observes the guest's effects).
pub struct Conformance {
    pub runtime: Runtime<Bundle>,
    pub keyvalue: KeyValueDefault,
    pub blobstore: BlobstoreDefault,
    pub sql: SqlDefault,
    pub vault: VaultDefault,
    pub docstore: DocStoreDefault,
    pub messaging: MessagingDefault,
    pub otel: CapturingOtel,
    /// The port the default WebSocket backend's server listens on.
    pub websocket_port: u16,
}

/// The conformance fixture, built once per suite process.
pub async fn conformance() -> Result<&'static Conformance> {
    static CELL: OnceCell<Conformance> = OnceCell::const_new();
    CELL.get_or_try_init(build).await
}

async fn build() -> Result<Conformance> {
    let websocket_port = free_port()?;
    let bundle = Bundle {
        http: HttpDefault::connect().await.context("connecting http")?,
        otel: CapturingOtel::default(),
        keyvalue: KeyValueDefault::connect().await.context("connecting keyvalue")?,
        blobstore: BlobstoreDefault::connect().await.context("connecting blobstore")?,
        config: ConfigDefault::connect().await.context("connecting config")?,
        identity: IdentityStub::connect().await.context("connecting identity stub")?,
        sql: SqlDefault::connect().await.context("connecting sql")?,
        vault: VaultDefault::connect().await.context("connecting vault")?,
        docstore: DocStoreDefault::connect().await.context("connecting docstore")?,
        messaging: <MessagingDefault as Backend>::connect()
            .await
            .context("connecting messaging")?,
        websocket: WebSocketDefault::connect_with(WsConnectOptions {
            socket_addr: format!("127.0.0.1:{websocket_port}"),
        })
        .await
        .context("connecting websocket")?,
    };

    let keyvalue = bundle.keyvalue.clone();
    let blobstore = bundle.blobstore.clone();
    let sql = bundle.sql.clone();
    let vault = bundle.vault.clone();
    let docstore = bundle.docstore.clone();
    let messaging = bundle.messaging.clone();
    let otel = bundle.otel.clone();

    let runtime = single_guest("conformance_wasm.wasm", bundle)
        .await?
        .host::<WasiHttp>()?
        .host::<WasiOtel>()?
        .host::<WasiKeyValue>()?
        .host::<WasiBlobstore>()?
        .host::<WasiConfig>()?
        .host::<WasiIdentity>()?
        .host::<WasiSql>()?
        .host::<WasiVault>()?
        .host::<WasiDocStore>()?
        .host::<WasiMessaging>()?
        .host::<WasiWebSocket>()?
        .into_runtime()?;

    // The websocket trigger loop forwards inbound peer messages to the guest
    // handler; spawn it the way a deployment's `run` would.
    let trigger = runtime.clone();
    tokio::spawn(async move {
        if let Err(e) = omnia::Server::run(&WasiWebSocket, &trigger).await {
            eprintln!("websocket trigger loop failed: {e}");
        }
    });

    Ok(Conformance {
        runtime,
        keyvalue,
        blobstore,
        sql,
        vault,
        docstore,
        messaging,
        otel,
        websocket_port,
    })
}

/// Reserve a free localhost port (for the WebSocket backend's server).
fn free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").context("reserving a port")?;
    Ok(listener.local_addr()?.port())
}

/// A process-unique suffix for keys/ids so concurrent tests sharing the
/// conformance backends never collide.
pub fn unique(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!("{prefix}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))
}
