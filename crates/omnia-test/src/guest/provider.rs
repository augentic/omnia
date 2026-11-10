//! `Provider`: every capability double bundled into one provider.

use crate::guest::{
    FixedIdentity, MapConfig, MatchedHttp, Memory, MemoryDocs, ScriptedLoader, ScriptedModel,
    ScriptedTables, ScriptedVcs, Sink,
};

/// A provider holding one default double per capability.
///
/// Each capability impl delegates to the `pub` field named for it, seeded
/// through a consuming builder of the same name. `StateStore`, `BlobStore`,
/// and `Vault` share the one `storage` field, as one [`Memory`] serves all
/// three — the shape a production provider's storage backend has.
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
/// assert_eq!(Config::get(&provider, "region").await.unwrap(), "eu");
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
    /// The `HttpRequest` double.
    #[provider(HttpRequest)]
    pub http: MatchedHttp,
    /// The `Identity` double.
    #[provider(Identity)]
    pub identity: FixedIdentity,
    /// The `Publish` double.
    #[provider(Publish)]
    pub publish: Sink,
    /// The `Broadcast` double.
    #[provider(Broadcast)]
    pub broadcast: Sink,
    /// The `StateStore` / `BlobStore` / `Vault` double: one shared `Memory`.
    #[provider(StateStore, BlobStore, Vault)]
    pub storage: Memory,
    /// The `DocumentStore` double.
    #[provider(DocumentStore)]
    pub docs: MemoryDocs,
    /// The `TableStore` double.
    #[provider(TableStore)]
    pub tables: ScriptedTables,
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

    /// Replaces the `publish` double.
    #[must_use]
    pub fn publish(mut self, publish: Sink) -> Self {
        self.publish = publish;
        self
    }

    /// Replaces the `broadcast` double.
    #[must_use]
    pub fn broadcast(mut self, broadcast: Sink) -> Self {
        self.broadcast = broadcast;
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

    /// Replaces the `tables` double.
    #[must_use]
    pub fn tables(mut self, tables: ScriptedTables) -> Self {
        self.tables = tables;
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
