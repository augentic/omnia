//! The store's backend bundle: every `wasi-*` in-memory default, each
//! swappable for another backend of its host.

use std::collections::BTreeMap;

use omnia::{Connect as _, HostCtx, NoOptions, Provides};
use omnia_wasi_blobstore::{BlobStoreClient, BlobStoreDefault, WasiBlobStore};
use omnia_wasi_config::{ConfigDefault, WasiConfig};
use omnia_wasi_docstore::{
    ConnectOptions as DocStoreOptions, DocStoreClient, DocStoreDefault, WasiDocStore,
};
use omnia_wasi_http::{ConnectOptions as HttpOptions, HttpBorrow, HttpDefault, WasiHttp};
use omnia_wasi_identity::{IdentityClient, IdentityStub, WasiIdentity};
use omnia_wasi_keyvalue::{
    ConnectOptions as KeyValueOptions, KeyValueClient, KeyValueDefault, WasiKeyValue,
};
use omnia_wasi_messaging::{MessagingClient, MessagingDefault, WasiMessaging};
use omnia_wasi_model::{ConnectOptions as ModelOptions, ModelClient, ModelDefault, WasiModel};
use omnia_wasi_otel::{OtelClient, OtelDefault, WasiOtel};
use omnia_wasi_sql::{ConnectOptions as SqlOptions, SqlClient, SqlDefault, WasiSql};
use omnia_wasi_vault::{ConnectOptions as VaultOptions, VaultClient, VaultDefault, WasiVault};
use omnia_wasi_vcs::{VcsClient, WasiVcs};
use omnia_wasi_websocket::{WasiWebSocket, WebSocketClient, WebSocketDefault};

#[cfg(doc)]
use super::{MatchedHttp, ScriptedModel, Sink};

/// The one bucket the default keyvalue backend declares, the `DEFAULT`
/// bucket of `omnia_sdk::keyvalue`.
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
/// listener, and the HTTP client, keyvalue buckets, docstore collections,
/// `SQLite` databases, identities, vault lockers, and echo models are built
/// from fixed options rather than `HTTP_CONNECT_TIMEOUT` /
/// `KEYVALUE_BUCKETS` / `DOCSTORE_COLLECTIONS` / `SQL_DATABASES` /
/// `IDENTITY_NAMES` / `VAULT_LOCKERS` / `MODEL_NAMES`. The keyvalue backend
/// declares [`STATE_BUCKET`] alone, the docstore backend the `default`
/// collection alone, the `SQLite` backend a private `default` database
/// alone, the identity stub the `default` identity alone, the vault backend
/// the `default` locker alone, and the echo model backend the `default`
/// model alone. A scenario that opens another name sets its own backend
/// with [`keyvalue`](Self::keyvalue), [`docstore`](Self::docstore),
/// [`sql`](Self::sql), [`identity`](Self::identity),
/// [`vault`](Self::vault), or [`model`](Self::model).
///
/// One type parameter per swappable host, each defaulting to the in-memory
/// backend, so `Backends` with no arguments is the all-default bundle and a
/// setter changes exactly one parameter. Config alone stays concrete, since
/// the map a scenario seeds is the whole backend. Version control has no
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
    H = HttpDefault,
    W = WebSocketDefault,
> {
    /// The `omnia:blobstore` backend.
    pub blobstore: B,
    /// Map-backed `omnia:config`, empty until seeded by [`Backends::config`].
    pub config: ConfigDefault,
    /// The `omnia:docstore` backend.
    pub docstore: D,
    /// The outbound `wasi:http` backend.
    pub http: H,
    /// The `omnia:identity` backend.
    pub identity: I,
    /// The `omnia:keyvalue` backend.
    pub keyvalue: K,
    /// The `omnia:messaging` backend.
    pub messaging: G,
    /// The `omnia:model` backend.
    pub model: M,
    /// The `omnia:otel` backend.
    pub otel: O,
    /// The `omnia:sql` backend.
    pub sql: S,
    /// The `omnia:vault` backend.
    pub vault: V,
    /// The `omnia:vcs` backend, [`NoVcs`] until one is set.
    pub vcs: C,
    /// The `omnia:websocket` backend, by default in process with no
    /// listener.
    pub websocket: W,
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
            config: ConfigDefault::default(),
            docstore: DocStoreDefault::connect_with(DocStoreOptions::default())
                .await
                .expect("in-memory docstore"),
            http: HttpDefault::connect_with(HttpOptions { connect_timeout: 10 })
                .await
                .expect("outbound http client"),
            identity: IdentityStub::default(),
            keyvalue: KeyValueDefault::connect_with(KeyValueOptions::default())
                .await
                .expect("in-memory keyvalue"),
            messaging: MessagingDefault::connect_with(NoOptions)
                .await
                .expect("in-process messaging"),
            model: ModelDefault::connect_with(ModelOptions::default()).await.expect("echo model"),
            otel: OtelDefault,
            // a private database per bundle: the crate default is shared process-wide
            sql: SqlDefault::connect_with(SqlOptions {
                database: ":memory:".to_owned(),
                databases: BTreeMap::new(),
            })
            .await
            .expect("in-memory sqlite"),
            vault: VaultDefault::connect_with(VaultOptions::default())
                .await
                .expect("in-memory vault"),
            vcs: NoVcs,
            websocket: WebSocketDefault::new(),
        }
    }
}

