//! The trivial doubles: a recording `Publish` + `Broadcast` sink, a map
//! `Config`, and a fixed-token `Identity`.

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use omnia_guest::{Broadcast, Config, Identity, Message, Publish};

/// One broadcast the code under test sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Broadcasted {
    /// The channel name.
    pub name: String,
    /// The payload.
    pub data: Vec<u8>,
    /// The targeted sockets; `None` is every subscriber.
    pub sockets: Option<Vec<String>>,
}

#[derive(Debug, Default)]
struct SinkInner {
    sent: Mutex<Vec<(String, Message)>>,
    broadcasts: Mutex<Vec<Broadcasted>>,
}

/// Records every published message and broadcast; clones share one log.
///
/// ```
/// use omnia_guest::{Message, Publish as _};
/// use omnia_test::guest::Sink;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let sink = Sink::default();
/// sink.send("orders", &Message::new(b"one")).await.unwrap();
/// assert_eq!(sink.sent()[0].0, "orders");
/// # });
/// ```
#[derive(Clone, Debug, Default)]
pub struct Sink {
    inner: Arc<SinkInner>,
}

impl Sink {
    /// Every `(topic, message)` published, in call order.
    ///
    /// # Panics
    ///
    /// Panics if a lock is poisoned.
    #[must_use]
    pub fn sent(&self) -> Vec<(String, Message)> {
        self.inner.sent.lock().expect("sent lock").clone()
    }

    /// Every broadcast, in call order.
    ///
    /// # Panics
    ///
    /// Panics if a lock is poisoned.
    #[must_use]
    pub fn broadcasts(&self) -> Vec<Broadcasted> {
        self.inner.broadcasts.lock().expect("broadcasts lock").clone()
    }
}

impl Publish for Sink {
    fn send(&self, topic: &str, message: &Message) -> impl Future<Output = Result<()>> + Send {
        self.inner.sent.lock().expect("sent lock").push((topic.to_owned(), message.clone()));
        ready(Ok(()))
    }
}

impl Broadcast for Sink {
    fn send(
        &self, name: &str, data: &[u8], sockets: Option<Vec<String>>,
    ) -> impl Future<Output = Result<()>> + Send {
        self.inner.broadcasts.lock().expect("broadcasts lock").push(Broadcasted {
            name: name.to_owned(),
            data: data.to_vec(),
            sockets,
        });
        ready(Ok(()))
    }
}

/// Configuration from a map; an unknown key is an error, as it is on the
/// runtime.
///
/// ```
/// use omnia_guest::Config as _;
/// use omnia_test::guest::MapConfig;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let config = MapConfig::default().with([("region", "eu")]);
/// assert_eq!(config.get("region").await.unwrap(), "eu");
/// assert!(config.get("zone").await.is_err());
/// # });
/// ```
#[derive(Clone, Debug, Default)]
pub struct MapConfig {
    values: BTreeMap<String, String>,
}

impl MapConfig {
    /// Adds the given `(key, value)` pairs.
    #[must_use]
    pub fn with<K: Into<String>, V: Into<String>>(
        mut self, pairs: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        self.values.extend(pairs.into_iter().map(|(k, v)| (k.into(), v.into())));
        self
    }
}

impl Config for MapConfig {
    fn get(&self, key: &str) -> impl Future<Output = Result<String>> + Send {
        ready(self.values.get(key).cloned().ok_or_else(|| anyhow!("config key `{key}` is not set")))
    }
}

/// An identity provider answering every request with one fixed token and
/// recording the identities asked for.
///
/// ```
/// use omnia_guest::Identity as _;
/// use omnia_test::guest::FixedIdentity;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let identity = FixedIdentity::new("tok");
/// assert_eq!(identity.access_token("svc".into()).await.unwrap(), "tok");
/// assert_eq!(identity.asked(), ["svc"]);
/// # });
/// ```
#[derive(Clone, Debug, Default)]
pub struct FixedIdentity {
    token: String,
    asked: Arc<Mutex<Vec<String>>>,
}

impl FixedIdentity {
    /// An identity whose every access token is `token`.
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            asked: Arc::default(),
        }
    }

    /// Every identity a token was requested for, in call order.
    ///
    /// # Panics
    ///
    /// Panics if a lock is poisoned.
    #[must_use]
    pub fn asked(&self) -> Vec<String> {
        self.asked.lock().expect("asked lock").clone()
    }
}

impl Identity for FixedIdentity {
    fn access_token(&self, identity: String) -> impl Future<Output = Result<String>> + Send {
        self.asked.lock().expect("asked lock").push(identity);
        ready(Ok(self.token.clone()))
    }
}
