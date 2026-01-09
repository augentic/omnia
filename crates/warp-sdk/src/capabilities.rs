//! # Provider
//!
//! Provider defines external data interfaces for the crate.

use std::any::Any;
use std::collections::HashMap;
use std::error::Error;

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use http::{Request, Response};
use http_body::Body;

/// The `Config` trait is used by implementers to provide configuration from
/// WASI-guest to dependent crates.
pub trait Config: Send + Sync {
    #[cfg(not(target_arch = "wasm32"))]
    fn get(&self, key: &str) -> impl Future<Output = Result<String>> + Send;

    /// Request configuration setting.
    #[cfg(target_arch = "wasm32")]
    fn get(&self, key: &str) -> impl Future<Output = Result<String>> + Send {
        async move {
            let config = wasi_config::store::get(key).context("getting configuration")?;
            config.ok_or_else(|| anyhow!("configuration not found"))
        }
    }
}

/// The `HttpRequest` trait defines the behavior for fetching data from a source.
pub trait HttpRequest: Send + Sync {
    #[cfg(not(target_arch = "wasm32"))]
    fn fetch<T>(&self, request: Request<T>)
    -> impl Future<Output = Result<Response<Bytes>>> + Send;

    /// Make outbound HTTP request.
    #[cfg(target_arch = "wasm32")]
    fn fetch<T>(&self, request: Request<T>) -> impl Future<Output = Result<Response<Bytes>>> + Send
    where
        T: Body + Any + Send,
        T::Data: Into<Vec<u8>>,
        T::Error: Into<Box<dyn Error + Send + Sync + 'static>>,
    {
        async move { wasi_http::handle(request).await }
    }
}

/// Message represents a message to be published.
#[derive(Clone, Debug)]
pub struct Message {
    pub payload: Vec<u8>,
    pub headers: HashMap<String, String>,
}

impl Message {
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
    #[cfg(not(target_arch = "wasm32"))]
    fn send(&self, topic: &str, message: &Message) -> impl Future<Output = Result<()>> + Send;

    /// Make outbound HTTP request.
    #[cfg(target_arch = "wasm32")]
    fn send(&self, topic: &str, message: &Message) -> impl Future<Output = Result<()>> + Send {
        use wasi_messaging::producer;
        use wasi_messaging::types::{self as wasi, Client};

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
    #[cfg(not(target_arch = "wasm32"))]
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;

    #[cfg(not(target_arch = "wasm32"))]
    fn set(
        &self, key: &str, value: &[u8], ttl_secs: Option<u64>,
    ) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;

    #[cfg(not(target_arch = "wasm32"))]
    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send;

    #[cfg(target_arch = "wasm32")]
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        async move {
            let bucket = wasi_keyvalue::cache::open("cache").await.context("opening cache")?;
            bucket.get(key).await.context("reading state from cache")
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn set(
        &self, key: &str, value: &[u8], ttl_secs: Option<u64>,
    ) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        async move {
            let bucket = wasi_keyvalue::cache::open("cache").await.context("opening cache")?;
            bucket.set(key, value, ttl_secs).await.context("reading state from cache")
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send {
        async move {
            let bucket = wasi_keyvalue::cache::open("cache").await.context("opening cache")?;
            bucket.delete(key).await.context("deleting entry from cache")
        }
    }
}

pub trait Identity: Send + Sync {
    #[cfg(not(target_arch = "wasm32"))]
    fn access_token(&self, identity: String) -> impl Future<Output = Result<String>> + Send;

    /// Get the unique identifier for the entity.
    #[cfg(target_arch = "wasm32")]
    fn access_token(&self, identity: String) -> impl Future<Output = Result<String>> + Send {
        use wasi_identity::credentials::get_identity;

        async move {
            let identity = wit_bindgen::block_on(get_identity(identity))?;
            let access_token =
                wit_bindgen::block_on(async move { identity.get_token(vec![]).await })?;
            Ok(access_token.token)
        }
    }
}
