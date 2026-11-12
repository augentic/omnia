//! The store's backend bundle: every `wasi-*` in-memory default, each
//! swappable for another backend of its host.

use std::sync::Arc;

use omnia::{Connect as _, HostCtx, NoOptions, Provides};
use omnia_wasi_blobstore::{BlobStoreClient, BlobStoreDefault, WasiBlobStore};
use omnia_wasi_config::wasmtime_wasi_config::WasiConfigVariables;
use omnia_wasi_config::{ConfigDefault, WasiConfig};
use omnia_wasi_docstore::{DocStoreClient, DocStoreDefault, WasiDocStore};
use omnia_wasi_http::{ConnectOptions as HttpOptions, HttpDefault, WasiHttp};
use omnia_wasi_identity::{IdentityClient, IdentityStub, WasiIdentity};
use omnia_wasi_keyvalue::{
    ConnectOptions as KeyValueOptions, KeyValueClient, KeyValueDefault, WasiKeyValue,
};
use omnia_wasi_messaging::{MessagingClient, MessagingDefault, WasiMessaging};
use omnia_wasi_model::{ModelClient, ModelDefault, WasiModel};
use omnia_wasi_otel::{OtelClient, OtelDefault, WasiOtel};
use omnia_wasi_sql::{ConnectOptions as SqlOptions, SqlClient, SqlDefault, WasiSql};
use omnia_wasi_vault::{VaultClient, VaultDefault, WasiVault};
use omnia_wasi_vcs::{VcsClient, WasiVcs};
use omnia_wasi_websocket::{WasiWebSocket, WebSocketDefault};

#[cfg(doc)]
use super::ScriptedModel;

/// The keyvalue bucket `omnia_sdk`'s wasm32 `StateStore` opens, the one bucket
/// the default keyvalue backend declares.
pub const STATE_BUCKET: &str = "default";

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
/// listener, and the HTTP client, keyvalue buckets, and `SQLite` connection
/// are built from fixed options rather than `HTTP_CONNECT_TIMEOUT` /
/// `KEYVALUE_BUCKETS` / `SQL_DATABASE`. The keyvalue backend declares
/// [`STATE_BUCKET`] alone; a scenario that opens another sets its own with
/// [`keyvalue`](Self::keyvalue).
///
/// One type parameter per swappable host, each defaulting to the in-memory
/// backend, so `Backends` with no arguments is the all-default bundle and a
/// setter changes exactly one parameter. Config, HTTP, and websocket have no
/// production backend to swap in and stay concrete. Version control has no
/// in-memory backend at all: the slot holds [`NoVcs`] until
/// [`vcs`](Self::vcs) fills it, and a deployment linking `WasiVcs` over the
/// default bundle fails to link.
#[derive(Clone, Debug)]
pub struct Backends<
    M = ModelDefault,
    K = KeyValueDefault,
    B = BlobStoreDefault,
    D = DocStoreDefault,
    S = SqlDefault,
    V = VaultDefault,
    G = MessagingDefault,
    I = IdentityStub,
    O = OtelDefault,
    C = NoVcs,
> {
    /// The `wasi:blobstore` backend.
    pub blobstore: B,
    /// Map-backed `wasi:config`, empty until seeded by [`Backends::config`].
    pub config: ConfigDefault,
    /// The `omnia:docstore` backend.
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
    /// The `omnia:vcs` backend, [`NoVcs`] until one is set.
    pub vcs: C,
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
            blobstore: BlobStoreDefault::connect_with(NoOptions)
                .await
                .expect("in-memory blobstore"),
            config: config_from(WasiConfigVariables::new()),
            docstore: DocStoreDefault::connect_with(NoOptions).await.expect("in-memory docstore"),
            http: HttpDefault::connect_with(HttpOptions { connect_timeout: 10 })
                .await
                .expect("outbound http client"),
            identity: IdentityStub,
            keyvalue: KeyValueDefault::connect_with(KeyValueOptions::default())
                .await
                .expect("in-memory keyvalue"),
            messaging: MessagingDefault::connect_with(NoOptions)
                .await
                .expect("in-process messaging"),
            model: ModelDefault,
            otel: OtelDefault,
            // a private database per bundle: the crate default is shared process-wide
            sql: SqlDefault::connect_with(SqlOptions {
                database: ":memory:".to_owned(),
            })
            .await
            .expect("in-memory sqlite"),
            vault: VaultDefault::connect_with(NoOptions).await.expect("in-memory vault"),
            vcs: NoVcs,
            websocket: WebSocketDefault::new(),
        }
    }
}

