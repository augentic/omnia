//! `Provider`: every capability double bundled into one provider.

use crate::guest::{
    FixedIdentity, MapConfig, MatchedHttp, Memory, MemoryDocs, ScriptedLoader, ScriptedModel,
    ScriptedTables, ScriptedVcs, Sink,
};

/// A provider holding one default double per capability.
///
/// Each capability impl delegates to the `pub` field named for it, seeded
/// through a consuming builder of the same name. `StateStore`, `BlobStore`,
/// and `SecretStore` share the one `storage` field, as one [`Memory`] serves
/// all three — the shape a production provider's storage backend has.
///
/// ```rust
/// use omnia_sdk::model::{Model as _, Request};
/// use omnia_sdk::{Config, StateStore as _};
/// use omnia_test::guest::{MapConfig, Provider, ScriptedModel};
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let provider = Provider::default()
///     .config(MapConfig::default().with([("region", "eu")]))
///     .model(ScriptedModel::answering(["ok"]));
/// provider.storage.insert_state("seen", b"1");
///
/// assert_eq!(Config::get(&provider, "region").await.unwrap().as_deref(), Some("eu"));
/// let request = Request::builder().messages(vec![]).build();
/// assert_eq!(provider.complete(request).await.unwrap().answer, "ok");
/// assert_eq!(provider.storage.state("seen"), Some(b"1".to_vec()));
/// # });
/// ```
#[derive(Clone, Debug, Default, omnia_sdk::Provider)]
pub struct Provider {
    /// The `Config` double.
    #[provider(Config)]
    pub config: MapConfig,
    /// The `HttpClient` double.
    #[provider(HttpClient)]
    pub http: MatchedHttp,
    /// The `Identity` double.
    #[provider(Identity)]
    pub identity: FixedIdentity,
    /// The `Publisher` double.
    #[provider(Publisher)]
    pub publisher: Sink,
    /// The `WebSockets` double.
    #[provider(WebSockets)]
    pub websockets: Sink,
    /// The `StateStore` / `BlobStore` / `SecretStore` double: one shared `Memory`.
    #[provider(StateStore, BlobStore, SecretStore)]
    pub storage: Memory,
    /// The `DocumentDb` double.
    #[provider(DocumentDb)]
    pub docs: MemoryDocs,
    /// The `SqlDb` double.
    #[provider(SqlDb)]
    pub sql: ScriptedTables,
    /// The `Model` double.
    #[provider(Model)]
    pub model: ScriptedModel,
    /// The `Plugins` double.
    #[provider(Plugins)]
    pub plugins: ScriptedLoader,
    /// The `Vcs` double.
    #[provider(Vcs)]
    pub vcs: ScriptedVcs,
}

impl Provider {
    /// Replaces the `config` double.
    #[must_use]
    pub fn config(mut self, config: MapConfig) -> Self {
        self.config = config;
        self
    }

    /// Replaces the `http` double.
    #[must_use]
    pub fn http(mut self, http: MatchedHttp) -> Self {
        self.http = http;
        self
    }

    /// Replaces the `identity` double.
    #[must_use]
    pub fn identity(mut self, identity: FixedIdentity) -> Self {
        self.identity = identity;
        self
    }

    /// Replaces the `publisher` double.
    #[must_use]
    pub fn publisher(mut self, publisher: Sink) -> Self {
        self.publisher = publisher;
        self
    }

    /// Replaces the `websockets` double.
    #[must_use]
    pub fn websockets(mut self, websockets: Sink) -> Self {
        self.websockets = websockets;
        self
    }

    /// Replaces the `storage` double.
    #[must_use]
    pub fn storage(mut self, storage: Memory) -> Self {
        self.storage = storage;
        self
    }

    /// Replaces the `docs` double.
    #[must_use]
    pub fn docs(mut self, docs: MemoryDocs) -> Self {
        self.docs = docs;
        self
    }

    /// Replaces the `sql` double.
    #[must_use]
    pub fn sql(mut self, sql: ScriptedTables) -> Self {
        self.sql = sql;
        self
    }

    /// Replaces the `model` double.
    #[must_use]
    pub fn model(mut self, model: ScriptedModel) -> Self {
        self.model = model;
        self
    }

    /// Replaces the `plugins` double.
    #[must_use]
    pub fn plugins(mut self, plugins: ScriptedLoader) -> Self {
        self.plugins = plugins;
        self
    }

    /// Replaces the `vcs` double.
    #[must_use]
    pub fn vcs(mut self, vcs: ScriptedVcs) -> Self {
        self.vcs = vcs;
        self
    }
}
