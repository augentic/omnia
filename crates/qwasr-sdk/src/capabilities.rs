//! # Provider
//!
//! Provider defines external data interfaces for the crate.

use std::any::Any;
use std::collections::HashMap;
use std::error::Error;

use anyhow::Result;
#[cfg(target_arch = "wasm32")]
use anyhow::{Context, anyhow};
use bytes::Bytes;
use http::{Request, Response};
use http_body::Body;

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
            let config = qwasr_wasi_config::store::get(key).context("getting configuration")?;
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
        async move { qwasr_wasi_http::handle(request).await }
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
pub trait Publisher: Send + Sync {
    /// Publish (send) a message to a topic.
    #[cfg(not(target_arch = "wasm32"))]
    fn send(&self, topic: &str, message: &Message) -> impl Future<Output = Result<()>> + Send;

    /// Publish (send) a message to a topic.
    #[cfg(target_arch = "wasm32")]
    fn send(&self, topic: &str, message: &Message) -> impl Future<Output = Result<()>> + Send {
        use qwasr_wasi_messaging::producer;
        use qwasr_wasi_messaging::types::{self as wasi, Client};

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
                qwasr_wasi_keyvalue::cache::open("cache").await.context("opening cache")?;
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
                qwasr_wasi_keyvalue::cache::open("cache").await.context("opening cache")?;
            bucket.set(key, value, ttl_secs).await.context("reading state from cache")
        }
    }

    /// Delete a value from the state store.
    #[cfg(target_arch = "wasm32")]
    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send {
        async move {
            let bucket =
                qwasr_wasi_keyvalue::cache::open("cache").await.context("opening cache")?;
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
        use qwasr_wasi_identity::credentials::get_identity;

        async move {
            let identity = wit_bindgen::block_on(get_identity(identity))?;
            let access_token =
                wit_bindgen::block_on(async move { identity.get_token(vec![]).await })?;
            Ok(access_token.token)
        }
    }
}

/// Marker trait for types that provide relational database access.
///
/// Any type implementing this trait automatically gets `TableStore` functionality
/// via a blanket implementation in the `qwasr-wasi-sql` crate.
#[cfg(target_arch = "wasm32")]
pub use qwasr_wasi_sql::orm::TableStore;
