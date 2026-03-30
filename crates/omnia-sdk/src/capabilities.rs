//! # Provider
//!
//! Provider defines external data interfaces for the crate.

use std::any::Any;
use std::collections::HashMap;
use std::error::Error;
use std::future::Future;

use anyhow::Result;
#[cfg(target_arch = "wasm32")]
use anyhow::{Context, anyhow};
use bytes::Bytes;
use http::{Request, Response};
use http_body::Body;
use omnia_wasi_sql::{DataType, Row};

/// The `Config` trait is used by implementers to provide configuration from
/// WASI-guest to dependent crates.
pub trait Config: Send + Sync {
    /// Get configuration setting.
    #[cfg(not(target_arch = "wasm32"))]
    fn get(&self, key: &str) -> impl Future<Output = Result<String>> + Send;

    /// Get configuration setting.
    #[cfg(target_arch = "wasm32")]
    fn get(&self, key: &str) -> impl Future<Output = Result<String>> + Send {
        async move {
            let config = omnia_wasi_config::store::get(key).context("getting configuration")?;
            config.ok_or_else(|| anyhow!("configuration not found"))
        }
    }
}

/// The `HttpRequest` trait defines the behavior for fetching data from a source.
pub trait HttpRequest: Send + Sync {
    /// Make outbound HTTP request.
    #[cfg(not(target_arch = "wasm32"))]
    fn fetch<T>(&self, request: Request<T>) -> impl Future<Output = Result<Response<Bytes>>> + Send
    where
        T: Body + Any + Send,
        T::Data: Into<Vec<u8>>,
        T::Error: Into<Box<dyn Error + Send + Sync + 'static>>;

    /// Make outbound HTTP request.
    #[cfg(target_arch = "wasm32")]
    fn fetch<T>(&self, request: Request<T>) -> impl Future<Output = Result<Response<Bytes>>> + Send
    where
        T: Body + Any + Send,
        T::Data: Into<Vec<u8>>,
        T::Error: Into<Box<dyn Error + Send + Sync + 'static>>,
    {
        async move { omnia_wasi_http::handle(request).await }
    }
}

/// Message represents a message to be published.
#[derive(Clone, Debug)]
pub struct Message {
    /// The message payload.
    pub payload: Vec<u8>,
    /// The message headers.
    pub headers: HashMap<String, String>,
}

impl Message {
    /// Create a new message with the specified payload.
    #[must_use]
    pub fn new(payload: &[u8]) -> Self {
        Self {
            payload: payload.to_vec(),
            headers: HashMap::new(),
        }
    }
}

/// The `Publisher` trait defines the message publishing behavior.
pub trait Publish: Send + Sync {
    /// Publish (send) a message to a topic.
    #[cfg(not(target_arch = "wasm32"))]
    fn send(&self, topic: &str, message: &Message) -> impl Future<Output = Result<()>> + Send;

    /// Publish (send) a message to a topic.
    #[cfg(target_arch = "wasm32")]
    fn send(&self, topic: &str, message: &Message) -> impl Future<Output = Result<()>> + Send {
        use omnia_wasi_messaging::producer;
        use omnia_wasi_messaging::types::{self as wasi, Client};

        async move {
            let client =
                Client::connect("host".to_string()).await.context("connecting to broker")?;
            producer::send(&client, topic.to_string(), wasi::Message::new(&message.payload))
                .await
                .with_context(|| format!("sending message to {topic}"))
        }
    }
}

/// The `StateStore` trait defines the behavior storing and retrieving train state.
pub trait StateStore: Send + Sync {
    /// Retrieve a previously stored value from the state store.
    #[cfg(not(target_arch = "wasm32"))]
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;

    /// Store a value in the state store.
    #[cfg(not(target_arch = "wasm32"))]
    fn set(
        &self, key: &str, value: &[u8], ttl_secs: Option<u64>,
    ) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;

    /// Delete a value from the state store.
    #[cfg(not(target_arch = "wasm32"))]
    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send;

