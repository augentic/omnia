//! `Provider`: every capability double bundled into one provider.

use std::any::Any;
use std::error::Error;
use std::future::Future;

use anyhow::Result;
use bytes::Bytes;
use http_body::Body;
use omnia_guest::document_store::{Document, QueryOptions, QueryResult};
use omnia_guest::orm::{DataType, Row};
use omnia_guest::{
    BlobStore, Broadcast, CasError, Config, ContainerMetadata, DocumentStore, HttpRequest,
    Identity, Message, Model, ObjectMetadata, Plugins, Publish, StateStore, TableStore, model,
    plugins,
};

use crate::guest::{
    FixedIdentity, MapConfig, MatchedHttp, Memory, MemoryDocs, Scripted, ScriptedLoader,
    ScriptedTables, Sink,
};

/// A provider holding one default double per capability.
///
/// Each capability impl delegates to the `pub` field named for it, seeded
/// through a consuming builder of the same name. `StateStore` and `BlobStore`
/// share the one `storage` field, as one [`Memory`] serves both — the shape a
/// production provider's single storage backend has.
///
/// ```rust
/// use omnia_guest::model::{Model as _, Request};
/// use omnia_guest::{Config, StateStore as _};
/// use omnia_test::guest::{MapConfig, Provider, Scripted};
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let provider = Provider::default()
///     .config(MapConfig::default().with([("region", "eu")]))
///     .model(Scripted::answering(["ok"]));
/// provider.storage.insert_state("seen", b"1");
///
/// assert_eq!(Config::get(&provider, "region").await.unwrap(), "eu");
/// let request = Request::builder().messages(vec![]).build();
/// assert_eq!(provider.complete(request).await.unwrap().answer, "ok");
/// assert_eq!(provider.storage.state("seen"), Some(b"1".to_vec()));
/// # });
/// ```
#[derive(Clone, Debug, Default)]
pub struct Provider {
    /// The `Config` double.
    pub config: MapConfig,
    /// The `HttpRequest` double.
    pub http: MatchedHttp,
    /// The `Identity` double.
    pub identity: FixedIdentity,
    /// The `Publish` double.
    pub publish: Sink,
    /// The `Broadcast` double.
    pub broadcast: Sink,
    /// The `StateStore` / `BlobStore` double: one shared `Memory`.
    pub storage: Memory,
    /// The `DocumentStore` double.
    pub docs: MemoryDocs,
    /// The `TableStore` double.
    pub tables: ScriptedTables,
    /// The `Model` double.
    pub model: Scripted,
    /// The `Plugins` double.
    pub plugins: ScriptedLoader,
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
    pub fn model(mut self, model: Scripted) -> Self {
        self.model = model;
        self
    }

    /// Replaces the `plugins` double.
    #[must_use]
    pub fn plugins(mut self, plugins: ScriptedLoader) -> Self {
        self.plugins = plugins;
        self
    }
}

// Several capabilities share method names (`get`, `send`, `delete`, `put`,
// `query`), so every delegation is a fully qualified trait call.

impl Config for Provider {
    fn get(&self, key: &str) -> impl Future<Output = Result<String>> + Send {
        Config::get(&self.config, key)
    }
}

impl HttpRequest for Provider {
    fn fetch<T>(
        &self, request: http::Request<T>,
    ) -> impl Future<Output = Result<http::Response<Bytes>>> + Send
    where
        T: Body + Any + Send,
        T::Data: Into<Vec<u8>>,
        T::Error: Into<Box<dyn Error + Send + Sync + 'static>>,
    {
        HttpRequest::fetch(&self.http, request)
    }
}

impl Identity for Provider {
    fn access_token(&self, identity: String) -> impl Future<Output = Result<String>> + Send {
        Identity::access_token(&self.identity, identity)
    }
}

impl Publish for Provider {
    fn send(&self, topic: &str, message: &Message) -> impl Future<Output = Result<()>> + Send {
        Publish::send(&self.publish, topic, message)
    }
}

impl Broadcast for Provider {
    fn send(
        &self, name: &str, data: &[u8], sockets: Option<Vec<String>>,
    ) -> impl Future<Output = Result<()>> + Send {
        Broadcast::send(&self.broadcast, name, data, sockets)
    }
}

