//! Transport-neutral handler invocation and transport adapters.

use std::error::Error;
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::SystemTime;

/// Typed command routing over application operations.
pub mod command;
pub mod http;
/// Typed exact-topic messaging routing.
pub mod messaging;

pub use http::{HttpError, HttpResult};

/// Transport-neutral invocation metadata.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Metadata {
    /// Identifies this invocation at its transport boundary.
    pub request_id: Option<String>,

    /// Correlates work across transport and capability boundaries.
    pub correlation_id: Option<String>,

    /// Identifies the invocation that directly caused this work.
    pub causation_id: Option<String>,

    /// The latest instant at which the caller considers the work useful.
    pub deadline: Option<SystemTime>,
}

impl Metadata {
    /// Build metadata from a transport's named-value lookup.
    ///
    /// Names are the transport-neutral `request-id` / `correlation-id` /
    /// `causation-id`; the correlation id falls back to the request id.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let request_id = lookup("request-id");
        Self {
            correlation_id: lookup("correlation-id").or_else(|| request_id.clone()),
            request_id,
            causation_id: lookup("causation-id"),
            deadline: None,
        }
    }

    /// Mint metadata for a transport-initiated invocation.
    ///
    /// The freshly minted request id doubles as the correlation id.
    #[must_use]
    pub fn minted(request_id: String) -> Self {
        Self {
            correlation_id: Some(request_id.clone()),
            request_id: Some(request_id),
            causation_id: None,
            deadline: None,
        }
    }
}

/// Context shared with a handler call.
#[derive(Clone, Copy, Debug)]
pub struct Context<'a, P> {
    /// The owning tenant or namespace.
    pub owner: &'a str,

    /// The provider used to fulfil the call.
    pub provider: &'a P,

    /// Transport-neutral invocation metadata.
    pub metadata: &'a Metadata,
}

/// A stateless application handler whose `Self` is the input.
pub trait Handler<P>: Sized + Send {
    /// The typed handler output.
    type Output: Send;

    /// The handler failure.
    type Error: Error + Send + Sync + 'static;

    /// Execute the handler.
    ///
    /// # Errors
    ///
    /// Returns the handler's error.
    fn handle(
        self, context: Context<'_, P>,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send;
}

/// Provider-owning handler client.
///
/// Clones share one provider allocation. Transports define its lifetime; HTTP
/// constructs one client per WASI request and keeps durable state host-side.
pub struct Client<P> {
    owner: Arc<str>,
    provider: Arc<P>,
}

impl<P: Send + Sync + 'static> Client<P> {
    /// Create a client with one clone-shared provider allocation.
    pub fn new(owner: impl Into<String>, provider: P) -> Self {
        Self {
            owner: Arc::from(owner.into()),
            provider: Arc::new(provider),
        }
    }

    /// Return the owning tenant or namespace.
    #[must_use]
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Return the shared provider.
    #[must_use]
    pub fn provider(&self) -> &P {
        &self.provider
    }

    /// Invoke a handler with the given input and metadata.
    ///
    /// # Errors
    ///
    /// Returns the handler's error.
    pub async fn call<H: Handler<P>>(
        &self, input: H, metadata: &Metadata,
    ) -> Result<H::Output, H::Error> {
        let context = Context {
            owner: self.owner.as_ref(),
            provider: self.provider.as_ref(),
            metadata,
        };
        input.handle(context).await
    }
}

impl<P: Send + Sync + 'static> Clone for Client<P> {
    fn clone(&self) -> Self {
        Self {
            owner: Arc::clone(&self.owner),
            provider: Arc::clone(&self.provider),
        }
    }
}

impl<P: Send + Sync + 'static> fmt::Debug for Client<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client").field("owner", &self.owner).finish_non_exhaustive()
    }
}

/// The typed outcome supplied to a route projector.
#[derive(Debug)]
pub enum Outcome<T, O, D> {
    /// The operation completed successfully.
    Output(T),
    /// The operation returned its typed failure.
    Operation(O),
    /// The transport input could not be converted to operation input.
    Decode(D),
}