    /// Retrieve a previously stored value from the state store.
    #[cfg(target_arch = "wasm32")]
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        async move {
            let bucket =
                omnia_wasi_keyvalue::cache::open("cache").await.context("opening cache")?;
            bucket.get(key).await.context("reading state from cache")
        }
    }

    /// Store a value in the state store.
    #[cfg(target_arch = "wasm32")]
    fn set(
        &self, key: &str, value: &[u8], ttl_secs: Option<u64>,
    ) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        async move {
            let bucket =
                omnia_wasi_keyvalue::cache::open("cache").await.context("opening cache")?;
            bucket.set(key, value, ttl_secs).await.context("reading state from cache")
        }
    }

    /// Delete a value from the state store.
    #[cfg(target_arch = "wasm32")]
    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send {
        async move {
            let bucket =
                omnia_wasi_keyvalue::cache::open("cache").await.context("opening cache")?;
            bucket.delete(key).await.context("deleting entry from cache")
        }
    }
}

/// The `Identity` trait defines behaviors for interacting with identity providers.
pub trait Identity: Send + Sync {
    /// Get an access token for the specified identity.
    #[cfg(not(target_arch = "wasm32"))]
    fn access_token(&self, identity: String) -> impl Future<Output = Result<String>> + Send;

    /// Get an access token for the specified identity.
    #[cfg(target_arch = "wasm32")]
    fn access_token(&self, identity: String) -> impl Future<Output = Result<String>> + Send {
        use omnia_wasi_identity::credentials::get_identity;

        async move {
            let identity = wit_bindgen::block_on(get_identity(identity))?;
            let access_token =
                wit_bindgen::block_on(async move { identity.get_token(vec![]).await })?;
            Ok(access_token.token)
        }
    }
}

/// Trait for types that provide ORM database access.
///
/// Implement this trait to enable ORM operations. Default implementations
/// use the WASI SQL bindings to execute queries.
pub trait TableStore: Send + Sync {
    /// Executes a query and returns the result rows.
    #[cfg(not(target_arch = "wasm32"))]
    fn query(
        &self, cnn_name: String, query: String, params: Vec<DataType>,
    ) -> impl Future<Output = Result<Vec<Row>>> + Send;

    /// Executes a statement and returns the number of affected rows.
    #[cfg(not(target_arch = "wasm32"))]
    fn exec(
        &self, cnn_name: String, query: String, params: Vec<DataType>,
    ) -> impl Future<Output = Result<u32>> + Send;

    /// Executes a query and returns the result rows.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails, statement preparation fails, or query execution fails.
    #[cfg(target_arch = "wasm32")]
    fn query(
        &self, cnn_name: String, query: String, params: Vec<DataType>,
    ) -> impl Future<Output = Result<Vec<Row>>> + Send {
        use omnia_wasi_sql::types::{Connection, Statement};
        async move {
            let cnn = Connection::open(cnn_name)
                .await
                .map_err(|e| anyhow!("failed to open connection: {}", e.trace()))?;

            let stmt = Statement::prepare(query, params)
                .await
                .map_err(|e| anyhow!("failed to prepare statement: {}", e.trace()))?;

            let res = omnia_wasi_sql::readwrite::query(&cnn, &stmt)
                .await
                .map_err(|e| anyhow!("query failed: {}", e.trace()))?;

            Ok(res)
        }
    }

    /// Executes a statement and returns the number of affected rows.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails, statement preparation fails, or execution fails.
    #[cfg(target_arch = "wasm32")]
    fn exec(
        &self, cnn_name: String, query: String, params: Vec<DataType>,
    ) -> impl Future<Output = Result<u32>> + Send {
        use omnia_wasi_sql::types::{Connection, Statement};
        async move {
            let cnn = Connection::open(cnn_name)
                .await
                .map_err(|e| anyhow!("failed to open connection: {}", e.trace()))?;

            let stmt = Statement::prepare(query, params)
                .await
                .map_err(|e| anyhow!("failed to prepare statement: {}", e.trace()))?;

            let res = omnia_wasi_sql::readwrite::exec(&cnn, &stmt)
                .await
                .map_err(|e| anyhow!("exec failed: {}", e.trace()))?;

            Ok(res)
        }
    }
}

/// JSON document storage (WASI JSON DB).
///
/// Default WASM implementations delegate to `wasi:jsondb` via `omnia-wasi-jsondb`.
pub trait DocumentStore: Send + Sync {
    /// Fetch a document by id.
    #[cfg(not(target_arch = "wasm32"))]
    fn get(
        &self, store: &str, id: &str,
    ) -> impl Future<Output = Result<Option<crate::document_store::Document>>> + Send;