impl StateStore for Provider {
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        StateStore::get(&self.storage, key)
    }

    fn set(
        &self, key: &str, value: &[u8], ttl_secs: Option<u64>,
    ) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        StateStore::set(&self.storage, key, value, ttl_secs)
    }

    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send {
        StateStore::delete(&self.storage, key)
    }

    fn cas(
        &self, key: &str, expected: Option<&[u8]>, value: &[u8],
    ) -> impl Future<Output = Result<(), CasError>> + Send {
        StateStore::cas(&self.storage, key, expected, value)
    }

    fn increment(&self, key: &str, delta: i64) -> impl Future<Output = Result<i64>> + Send {
        StateStore::increment(&self.storage, key, delta)
    }
}

impl BlobStore for Provider {
    fn get(
        &self, container: &str, name: &str,
    ) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        BlobStore::get(&self.storage, container, name)
    }

    fn put(
        &self, container: &str, name: &str, data: &[u8],
    ) -> impl Future<Output = Result<()>> + Send {
        BlobStore::put(&self.storage, container, name, data)
    }

    fn delete(&self, container: &str, name: &str) -> impl Future<Output = Result<()>> + Send {
        BlobStore::delete(&self.storage, container, name)
    }

    fn list(&self, container: &str) -> impl Future<Output = Result<Vec<String>>> + Send {
        BlobStore::list(&self.storage, container)
    }

    fn get_range(
        &self, container: &str, name: &str, start: u64, end: u64,
    ) -> impl Future<Output = Result<Vec<u8>>> + Send {
        BlobStore::get_range(&self.storage, container, name, start, end)
    }

    fn object_info(
        &self, container: &str, name: &str,
    ) -> impl Future<Output = Result<ObjectMetadata>> + Send {
        BlobStore::object_info(&self.storage, container, name)
    }

    fn create_container(&self, name: &str) -> impl Future<Output = Result<()>> + Send {
        BlobStore::create_container(&self.storage, name)
    }

    fn delete_container(&self, name: &str) -> impl Future<Output = Result<()>> + Send {
        BlobStore::delete_container(&self.storage, name)
    }

    fn container_exists(&self, name: &str) -> impl Future<Output = Result<bool>> + Send {
        BlobStore::container_exists(&self.storage, name)
    }

    fn container_info(
        &self, container: &str,
    ) -> impl Future<Output = Result<ContainerMetadata>> + Send {
        BlobStore::container_info(&self.storage, container)
    }
}

impl DocumentStore for Provider {
    fn get(&self, store: &str, id: &str) -> impl Future<Output = Result<Option<Document>>> + Send {
        DocumentStore::get(&self.docs, store, id)
    }

    fn insert(&self, store: &str, doc: &Document) -> impl Future<Output = Result<()>> + Send {
        DocumentStore::insert(&self.docs, store, doc)
    }

    fn put(&self, store: &str, doc: &Document) -> impl Future<Output = Result<()>> + Send {
        DocumentStore::put(&self.docs, store, doc)
    }

    fn delete(&self, store: &str, id: &str) -> impl Future<Output = Result<bool>> + Send {
        DocumentStore::delete(&self.docs, store, id)
    }

    fn query(
        &self, store: &str, options: QueryOptions,
    ) -> impl Future<Output = Result<QueryResult>> + Send {
        DocumentStore::query(&self.docs, store, options)
    }
}

impl TableStore for Provider {
    fn query(
        &self, conn_name: String, query: String, params: Vec<DataType>,
    ) -> impl Future<Output = Result<Vec<Row>>> + Send {
        TableStore::query(&self.tables, conn_name, query, params)
    }

    fn exec(
        &self, conn_name: String, query: String, params: Vec<DataType>,
    ) -> impl Future<Output = Result<u32>> + Send {
        TableStore::exec(&self.tables, conn_name, query, params)
    }
}

impl Model for Provider {
    fn complete(
        &self, request: model::Request,
    ) -> impl Future<Output = Result<model::Reply, model::Error>> + Send {
        Model::complete(&self.model, request)
    }

    fn complete_with<H, F>(
        &self, request: model::Request, handler: H,
    ) -> impl Future<Output = Result<model::Reply, model::Error>> + Send
    where
        H: FnMut(model::ToolCall) -> F + Send,
        F: Future<Output = Result<String, String>> + Send,
    {
        Model::complete_with(&self.model, request, handler)
    }
}

impl Plugins for Provider {
    fn load(
        &self, plugin: &plugins::PluginRef,
    ) -> impl Future<Output = Result<plugins::Plugin, plugins::Error>> + Send {
        Plugins::load(&self.plugins, plugin)
    }
}
