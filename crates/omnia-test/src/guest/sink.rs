//! The trivial doubles: a recording `Publisher` + `WebSockets` sink, a map
//! `Config`, and a fixed-token `Identity`.

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use omnia_sdk::capability::Result;
use omnia_sdk::identity::{AccessToken, DEFAULT};
use omnia_sdk::messaging::Message;
use omnia_sdk::{Config, Identity, Publisher, WebSockets};

/// One WebSocket send the code under test made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Broadcasted {
    /// The payload.
    pub data: Vec<u8>,
    /// The targeted sockets; `None` is every connected socket.
    pub sockets: Option<Vec<String>>,
}

/// Records every published message and WebSocket send; clones share one log.
///
/// ```
/// use omnia_sdk::Publisher as _;
/// use omnia_sdk::messaging::Message;
/// use omnia_test::guest::Sink;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let sink = Sink::default();
/// sink.publish("orders", &Message::new(b"one").with_key("ann")).await.unwrap();
/// assert_eq!(sink.sent()[0].0, "orders");
/// assert_eq!(sink.sent()[0].1.key.as_deref(), Some("ann"));
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

    /// Every WebSocket send, in call order.
    ///
    /// # Panics
    ///
    /// Panics if a lock is poisoned.
    #[must_use]
    pub fn broadcasts(&self) -> Vec<Broadcasted> {
        self.inner.broadcasts.lock().expect("broadcasts lock").clone()
    }

    fn record(&self, data: &[u8], sockets: Option<Vec<String>>) {
        self.inner.broadcasts.lock().expect("broadcasts lock").push(Broadcasted {
            data: data.to_vec(),
            sockets,
        });
    }
}

impl Publisher for Sink {
    fn publish(&self, topic: &str, message: &Message) -> impl Future<Output = Result<()>> + Send {
        self.inner.sent.lock().expect("sent lock").push((topic.to_owned(), message.clone()));
        ready(Ok(()))
    }
}

impl WebSockets for Sink {
    fn broadcast(&self, data: &[u8]) -> impl Future<Output = Result<()>> + Send {
        self.record(data, None);
        ready(Ok(()))
    }

    fn send_to(&self, sockets: &[&str], data: &[u8]) -> impl Future<Output = Result<()>> + Send {
        self.record(data, Some(sockets.iter().map(|&socket| socket.to_owned()).collect()));
        ready(Ok(()))
    }
}

#[derive(Debug, Default)]
struct SinkInner {
    sent: Mutex<Vec<(String, Message)>>,
    broadcasts: Mutex<Vec<Broadcasted>>,
}

/// Configuration from a map; an unknown key reads as unset.
///
/// ```
/// use omnia_sdk::Config as _;
/// use omnia_test::guest::MapConfig;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let config = MapConfig::default().with([("region", "eu")]);
/// assert_eq!(config.get("region").await.unwrap().as_deref(), Some("eu"));
/// assert_eq!(config.get("zone").await.unwrap(), None);
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
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<String>>> + Send {
        ready(Ok(self.values.get(key).cloned()))
    }
}

/// An identity provider answering one fixed token per identity and recording
/// the identities asked for.
///
/// The [`DEFAULT`] identity's token is the one passed to [`new`](Self::new).
/// A [`named`](Identity::named) identity's token is that token, a colon,
/// and the identity's name. Every token lasts an hour.
///
/// ```
/// use omnia_sdk::Identity as _;
/// use omnia_test::guest::FixedIdentity;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let identity = FixedIdentity::new("tok");
/// assert_eq!(identity.token(&[]).await.unwrap().token, "tok");
/// assert_eq!(identity.named("svc").token(&[]).await.unwrap().token, "tok:svc");
/// assert_eq!(identity.asked(), ["default", "svc"]);
/// # });
/// ```
#[derive(Clone, Debug)]
pub struct FixedIdentity {
    token: String,
    name: Arc<str>,
    asked: Arc<Mutex<Vec<String>>>,
}

impl Default for FixedIdentity {
    fn default() -> Self {
        Self::new("")
    }
}

impl FixedIdentity {
    /// An identity provider whose [`DEFAULT`] identity's token is `token`.
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            name: Arc::from(DEFAULT),
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
    type Named = Self;

    fn token(&self, _scopes: &[&str]) -> impl Future<Output = Result<AccessToken>> + Send {
        self.asked.lock().expect("asked lock").push(self.name.to_string());
        let token = if &*self.name == DEFAULT {
            self.token.clone()
        } else {
            format!("{}:{}", self.token, self.name)
        };
        ready(Ok(AccessToken::new(token, Duration::from_secs(3600))))
    }

    fn named(&self, name: &str) -> Self {
        Self {
            token: self.token.clone(),
            name: Arc::from(name),
            asked: Arc::clone(&self.asked),
        }
    }
}
