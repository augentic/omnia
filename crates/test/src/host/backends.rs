//! The store's backend bundle: every `wasi-*` in-memory default plus a
//! swappable model backend.

use omnia::{Backend as _, HostCtx, HttpCtx, NoOptions, Provides};
use omnia_wasi_blobstore::{BlobstoreDefault, WasiBlobstore};
use omnia_wasi_config::{ConfigDefault, WasiConfig};
use omnia_wasi_docstore::{DocStoreDefault, WasiDocStore};
use omnia_wasi_http::HttpDefault;
use omnia_wasi_identity::{IdentityStub, WasiIdentity};
use omnia_wasi_keyvalue::{KeyValueDefault, WasiKeyValue};
use omnia_wasi_messaging::{MessagingDefault, WasiMessaging};
use omnia_wasi_model::{ModelDefault, WasiModel, WasiModelCtx};
use omnia_wasi_otel::{OtelDefault, WasiOtel};
use omnia_wasi_sql::{SqlDefault, WasiSql};
use omnia_wasi_vault::{VaultDefault, WasiVault};
use omnia_wasi_websocket::{ConnectOptions as WebSocketOptions, WasiWebSocket, WebSocketDefault};

use super::ScriptedModel;

/// The keyvalue bucket `omnia_guest`'s wasm32 `StateStore` opens.
pub const STATE_BUCKET: &str = "cache";

/// Every host's default backend as one bundle, with the model backend
/// swappable for a [`ScriptedModel`].
///
/// Each field is a shared handle: a clone the scenario keeps reads state
/// back after the run. The bundle implements `Provides` for every host, so
/// a deployment links any subset of them. Identity is the credential-free
/// `IdentityStub`, the one default that cannot connect without an identity
/// provider in the environment.
#[derive(Clone, Debug)]
pub struct Backends<M = ModelDefault> {
    /// In-memory `wasi:blobstore`.
    pub blobstore: BlobstoreDefault,
    /// Environment-backed `wasi:config`.
    pub config: ConfigDefault,
    /// In-memory `wasi:docstore`.
    pub docstore: DocStoreDefault,
    /// Outbound `wasi:http`.
    pub http: HttpDefault,
    /// Fixed-token `wasi:identity`.
    pub identity: IdentityStub,
    /// In-memory `wasi:keyvalue`.
    pub keyvalue: KeyValueDefault,
    /// In-process `wasi:messaging`.
    pub messaging: MessagingDefault,
    /// The `omnia:model` backend.
    pub model: M,
    /// No-op `wasi:otel`.
    pub otel: OtelDefault,
    /// In-memory `SQLite` `wasi:sql`.
    pub sql: SqlDefault,
    /// In-memory `wasi:vault`.
    pub vault: VaultDefault,
    /// In-process `omnia:websocket` on an ephemeral loopback port.
    pub websocket: WebSocketDefault,
}

impl Backends {
    /// Every default, freshly connected.
    ///
    /// # Panics
    ///
    /// Panics if a default cannot be constructed.
    pub async fn defaults() -> Self {
        Self {
            blobstore: BlobstoreDefault::connect_with(NoOptions)
                .await
                .expect("in-memory blobstore"),
            config: ConfigDefault::connect_with(NoOptions).await.expect("environment config"),
            docstore: DocStoreDefault::connect_with(NoOptions).await.expect("in-memory docstore"),
            http: HttpDefault::connect().await.expect("outbound http client"),
            identity: IdentityStub,
            keyvalue: KeyValueDefault::connect_with(NoOptions).await.expect("in-memory keyvalue"),
            messaging: MessagingDefault::connect_with(NoOptions)
                .await
                .expect("in-process messaging"),
            model: ModelDefault,
            otel: OtelDefault,
            sql: SqlDefault::connect().await.expect("in-memory sqlite"),
            vault: VaultDefault::connect_with(NoOptions).await.expect("in-memory vault"),
            websocket: WebSocketDefault::connect_with(WebSocketOptions {
                socket_addr: "127.0.0.1:0".to_owned(),
            })
            .await
            .expect("in-process websocket"),
        }
    }
}

impl<M> Backends<M> {
    /// The same bundle answering completions with `model`.
    #[must_use]
    pub fn model(self, model: ScriptedModel) -> Backends<ScriptedModel> {
        Backends {
            blobstore: self.blobstore,
            config: self.config,
            docstore: self.docstore,
            http: self.http,
            identity: self.identity,
            keyvalue: self.keyvalue,
            messaging: self.messaging,
            model,
            otel: self.otel,
            sql: self.sql,
            vault: self.vault,
            websocket: self.websocket,
        }
    }
}

macro_rules! provides {
    ($($host:ty => $field:ident),* $(,)?) => {
        $(
            impl<M: WasiModelCtx + Clone> Provides<$host> for Backends<M> {
                fn borrow(&mut self) -> <$host as HostCtx>::Borrow<'_> {
                    &mut self.$field
                }
            }
        )*
    };
}

provides! {
    WasiBlobstore => blobstore,
    WasiConfig => config,
    WasiDocStore => docstore,
    HttpCtx => http,
    WasiIdentity => identity,
    WasiKeyValue => keyvalue,
    WasiMessaging => messaging,
    WasiModel => model,
    WasiOtel => otel,
    WasiSql => sql,
    WasiVault => vault,
    WasiWebSocket => websocket,
}
