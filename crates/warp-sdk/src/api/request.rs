//! Request routing and handler traits.
//!
//! This module contains:
//! - [`Handler`]: implemented by request types to produce a [`Reply`]
//! - [`RequestHandler`]: a small request builder / router (supports `.headers(...)`)
//!
//! The main entry point is usually [`crate::Client`], re-exported from the
//! top-level `api` module.

use std::error::Error;
use std::fmt::Debug;
use std::future::{Future, IntoFuture};
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use http::HeaderMap;

use crate::api::reply::Reply;
use crate::api::{Body, Client, Provider};

/// Trait to provide a common interface for request handling.
pub trait Handler<P: Provider>: Decode {
    /// The output type of the handler.
    type Output: Body;

    /// The error type returned by the handler.
    type Error: Error + Send + Sync;

    /// Initialize a request handler from request bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the message cannot be decoded.
    fn handler(encoded: Self::Encoded) -> Result<PreHandler<Self, P>, Self::Error>
    where
        Self::Error: From<Self::DecodeError>,
    {
        let request = Self::decode(encoded)?;
        Ok(PreHandler::new(request))
    }

    /// Implemented by the request handler to process the request.
    fn handle(
        self, ctx: Context<P>,
    ) -> impl Future<Output = Result<Reply<Self::Output>, Self::Error>> + Send;
}

/// Trait for messages that can be decoded and built into handlers
pub trait Decode: Sized {
    type Encoded;
    type DecodeError: Error;

    /// Decode the message into a request handler.
    ///
    /// # Errors
    ///
    /// Returns an error if the message cannot be decoded.
    fn decode(encoded: Self::Encoded) -> Result<Self, Self::DecodeError>;
}

pub struct PreHandler<R: Handler<P>, P: Provider> {
    request: R,
    provider: PhantomData<P>,
}

impl<R: Handler<P>, P: Provider> PreHandler<R, P> {
    pub const fn new(request: R) -> Self {
        Self {
            request,
            provider: PhantomData,
        }
    }

    pub fn provider(self, provider: P) -> RequestHandler<R, P> {
        RequestHandler {
            request: self.request,
            headers: HeaderMap::default(),
            provider: Arc::new(provider),
            owner: Arc::<str>::from(""),
        }
    }
}

/// Request router.
///
/// The router is used to route a request to the appropriate handler with the
/// owner and headers set.
/// ```
#[derive(Debug)]
pub struct RequestHandler<R, P> {
    request: R,
    headers: HeaderMap<String>,

    /// The owning tenant/namespace.
    owner: Arc<str>,

    /// The provider to use while handling of the request.
    provider: Arc<P>,
}

pub struct NoProvider;
pub struct NoRequest;

impl Default for RequestHandler<NoRequest, NoProvider> {
    fn default() -> Self {
        Self {
            request: NoRequest,
            headers: HeaderMap::default(),
            provider: Arc::new(NoProvider),
            owner: Arc::<str>::from(""),
        }
    }
}
impl RequestHandler<NoRequest, NoProvider> {
    /// Create a new `RequestHandler` with no provider.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl<R> RequestHandler<R, NoProvider> {
    /// Set the provider (transitions typestate)
    pub fn provider<P: Provider>(self, provider: P) -> RequestHandler<R, P> {
        RequestHandler {
            request: self.request,
            headers: self.headers,
            provider: Arc::new(provider),
            owner: self.owner,
        }
    }
}

impl<P: Provider> RequestHandler<NoRequest, P> {
    /// Set the provider (transitions typestate)
    pub fn request<R: Handler<P>>(self, request: R) -> RequestHandler<R, P> {
        RequestHandler {
            request,
            headers: HeaderMap::default(),
            provider: self.provider,
            owner: self.owner,
        }
    }
}

impl<R, P> RequestHandler<R, P>
where
    R: Handler<P>,
    P: Provider,
{
    // Internal constructor for creating a `RequestHandler` from a `Client`.
    pub(crate) fn from_client(client: &Client<P>, request: R) -> Self {
        Self {
            request,
            headers: HeaderMap::default(),
            owner: Arc::clone(&client.owner),
            provider: Arc::clone(&client.provider),
        }
    }

    // pub fn with_provider(mut self, provider: P) -> Self {
    //     self.provider = Arc::new(provider);
    //     self
    // }

    /// Set the owner
    #[must_use]
    pub fn owner(mut self, owner: impl Into<String>) -> Self {
        self.owner = Arc::<str>::from(owner.into());
        self
    }

    /// Set request headers.
    #[must_use]
    pub fn headers(mut self, headers: HeaderMap<String>) -> Self {
        self.headers = headers;
        self
    }

    /// Handle the request by routing it to the appropriate handler.
    ///
    /// # Constraints
    ///
    /// This method requires that `R` implements [`Handler<P>`].
    /// If you see an error about missing trait implementations, ensure your request type
    /// has the appropriate handler implementation.
    ///
    /// # Errors
    ///
    /// Returns the error from the underlying handler on failure.
    #[inline]
    pub async fn handle(self) -> Result<Reply<R::Output>, R::Error> {
        let ctx = Context {
            owner: &self.owner,
            provider: &*self.provider,
            headers: &self.headers,
        };
        self.request.handle(ctx).await
    }
}

// Implement [`IntoFuture`] so that the request can be awaited directly (without
// needing to call the `handle` method).
impl<R, P> IntoFuture for RequestHandler<R, P>
where
    P: Provider + 'static,
    R: Handler<P> + Send + 'static,
{
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send + 'static>>;
    type Output = Result<Reply<R::Output>, R::Error>;

    fn into_future(self) -> Self::IntoFuture
    where
        R::Output: Body,
        R::Error: Send,
    {
        Box::pin(self.handle())
    }
}

/// Request-scoped context passed to [`Handler::handle`].
///
/// Bundles common request inputs (owner, provider, headers) into a single
/// parameter, making handler signatures more ergonomic and easier to extend.
#[derive(Clone, Copy, Debug)]
pub struct Context<'a, P: Provider> {
    /// The owning tenant / namespace for the request.
    pub owner: &'a str,

    /// The provider implementation used to fulfill the request.
    pub provider: &'a P,

    /// Request headers (typed).
    pub headers: &'a HeaderMap<String>,
}
