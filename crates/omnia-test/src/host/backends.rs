//! The store's backend bundle: every `wasi-*` in-memory default plus a
//! swappable model backend.

use std::sync::Arc;

use omnia::{Backend as _, HostCtx, HttpCtx, NoOptions, Provides};
use omnia_wasi_blobstore::{BlobstoreDefault, WasiBlobstore};
use omnia_wasi_config::wasmtime_wasi_config::WasiConfigVariables;
use omnia_wasi_config::{ConfigDefault, WasiConfig};
use omnia_wasi_docstore::{DocStoreDefault, WasiDocStore};
use omnia_wasi_http::{ConnectOptions as HttpOptions, HttpDefault};
use omnia_wasi_identity::{IdentityStub, WasiIdentity};
use omnia_wasi_keyvalue::{KeyValueDefault, WasiKeyValue};
use omnia_wasi_messaging::{MessagingDefault, WasiMessaging};
use omnia_wasi_model::{ModelDefault, WasiModel, WasiModelCtx};
use omnia_wasi_otel::{OtelDefault, WasiOtel};
use omnia_wasi_sql::{ConnectOptions as SqlOptions, SqlDefault, WasiSql};
use omnia_wasi_vault::{VaultDefault, WasiVault};
use omnia_wasi_websocket::{WasiWebSocket, WebSocketDefault};

#[cfg(doc)]
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
///
/// The bundle is deterministic: [`defaults`](Self::defaults) reads no
/// environment variable and opens no socket. Config answers from the map
/// seeded by [`config`](Self::config) alone, the websocket backend serves no
/// listener, and the HTTP client and `SQLite` connection are built from
/// fixed options rather than `HTTP_CONNECT_TIMEOUT` / `SQL_DATABASE`.
#[derive(Clone, Debug)]
pub struct Backends<M = ModelDefault> {
    /// In-memory `wasi:blobstore`.
    pub blobstore: BlobstoreDefault,
    /// Map-backed `wasi:config`, empty until seeded by [`Backends::config`].
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
    /// In-process `omnia:websocket` serving no listener.
    pub websocket: WebSocketDefault,
}

impl Backends {
    /// Every default, freshly constructed from nothing in the environment.
    ///
    /// # Panics
    ///
    /// Panics if a default cannot be constructed.
    pub async fn defaults() -> Self {
        Self {
            blobstore: BlobstoreDefault::connect_with(NoOptions)
                .await
                .expect("in-memory blobstore"),
            config: config_from(WasiConfigVariables::new()),
            docstore: DocStoreDefault::connect_with(NoOptions).await.expect("in-memory docstore"),
            http: HttpDefault::connect_with(HttpOptions { connect_timeout: 10 })
                .await
                .expect("outbound http client"),
            identity: IdentityStub,
            keyvalue: KeyValueDefault::connect_with(NoOptions).await.expect("in-memory keyvalue"),
            messaging: MessagingDefault::connect_with(NoOptions)
                .await
                .expect("in-process messaging"),
            model: ModelDefault,
            otel: OtelDefault,
            // A private database per bundle: the crate default
            // `file::memory:?cache=shared` is one store for the whole process.
            sql: SqlDefault::connect_with(SqlOptions {
                database: ":memory:".to_owned(),
            })
            .await
            .expect("in-memory sqlite"),
            vault: VaultDefault::connect_with(NoOptions).await.expect("in-memory vault"),
            websocket: WebSocketDefault::new(),
        }
    }
}

impl<M> Backends<M> {
    /// The same bundle answering `wasi:config` lookups from `vars` alone.
    #[must_use]
    pub fn config<K, V>(mut self, vars: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        let vars = vars.into_iter().map(|(key, value)| (key.into(), value.into()));
        self.config = config_from(vars.collect());
        self
    }

    /// The same bundle answering completions with `model` — a
    /// [`ScriptedModel`] or any other `WasiModelCtx`.
    #[must_use]
    pub fn model<N: WasiModelCtx + Clone>(self, model: N) -> Backends<N> {
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

fn config_from(vars: WasiConfigVariables) -> ConfigDefault {
    ConfigDefault {
        config_vars: Arc::new(vars),
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