impl<M, K, B, D, S, V, G, I, O, C> Backends<M, K, B, D, S, V, G, I, O, C> {
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
    /// [`ScriptedModel`] or any other `ModelClient`.
    #[must_use]
    pub fn model<N: ModelClient + Clone>(self, model: N) -> Backends<N, K, B, D, S, V, G, I, O, C> {
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
            vcs: self.vcs,
            websocket: self.websocket,
        }
    }

    /// The same bundle serving `wasi:keyvalue` from `keyvalue`.
    #[must_use]
    pub fn keyvalue<N: KeyValueClient + Clone>(
        self, keyvalue: N,
    ) -> Backends<M, N, B, D, S, V, G, I, O, C> {
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
            vcs: self.vcs,
            websocket: self.websocket,
        }
    }

    /// The same bundle serving `wasi:blobstore` from `blobstore`.
    #[must_use]
    pub fn blobstore<N: BlobStoreClient + Clone>(
        self, blobstore: N,
    ) -> Backends<M, K, N, D, S, V, G, I, O, C> {
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
            vcs: self.vcs,
            websocket: self.websocket,
        }
    }

    /// The same bundle serving `omnia:docstore` from `docstore`.
    #[must_use]
    pub fn docstore<N: DocStoreClient + Clone>(
        self, docstore: N,
    ) -> Backends<M, K, B, N, S, V, G, I, O, C> {
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
            vcs: self.vcs,
            websocket: self.websocket,
        }
    }

    /// The same bundle serving `wasi:sql` from `sql`.
    #[must_use]
    pub fn sql<N: SqlClient + Clone>(self, sql: N) -> Backends<M, K, B, D, N, V, G, I, O, C> {
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
            vcs: self.vcs,
            websocket: self.websocket,
        }
    }

    /// The same bundle serving `wasi:vault` from `vault`.
    #[must_use]
    pub fn vault<N: VaultClient + Clone>(self, vault: N) -> Backends<M, K, B, D, S, N, G, I, O, C> {
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
            vcs: self.vcs,
            websocket: self.websocket,
        }
    }

    /// The same bundle serving `wasi:messaging` from `messaging`.
    #[must_use]
    pub fn messaging<N: MessagingClient + Clone>(
        self, messaging: N,
    ) -> Backends<M, K, B, D, S, V, N, I, O, C> {
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
            vcs: self.vcs,
            websocket: self.websocket,
        }
    }

    /// The same bundle serving `wasi:identity` from `identity`.
    #[must_use]
    pub fn identity<N: IdentityClient + Clone>(
        self, identity: N,
    ) -> Backends<M, K, B, D, S, V, G, N, O, C> {
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
            vcs: self.vcs,
            websocket: self.websocket,
        }
    }

    /// The same bundle serving `wasi:otel` from `otel`.
    #[must_use]
    pub fn otel<N: OtelClient + Clone>(self, otel: N) -> Backends<M, K, B, D, S, V, G, I, N, C> {
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
            vcs: self.vcs,
            websocket: self.websocket,
        }
    }

    /// The same bundle serving `omnia:vcs` from `vcs`, the one slot with no
    /// default to replace.
    #[must_use]
    pub fn vcs<N: VcsClient + Clone>(self, vcs: N) -> Backends<M, K, B, D, S, V, G, I, O, N> {
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
            vault: self.vault,
            vcs,
            websocket: self.websocket,
        }
    }
}

/// One `Provides` impl per host, each yielding the named field's borrow.
///
/// Every impl carries the full set of bounds over the slots that have a
/// default, so that a bundle holding a non-backend in one of them fails to
/// link at all, rather than for the one host that reaches the slot. The
/// `vcs` slot stays unbounded here: it holds [`NoVcs`] by default, and only
/// the `WasiVcs` impl below asks it for a backend.
macro_rules! provides {
    ($($host:ty => $field:ident),* $(,)?) => {
        $(
            impl<M, K, B, D, S, V, G, I, O, C> Provides<$host>
                for Backends<M, K, B, D, S, V, G, I, O, C>
            where
                M: ModelClient + Clone,
                K: KeyValueClient + Clone,
                B: BlobStoreClient + Clone,
                D: DocStoreClient + Clone,
                S: SqlClient + Clone,
                V: VaultClient + Clone,
                G: MessagingClient + Clone,
                I: IdentityClient + Clone,
                O: OtelClient + Clone,
                C: Clone + Send + Sync + 'static,
            {
                fn borrow(&mut self) -> <$host as HostCtx>::Borrow<'_> {
                    &mut self.$field
                }
            }
        )*
    };
}

provides! {
    WasiBlobStore => blobstore,
    WasiConfig => config,
    WasiDocStore => docstore,
    WasiHttp => http,
    WasiIdentity => identity,
    WasiKeyValue => keyvalue,
    WasiMessaging => messaging,
    WasiModel => model,
    WasiOtel => otel,
    WasiSql => sql,
    WasiVault => vault,
    WasiWebSocket => websocket,
}

impl<M, K, B, D, S, V, G, I, O, C> Provides<WasiVcs> for Backends<M, K, B, D, S, V, G, I, O, C>
where
    M: ModelClient + Clone,
    K: KeyValueClient + Clone,
    B: BlobStoreClient + Clone,
    D: DocStoreClient + Clone,
    S: SqlClient + Clone,
    V: VaultClient + Clone,
    G: MessagingClient + Clone,
    I: IdentityClient + Clone,
    O: OtelClient + Clone,
    C: VcsClient + Clone,
{
    fn borrow(&mut self) -> <WasiVcs as HostCtx>::Borrow<'_> {
        &mut self.vcs
    }
}

fn config_from(vars: WasiConfigVariables) -> ConfigDefault {
    ConfigDefault {
        config_vars: Arc::new(vars),
    }
}

/// The empty `omnia:vcs` slot: no backend, so no deployment links `WasiVcs`
/// over it.
///
/// `omnia:vcs` ships no in-memory default, since a repository is a tree on
/// disk; a scenario that drives it sets a backend with
/// [`Backends::vcs`].
#[derive(Clone, Copy, Debug, Default)]
pub struct NoVcs;
