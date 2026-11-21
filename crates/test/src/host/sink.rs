//! Messaging and WebSocket backends recording everything a guest sends, and
//! an identity backend answering fixed tokens.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures::future::{self, ready};
use futures::{FutureExt as _, stream};
use omnia_wasi_identity::{AccessToken, Identity, IdentityClient};
use omnia_wasi_messaging::{self as messaging, Handler, Message, MessagingClient};
use omnia_wasi_websocket::{self as websocket, Event, Events, WebSocketClient};

/// One WebSocket send a guest made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Broadcasted {
    /// The payload.
    pub data: Vec<u8>,
    /// The connections the send named, or `None` for every open connection.
    pub sockets: Option<Vec<String>>,
    /// The connections a broadcast left out.
    pub except: Vec<String>,
}

/// Records every message a guest sends and every WebSocket event it sends.
///
/// - [`sent`](Self::sent) lists each message with its topic. The host
///   carries a message's key and content type as its `key` and
///   `content-type` metadata.
/// - [`broadcasts`](Self::broadcasts) lists each WebSocket send.
///
/// A request that awaits a reply fails with `unsupported`, and nothing
/// arrives for a guest to consume: [`deliver`](super::deliver) drives one
/// delivery. Clones share one log.
///
/// [`Backends::messaging`](super::Backends::messaging) and
/// [`Backends::websocket`](super::Backends::websocket) serve it to a
/// deployment.
#[derive(Clone, Debug, Default)]
pub struct Sink {
    inner: Arc<Inner>,
}

impl Sink {
    /// Every message sent, in the order the host received them.
    #[must_use]
    pub fn sent(&self) -> Vec<Message> {
        lock(&self.inner.sent).clone()
    }

    /// Every WebSocket send, in the order the host received them.
    #[must_use]
    pub fn broadcasts(&self) -> Vec<Broadcasted> {
        lock(&self.inner.broadcasts).clone()
    }

    fn record(&self, broadcasted: Broadcasted) -> websocket::FutureResult<()> {
        lock(&self.inner.broadcasts).push(broadcasted);
        ready(Ok(())).boxed()
    }
}

#[derive(Debug, Default)]
struct Inner {
    sent: Mutex<Vec<Message>>,
    broadcasts: Mutex<Vec<Broadcasted>>,
}

impl MessagingClient for Sink {
    fn endpoint(&self) -> messaging::FutureResult<Arc<dyn messaging::Endpoint>> {
        let endpoint: Arc<dyn messaging::Endpoint> = Arc::new(self.clone());
        ready(Ok(endpoint)).boxed()
    }
}

impl messaging::Endpoint for Sink {
    // a consumer never ends on purpose, and nothing arrives at this one
    fn consume(&self, _handler: Arc<dyn Handler>) -> messaging::FutureResult<()> {
        future::pending().boxed()
    }

    fn send(&self, topic: String, mut message: Message) -> messaging::FutureResult<()> {
        message.topic = topic;
        lock(&self.inner.sent).push(message);
        ready(Ok(())).boxed()
    }

    fn request(
        &self, topic: String, _message: Message, _timeout: Option<Duration>,
    ) -> messaging::FutureResult<Message> {
        let refused =
            messaging::Error::Unsupported(format!("the sink answers no request: {topic}"));
        ready(Err(refused.into())).boxed()
    }
}

impl WebSocketClient for Sink {
    fn endpoint(&self) -> websocket::FutureResult<Arc<dyn websocket::Endpoint>> {
        let endpoint: Arc<dyn websocket::Endpoint> = Arc::new(self.clone());
        ready(Ok(endpoint)).boxed()
    }
}

impl websocket::Endpoint for Sink {
    fn events(&self) -> websocket::FutureResult<Events> {
        let events: Events = Box::pin(stream::pending());
        ready(Ok(events)).boxed()
    }

    fn send(&self, event: Event, sockets: Option<Vec<String>>) -> websocket::FutureResult<()> {
        self.record(Broadcasted {
            data: event.data,
            sockets,
            except: Vec::new(),
        })
    }

    fn broadcast(&self, event: Event, except: Vec<String>) -> websocket::FutureResult<()> {
        self.record(Broadcasted {
            data: event.data,
            sockets: None,
            except,
        })
    }
}

/// One token a guest asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asked {
    /// The identity the guest opened.
    pub identity: String,
    /// The scopes it asked the token for.
    pub scopes: Vec<String>,
}

/// An `omnia:identity` backend serving every identity, each with a fixed
/// token, and recording each token asked for.
///
/// The `default` identity's token is the one passed to [`new`](Self::new).
/// Another identity's token is that token, a colon, and the identity's name.
/// Every token lasts an hour. Clones share one record.
///
/// [`Backends::identity`](super::Backends::identity) serves it to a
/// deployment.
#[derive(Clone, Debug)]
pub struct FixedIdentity {
    token: Arc<str>,
    asked: Arc<Mutex<Vec<Asked>>>,
}

impl FixedIdentity {
    /// An identity backend whose `default` identity's token is `token`.
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: Arc::from(token.into()),
            asked: Arc::default(),
        }
    }

    /// Every token asked for, in the order asked.
    #[must_use]
    pub fn asked(&self) -> Vec<Asked> {
        lock(&self.asked).clone()
    }
}

impl IdentityClient for FixedIdentity {
    fn get_identity(&self, name: String) -> omnia_wasi_identity::FutureResult<Arc<dyn Identity>> {
        let identity: Arc<dyn Identity> = Arc::new(Named {
            name,
            fixed: self.clone(),
        });
        ready(Ok(identity)).boxed()
    }
}

#[derive(Debug)]
struct Named {
    name: String,
    fixed: FixedIdentity,
}

impl Identity for Named {
    fn get_token(&self, scopes: Vec<String>) -> omnia_wasi_identity::FutureResult<AccessToken> {
        lock(&self.fixed.asked).push(Asked {
            identity: self.name.clone(),
            scopes,
        });
        let token = match self.name.as_str() {
            "default" => self.fixed.token.to_string(),
            name => format!("{}:{name}", self.fixed.token),
        };
        let token = AccessToken {
            token,
            expires_in: 3600,
        };
        ready(Ok(token)).boxed()
    }
}

// a panicking assertion elsewhere must not hide what the run recorded
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
