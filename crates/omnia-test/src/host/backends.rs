//! The store's backend bundle: every `wasi-*` in-memory default, each
//! swappable for another backend of its host.

use std::sync::Arc;

use omnia::{Backend as _, HostCtx, HttpCtx, NoOptions, Provides};
use omnia_wasi_blobstore::{BlobstoreDefault, WasiBlobstore, WasiBlobstoreCtx};
use omnia_wasi_config::wasmtime_wasi_config::WasiConfigVariables;
use omnia_wasi_config::{ConfigDefault, WasiConfig};
use omnia_wasi_docstore::{DocStoreDefault, WasiDocStore, WasiDocStoreCtx};
use omnia_wasi_http::{ConnectOptions as HttpOptions, HttpDefault};
use omnia_wasi_identity::{IdentityStub, WasiIdentity, WasiIdentityCtx};
use omnia_wasi_keyvalue::{KeyValueDefault, WasiKeyValue, WasiKeyValueCtx};
use omnia_wasi_messaging::{MessagingDefault, WasiMessaging, WasiMessagingCtx};
use omnia_wasi_model::{ModelDefault, WasiModel, WasiModelCtx};
use omnia_wasi_otel::{OtelDefault, WasiOtel, WasiOtelCtx};
use omnia_wasi_sql::{ConnectOptions as SqlOptions, SqlDefault, WasiSql, WasiSqlCtx};
use omnia_wasi_vault::{VaultDefault, WasiVault, WasiVaultCtx};
use omnia_wasi_websocket::{WasiWebSocket, WebSocketDefault};

#[cfg(doc)]
use super::ScriptedModel;

/// The keyvalue bucket `omnia_sdk`'s wasm32 `StateStore` opens.
pub const STATE_BUCKET: &str = "cache";

/// Every host's default backend as one bundle, each swappable for another
/// backend of its host — the model for a [`ScriptedModel`], a store for a
/// production backend or a recording wrapper.
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
///
/// One type parameter per swappable host, each defaulting to the in-memory
/// backend, so `Backends` with no arguments is the all-default bundle and a
/// setter changes exactly one parameter. Config, HTTP, and websocket have no
/// production backend to swap in and stay concrete.
#[derive(Clone, Debug)]
pub struct Backends<
    M = ModelDefault,
    K = KeyValueDefault,
    B = BlobstoreDefault,
    D = DocStoreDefault,
    S = SqlDefault,
    V = VaultDefault,
    G = MessagingDefault,
    I = IdentityStub,
    O = OtelDefault,