    /// Insert a new document (fails if the id already exists).
    #[cfg(not(target_arch = "wasm32"))]
    fn insert(
        &self, store: &str, doc: &crate::document_store::Document,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Upsert a document by id.
    #[cfg(not(target_arch = "wasm32"))]
    fn put(
        &self, store: &str, doc: &crate::document_store::Document,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Delete a document by id. Returns whether a document was removed.
    #[cfg(not(target_arch = "wasm32"))]
    fn delete(&self, store: &str, id: &str) -> impl Future<Output = Result<bool>> + Send;

    /// Query documents in a collection.
    #[cfg(not(target_arch = "wasm32"))]
    fn query(
        &self, store: &str, options: crate::document_store::QueryOptions,
    ) -> impl Future<Output = Result<crate::document_store::QueryResult>> + Send;

    /// Fetch a document by id.
    #[cfg(target_arch = "wasm32")]
    fn get(
        &self, store: &str, id: &str,
    ) -> impl Future<Output = Result<Option<crate::document_store::Document>>> + Send {
        async move { omnia_wasi_jsondb::store::get(store, id).await }
    }

    /// Insert a new document (fails if the id already exists).
    #[cfg(target_arch = "wasm32")]
    fn insert(
        &self, store: &str, doc: &crate::document_store::Document,
    ) -> impl Future<Output = Result<()>> + Send {
        async move { omnia_wasi_jsondb::store::insert(store, doc).await }
    }

    /// Upsert a document by id.
    #[cfg(target_arch = "wasm32")]
    fn put(
        &self, store: &str, doc: &crate::document_store::Document,
    ) -> impl Future<Output = Result<()>> + Send {
        async move { omnia_wasi_jsondb::store::put(store, doc).await }
    }

    /// Delete a document by id. Returns whether a document was removed.
    #[cfg(target_arch = "wasm32")]
    fn delete(&self, store: &str, id: &str) -> impl Future<Output = Result<bool>> + Send {
        async move { omnia_wasi_jsondb::store::delete(store, id).await }
    }

    /// Query documents in a collection.
    #[cfg(target_arch = "wasm32")]
    fn query(
        &self, store: &str, options: crate::document_store::QueryOptions,
    ) -> impl Future<Output = Result<crate::document_store::QueryResult>> + Send {
        async move { omnia_wasi_jsondb::store::query(store, options).await }
    }
}

/// The `Broadcast` trait defines behavior for sending events to WebSocket
/// or other broadcast channels.
pub trait Broadcast: Send + Sync {
    /// Send an event to connected WebSocket clients.
    #[cfg(not(target_arch = "wasm32"))]
    fn send(
        &self, name: &str, data: &[u8], sockets: Option<Vec<String>>,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Send an event to connected WebSocket clients.
    #[cfg(target_arch = "wasm32")]
    fn send(
        &self, name: &str, data: &[u8], sockets: Option<Vec<String>>,
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            let client = omnia_wasi_websocket::types::Client::connect(name.to_string())
                .await
                .map_err(|e| anyhow!("connecting to websocket: {e}"))?;
            let event = omnia_wasi_websocket::types::Event::new(data);
            omnia_wasi_websocket::client::send(&client, event, sockets)
                .await
                .map_err(|e| anyhow!("sending websocket event: {e}"))
        }
    }
}

/// Metadata for a blobstore container.
///
/// Mirrors the `container-metadata` record from `wasi:blobstore/types`.
#[derive(Clone, Debug)]
pub struct ContainerMetadata {
    /// The container's name.
    pub name: String,
    /// Seconds since Unix epoch when the container was created.
    pub created_at: u64,
}

/// Metadata for an object in a blobstore container.
///
/// Mirrors the `object-metadata` record from `wasi:blobstore/types`.
#[derive(Clone, Debug)]
pub struct ObjectMetadata {
    /// The object's name.
    pub name: String,
    /// The object's parent container.
    pub container: String,
    /// Seconds since Unix epoch when the object was created.
    pub created_at: u64,
    /// Size of the object in bytes.
    pub size: u64,
}

/// Binary large object storage (WASI Blobstore).
///
/// Default WASM implementations delegate to `wasi:blobstore` via
/// `omnia-wasi-blobstore`.
pub trait BlobStore: Send + Sync {
    // ------------------------------------------------------------------
    // Object operations
    // ------------------------------------------------------------------

    /// Retrieve an object's data from a container.
    #[cfg(not(target_arch = "wasm32"))]
    fn get(
        &self, container: &str, name: &str,
    ) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;

    /// Store an object in a container.
    #[cfg(not(target_arch = "wasm32"))]
    fn put(
        &self, container: &str, name: &str, data: &[u8],
    ) -> impl Future<Output = Result<()>> + Send;

    /// Delete an object from a container.
    #[cfg(not(target_arch = "wasm32"))]
    fn delete(&self, container: &str, name: &str) -> impl Future<Output = Result<()>> + Send;

    /// Check whether an object exists in a container.
    #[cfg(not(target_arch = "wasm32"))]
    fn has(&self, container: &str, name: &str) -> impl Future<Output = Result<bool>> + Send;

    /// List all object names in a container.
    #[cfg(not(target_arch = "wasm32"))]
    fn list(&self, container: &str) -> impl Future<Output = Result<Vec<String>>> + Send;

    /// Retrieve a byte range of an object's data.
    ///
    /// Both `start` and `end` offsets are inclusive.
    #[cfg(not(target_arch = "wasm32"))]
    fn get_range(
        &self, container: &str, name: &str, start: u64, end: u64,
    ) -> impl Future<Output = Result<Vec<u8>>> + Send;

    /// Return metadata for an object.
    #[cfg(not(target_arch = "wasm32"))]
    fn object_info(
        &self, container: &str, name: &str,
    ) -> impl Future<Output = Result<ObjectMetadata>> + Send;

    /// Delete multiple objects from a container.
    #[cfg(not(target_arch = "wasm32"))]
    fn delete_objects(
        &self, container: &str, names: &[String],
    ) -> impl Future<Output = Result<()>> + Send;

    /// Remove all objects from a container, leaving it empty.
    #[cfg(not(target_arch = "wasm32"))]
    fn clear(&self, container: &str) -> impl Future<Output = Result<()>> + Send;

    // ------------------------------------------------------------------
    // Container management
    // ------------------------------------------------------------------

    /// Create a new empty container.
    #[cfg(not(target_arch = "wasm32"))]
    fn create_container(&self, name: &str) -> impl Future<Output = Result<()>> + Send;

    /// Delete a container and all objects within it.
    #[cfg(not(target_arch = "wasm32"))]
    fn delete_container(&self, name: &str) -> impl Future<Output = Result<()>> + Send;

    /// Check whether a container exists.
    #[cfg(not(target_arch = "wasm32"))]
    fn container_exists(&self, name: &str) -> impl Future<Output = Result<bool>> + Send;

    /// Return metadata for a container.
    #[cfg(not(target_arch = "wasm32"))]
    fn container_info(
        &self, container: &str,
    ) -> impl Future<Output = Result<ContainerMetadata>> + Send;

    // ------------------------------------------------------------------
    // Cross-container operations
    // ------------------------------------------------------------------

    /// Copy an object to the same or a different container.
    ///
    /// Overwrites the destination object if it already exists. Returns an
    /// error if the destination container does not exist.
    #[cfg(not(target_arch = "wasm32"))]
    fn copy_object(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Move or rename an object to the same or a different container.
    ///
    /// Overwrites the destination object if it already exists. Returns an
    /// error if the destination container does not exist.
    #[cfg(not(target_arch = "wasm32"))]
    fn move_object(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> impl Future<Output = Result<()>> + Send;

    // ------------------------------------------------------------------
    // WASM default implementations
    // ------------------------------------------------------------------

    /// Retrieve an object's data from a container.
    #[cfg(target_arch = "wasm32")]
    fn get(
        &self, container: &str, name: &str,
    ) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        use omnia_wasi_blobstore::blobstore;
        use omnia_wasi_blobstore::types::IncomingValue;

        async move {
            let ctr = blobstore::get_container(container.to_string())
                .await
                .map_err(|e| anyhow!("opening container: {e}"))?;
            if !ctr
                .has_object(name.to_string())
                .await
                .map_err(|e| anyhow!("checking object existence: {e}"))?
            {
                return Ok(None);
            }
            let incoming = ctr
                .get_data(name.to_string(), 0, u64::MAX)
                .await
                .map_err(|e| anyhow!("reading object: {e}"))?;
            let data = IncomingValue::incoming_value_consume_sync(incoming)
                .map_err(|e| anyhow!("consuming incoming value: {e}"))?;
            Ok(Some(data))
        }
    }

    /// Store an object in a container.
    #[cfg(target_arch = "wasm32")]
    fn put(
        &self, container: &str, name: &str, data: &[u8],
    ) -> impl Future<Output = Result<()>> + Send {
        use omnia_wasi_blobstore::blobstore;
        use omnia_wasi_blobstore::types::OutgoingValue;

        async move {
            let ctr = blobstore::get_container(container.to_string())
                .await
                .map_err(|e| anyhow!("opening container: {e}"))?;
            let outgoing = OutgoingValue::new_outgoing_value();
            {
                let body = outgoing
                    .outgoing_value_write_body()
                    .await
                    .map_err(|e| anyhow!("getting write body: {e}"))?;
                body.blocking_write_and_flush(data).map_err(|e| anyhow!("writing data: {e}"))?;
            }
            ctr.write_data(name.to_string(), &outgoing)
                .await
                .map_err(|e| anyhow!("writing object: {e}"))?;
            OutgoingValue::finish(outgoing).map_err(|e| anyhow!("finishing write: {e}"))?;
            Ok(())
        }
    }

    /// Delete an object from a container.
    #[cfg(target_arch = "wasm32")]
    fn delete(&self, container: &str, name: &str) -> impl Future<Output = Result<()>> + Send {
        use omnia_wasi_blobstore::blobstore;

        async move {
            let ctr = blobstore::get_container(container.to_string())
                .await
                .map_err(|e| anyhow!("opening container: {e}"))?;
            ctr.delete_object(name.to_string()).await.map_err(|e| anyhow!("deleting object: {e}"))
        }
    }

    /// Check whether an object exists in a container.
    #[cfg(target_arch = "wasm32")]
    fn has(&self, container: &str, name: &str) -> impl Future<Output = Result<bool>> + Send {
        use omnia_wasi_blobstore::blobstore;

        async move {
            let ctr = blobstore::get_container(container.to_string())
                .await
                .map_err(|e| anyhow!("opening container: {e}"))?;
            ctr.has_object(name.to_string())
                .await
                .map_err(|e| anyhow!("checking object existence: {e}"))
        }
    }

    /// List all object names in a container.
    #[cfg(target_arch = "wasm32")]
    fn list(&self, container: &str) -> impl Future<Output = Result<Vec<String>>> + Send {
        use omnia_wasi_blobstore::blobstore;

        async move {
            let ctr = blobstore::get_container(container.to_string())
                .await
                .map_err(|e| anyhow!("opening container: {e}"))?;
            let stream = ctr.list_objects().await.map_err(|e| anyhow!("listing objects: {e}"))?;
            let mut names = Vec::new();
            loop {
                let (batch, done) = stream
                    .read_stream_object_names(100)
                    .await
                    .map_err(|e| anyhow!("reading object names: {e}"))?;
                names.extend(batch);
                if done {
                    break;
                }
            }
            Ok(names)
        }
    }

    /// Retrieve a byte range of an object's data.
    ///
    /// Both `start` and `end` offsets are inclusive.
    #[cfg(target_arch = "wasm32")]
    fn get_range(
        &self, container: &str, name: &str, start: u64, end: u64,
    ) -> impl Future<Output = Result<Vec<u8>>> + Send {
        use omnia_wasi_blobstore::blobstore;
        use omnia_wasi_blobstore::types::IncomingValue;

        async move {
            let ctr = blobstore::get_container(container.to_string())
                .await
                .map_err(|e| anyhow!("opening container: {e}"))?;
            let incoming = ctr
                .get_data(name.to_string(), start, end)
                .await
                .map_err(|e| anyhow!("reading object range: {e}"))?;
            let data = IncomingValue::incoming_value_consume_sync(incoming)
                .map_err(|e| anyhow!("consuming incoming value: {e}"))?;
            Ok(data)
        }
    }

    /// Return metadata for an object.
    #[cfg(target_arch = "wasm32")]
    fn object_info(
        &self, container: &str, name: &str,
    ) -> impl Future<Output = Result<ObjectMetadata>> + Send {
        use omnia_wasi_blobstore::blobstore;

        async move {
            let ctr = blobstore::get_container(container.to_string())
                .await
                .map_err(|e| anyhow!("opening container: {e}"))?;
            let info = ctr
                .object_info(name.to_string())
                .await
                .map_err(|e| anyhow!("getting object info: {e}"))?;
            Ok(ObjectMetadata {
                name: info.name,
                container: info.container,
                created_at: info.created_at,
                size: info.size,
            })
        }
    }

    /// Delete multiple objects from a container.
    #[cfg(target_arch = "wasm32")]
    fn delete_objects(
        &self, container: &str, names: &[String],
    ) -> impl Future<Output = Result<()>> + Send {
        use omnia_wasi_blobstore::blobstore;

        let names = names.to_vec();
        async move {
            let ctr = blobstore::get_container(container.to_string())
                .await
                .map_err(|e| anyhow!("opening container: {e}"))?;
            ctr.delete_objects(&names).await.map_err(|e| anyhow!("deleting objects: {e}"))
        }
    }

    /// Remove all objects from a container, leaving it empty.
    #[cfg(target_arch = "wasm32")]
    fn clear(&self, container: &str) -> impl Future<Output = Result<()>> + Send {
        use omnia_wasi_blobstore::blobstore;

        async move {
            let ctr = blobstore::get_container(container.to_string())
                .await
                .map_err(|e| anyhow!("opening container: {e}"))?;
            ctr.clear().await.map_err(|e| anyhow!("clearing container: {e}"))
        }
    }

    /// Create a new empty container.
    #[cfg(target_arch = "wasm32")]
    fn create_container(&self, name: &str) -> impl Future<Output = Result<()>> + Send {
        use omnia_wasi_blobstore::blobstore;

        async move {
            let _container = blobstore::create_container(name.to_string())
                .await
                .map_err(|e| anyhow!("creating container: {e}"))?;
            Ok(())
        }
    }

    /// Delete a container and all objects within it.
    #[cfg(target_arch = "wasm32")]
    fn delete_container(&self, name: &str) -> impl Future<Output = Result<()>> + Send {
        use omnia_wasi_blobstore::blobstore;

        async move {
            blobstore::delete_container(name.to_string())
                .await
                .map_err(|e| anyhow!("deleting container: {e}"))
        }
    }

    /// Check whether a container exists.
    #[cfg(target_arch = "wasm32")]
    fn container_exists(&self, name: &str) -> impl Future<Output = Result<bool>> + Send {
        use omnia_wasi_blobstore::blobstore;

        async move {
            blobstore::container_exists(name.to_string())
                .await
                .map_err(|e| anyhow!("checking container existence: {e}"))
        }
    }

    /// Return metadata for a container.
    #[cfg(target_arch = "wasm32")]
    fn container_info(
        &self, container: &str,
    ) -> impl Future<Output = Result<ContainerMetadata>> + Send {
        use omnia_wasi_blobstore::blobstore;

        async move {
            let ctr = blobstore::get_container(container.to_string())
                .await
                .map_err(|e| anyhow!("opening container: {e}"))?;
            let info = ctr.info().map_err(|e| anyhow!("getting container info: {e}"))?;
            Ok(ContainerMetadata {
                name: info.name,
                created_at: info.created_at,
            })
        }
    }

    /// Copy an object to the same or a different container.
    ///
    /// Overwrites the destination object if it already exists. Returns an
    /// error if the destination container does not exist.
    #[cfg(target_arch = "wasm32")]
    fn copy_object(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        use omnia_wasi_blobstore::blobstore;
        use omnia_wasi_blobstore::types::ObjectId;

        async move {
            let src = ObjectId {
                container: src_container.to_string(),
                object: src_name.to_string(),
            };
            let dest = ObjectId {
                container: dest_container.to_string(),
                object: dest_name.to_string(),
            };
            blobstore::copy_object(&src, &dest).await.map_err(|e| anyhow!("copying object: {e}"))
        }
    }

    /// Move or rename an object to the same or a different container.
    ///
    /// Overwrites the destination object if it already exists. Returns an
    /// error if the destination container does not exist.
    #[cfg(target_arch = "wasm32")]
    fn move_object(
        &self, src_container: &str, src_name: &str, dest_container: &str, dest_name: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        use omnia_wasi_blobstore::blobstore;
        use omnia_wasi_blobstore::types::ObjectId;

        async move {
            let src = ObjectId {
                container: src_container.to_string(),
                object: src_name.to_string(),
            };
            let dest = ObjectId {
                container: dest_container.to_string(),
                object: dest_name.to_string(),
            };
            blobstore::move_object(&src, &dest).await.map_err(|e| anyhow!("moving object: {e}"))
        }
    }
}