impl<M, K, B, D, S, V, G, I, O, C, H, W> Backends<M, K, B, D, S, V, G, I, O, C, H, W> {
    /// The same bundle answering `omnia:config` lookups from `vars` alone.
    #[must_use]
    pub fn config<Key, Value>(mut self, vars: impl IntoIterator<Item = (Key, Value)>) -> Self
    where
        Key: Into<String>,
        Value: Into<String>,
    {
        self.config = vars.into_iter().collect();
        self
    }

    /// The same bundle answering completions with `model` — a
    /// [`ScriptedModel`] or any other `ModelClient`.
    #[must_use]
    pub fn model<N: ModelClient + Clone>(
        self, model: N,
    ) -> Backends<N, K, B, D, S, V, G, I, O, C, H, W> {
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

    /// The same bundle serving `omnia:keyvalue` from `keyvalue`.
    #[must_use]
    pub fn keyvalue<N: KeyValueClient + Clone>(
        self, keyvalue: N,
    ) -> Backends<M, N, B, D, S, V, G, I, O, C, H, W> {
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

    /// The same bundle serving `omnia:blobstore` from `blobstore`.
    #[must_use]
    pub fn blobstore<N: BlobStoreClient + Clone>(
        self, blobstore: N,
    ) -> Backends<M, K, N, D, S, V, G, I, O, C, H, W> {
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
    ) -> Backends<M, K, B, N, S, V, G, I, O, C, H, W> {
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

    /// The same bundle serving `omnia:sql` from `sql`.
    #[must_use]
    pub fn sql<N: SqlClient + Clone>(self, sql: N) -> Backends<M, K, B, D, N, V, G, I, O, C, H, W> {
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

    /// The same bundle serving `omnia:vault` from `vault`.
    #[must_use]
    pub fn vault<N: VaultClient + Clone>(
        self, vault: N,
    ) -> Backends<M, K, B, D, S, N, G, I, O, C, H, W> {
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

    /// The same bundle serving `omnia:messaging` from `messaging`.
    #[must_use]
    pub fn messaging<N: MessagingClient + Clone>(
        self, messaging: N,
    ) -> Backends<M, K, B, D, S, V, N, I, O, C, H, W> {
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

    /// The same bundle serving `omnia:identity` from `identity`.
    #[must_use]
    pub fn identity<N: IdentityClient + Clone>(
        self, identity: N,
    ) -> Backends<M, K, B, D, S, V, G, N, O, C, H, W> {
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

    /// The same bundle serving `omnia:otel` from `otel`.
    #[must_use]
    pub fn otel<N: OtelClient + Clone>(
        self, otel: N,
    ) -> Backends<M, K, B, D, S, V, G, I, N, C, H, W> {
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
    pub fn vcs<N: VcsClient + Clone>(self, vcs: N) -> Backends<M, K, B, D, S, V, G, I, O, N, H, W> {
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

    /// The same bundle answering outbound `wasi:http` with `http` — a
    /// [`MatchedHttp`] or any other `HttpBorrow`.
    #[must_use]
    pub fn http<N: HttpBorrow + Clone + Sync + 'static>(
        self, http: N,
    ) -> Backends<M, K, B, D, S, V, G, I, O, C, N, W> {
        Backends {
            blobstore: self.blobstore,
            config: self.config,
            docstore: self.docstore,
            http,
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

    /// The same bundle serving `omnia:websocket` from `websocket` — a
    /// [`Sink`] or any other `WebSocketClient`.
    #[must_use]
    pub fn websocket<N: WebSocketClient + Clone>(
        self, websocket: N,
    ) -> Backends<M, K, B, D, S, V, G, I, O, C, H, N> {
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
            vcs: self.vcs,
            websocket,
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
            impl<M, K, B, D, S, V, G, I, O, C, H, W> Provides<$host>
                for Backends<M, K, B, D, S, V, G, I, O, C, H, W>
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
                H: HttpBorrow + Clone + Sync + 'static,
                W: WebSocketClient + Clone,
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

impl<M, K, B, D, S, V, G, I, O, C, H, W> Provides<WasiVcs>
    for Backends<M, K, B, D, S, V, G, I, O, C, H, W>
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
    H: HttpBorrow + Clone + Sync + 'static,
    W: WebSocketClient + Clone,
{
    fn borrow(&mut self) -> <WasiVcs as HostCtx>::Borrow<'_> {
        &mut self.vcs
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