> {
    /// The `wasi:blobstore` backend.
    pub blobstore: B,
    /// Map-backed `wasi:config`, empty until seeded by [`Backends::config`].
    pub config: ConfigDefault,
    /// The `wasi:docstore` backend.
    pub docstore: D,
    /// Outbound `wasi:http`.
    pub http: HttpDefault,
    /// The `wasi:identity` backend.
    pub identity: I,
    /// The `wasi:keyvalue` backend.
    pub keyvalue: K,
    /// The `wasi:messaging` backend.
    pub messaging: G,
    /// The `omnia:model` backend.
    pub model: M,
    /// The `wasi:otel` backend.
    pub otel: O,
    /// The `wasi:sql` backend.
    pub sql: S,
    /// The `wasi:vault` backend.
    pub vault: V,
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

impl<M, K, B, D, S, V, G, I, O> Backends<M, K, B, D, S, V, G, I, O> {
    /// The same bundle answering `wasi:config` lookups from `vars` alone.
    #[must_use]
    pub fn config<Key, Value>(mut self, vars: impl IntoIterator<Item = (Key, Value)>) -> Self
    where
        Key: Into<String>,
        Value: Into<String>,
    {
        let vars = vars.into_iter().map(|(key, value)| (key.into(), value.into()));
        self.config = config_from(vars.collect());
        self
    }

    /// The same bundle answering completions with `model` — a
    /// [`ScriptedModel`] or any other `WasiModelCtx`.
    #[must_use]
    pub fn model<N: WasiModelCtx + Clone>(self, model: N) -> Backends<N, K, B, D, S, V, G, I, O> {
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

    /// The same bundle serving `wasi:keyvalue` from `keyvalue`.
    #[must_use]
    pub fn keyvalue<N: WasiKeyValueCtx + Clone>(
        self, keyvalue: N,
    ) -> Backends<M, N, B, D, S, V, G, I, O> {
        Backends {
            blobstore: self.blobstore,
            config: self.config,
            docstore: self.docstore,
            http: self.http,
            identity: self.identity,
            keyvalue,
            messaging: self.messaging,
            model: self.model,
            otel: self.otel,
            sql: self.sql,
            vault: self.vault,
            websocket: self.websocket,
        }
    }

    /// The same bundle serving `wasi:blobstore` from `blobstore`.
    #[must_use]
    pub fn blobstore<N: WasiBlobstoreCtx + Clone>(
        self, blobstore: N,
    ) -> Backends<M, K, N, D, S, V, G, I, O> {
        Backends {
            blobstore,
            config: self.config,
            docstore: self.docstore,
            http: self.http,
            identity: self.identity,
            keyvalue: self.keyvalue,
            messaging: self.messaging,
            model: self.model,
            otel: self.otel,
            sql: self.sql,
            vault: self.vault,
            websocket: self.websocket,
        }
    }

    /// The same bundle serving `wasi:docstore` from `docstore`.
    #[must_use]
    pub fn docstore<N: WasiDocStoreCtx + Clone>(
        self, docstore: N,
    ) -> Backends<M, K, B, N, S, V, G, I, O> {
        Backends {
            blobstore: self.blobstore,
            config: self.config,
            docstore,
            http: self.http,
            identity: self.identity,
            keyvalue: self.keyvalue,
            messaging: self.messaging,
            model: self.model,
            otel: self.otel,
            sql: self.sql,
            vault: self.vault,
            websocket: self.websocket,
        }
    }

    /// The same bundle serving `wasi:sql` from `sql`.
    #[must_use]
    pub fn sql<N: WasiSqlCtx + Clone>(self, sql: N) -> Backends<M, K, B, D, N, V, G, I, O> {
        Backends {
            blobstore: self.blobstore,
            config: self.config,
            docstore: self.docstore,
            http: self.http,
            identity: self.identity,
            keyvalue: self.keyvalue,
            messaging: self.messaging,
            model: self.model,
            otel: self.otel,
            sql,
            vault: self.vault,
            websocket: self.websocket,
        }
    }

    /// The same bundle serving `wasi:vault` from `vault`.
    #[must_use]
    pub fn vault<N: WasiVaultCtx + Clone>(self, vault: N) -> Backends<M, K, B, D, S, N, G, I, O> {
        Backends {
            blobstore: self.blobstore,
            config: self.config,
            docstore: self.docstore,
            http: self.http,
            identity: self.identity,
            keyvalue: self.keyvalue,
            messaging: self.messaging,
            model: self.model,
            otel: self.otel,
            sql: self.sql,
            vault,
            websocket: self.websocket,
        }
    }

    /// The same bundle serving `wasi:messaging` from `messaging`.
    #[must_use]
    pub fn messaging<N: WasiMessagingCtx + Clone>(
        self, messaging: N,
    ) -> Backends<M, K, B, D, S, V, N, I, O> {
        Backends {
            blobstore: self.blobstore,
            config: self.config,
            docstore: self.docstore,
            http: self.http,
            identity: self.identity,
            keyvalue: self.keyvalue,
            messaging,
            model: self.model,
            otel: self.otel,
            sql: self.sql,
            vault: self.vault,
            websocket: self.websocket,
        }
    }

    /// The same bundle serving `wasi:identity` from `identity`.
    #[must_use]
    pub fn identity<N: WasiIdentityCtx + Clone>(
        self, identity: N,
    ) -> Backends<M, K, B, D, S, V, G, N, O> {
        Backends {
            blobstore: self.blobstore,
            config: self.config,
            docstore: self.docstore,
            http: self.http,
            identity,
            keyvalue: self.keyvalue,
            messaging: self.messaging,
            model: self.model,
            otel: self.otel,
            sql: self.sql,
            vault: self.vault,
            websocket: self.websocket,
        }
    }

    /// The same bundle serving `wasi:otel` from `otel`.
    #[must_use]
    pub fn otel<N: WasiOtelCtx + Clone>(self, otel: N) -> Backends<M, K, B, D, S, V, G, I, N> {
        Backends {
            blobstore: self.blobstore,
            config: self.config,
            docstore: self.docstore,
            http: self.http,
            identity: self.identity,
            keyvalue: self.keyvalue,
            messaging: self.messaging,
            model: self.model,
            otel,
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

/// One `Provides` impl per host, each yielding the named field's borrow.
///
/// Every impl carries the full set of bounds so that a bundle holding a
/// non-backend in any slot fails to link at all, rather than for the one
/// host that reaches the slot.
macro_rules! provides {
    ($($host:ty => $field:ident),* $(,)?) => {
        $(
            impl<M, K, B, D, S, V, G, I, O> Provides<$host> for Backends<M, K, B, D, S, V, G, I, O>
            where
                M: WasiModelCtx + Clone,
                K: WasiKeyValueCtx + Clone,
                B: WasiBlobstoreCtx + Clone,
                D: WasiDocStoreCtx + Clone,
                S: WasiSqlCtx + Clone,
                V: WasiVaultCtx + Clone,
                G: WasiMessagingCtx + Clone,
                I: WasiIdentityCtx + Clone,
                O: WasiOtelCtx + Clone,
            {
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
