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
pub trait Handler<P: Provider>: TryFrom<Self::Input, Error = <Self as Handler<P>>::Error> {
    /// The raw input type of the handler.
    type Input;

    /// The output type of the handler.
    type Output: Body;

    /// The error type returned by the handler.
    type Error: Error + Send + Sync;

    /// Initialize a request handler from request bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the message cannot be decoded.
    fn handler(input: Self::Input) -> Result<PreHandler<Self, P>, <Self as Handler<P>>::Error> {
        let request = Self::try_from(input)?;
        Ok(PreHandler::new(request))
    }

    /// Implemented by the request handler to process the request.
    fn handle(
        self, ctx: Context<P>,
    ) -> impl Future<Output = Result<Reply<Self::Output>, <Self as Handler<P>>::Error>> + Send;
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

    pub fn provider(self, provider: P) -> RequestHandler<R, NoOwner, P> {
        RequestHandler {
            request: self.request,
            headers: HeaderMap::default(),
            provider: Arc::new(provider),
            owner: NoOwner,
        }
    }
}

/// Request router.
///
/// The router is used to route a request to the appropriate handler with the
/// owner and headers set.
/// ```
#[derive(Debug)]
pub struct RequestHandler<R, O, P> {
    request: R,
    headers: HeaderMap<String>,

    /// The owning tenant/namespace.
    owner: O,

    /// The provider to use while handling of the request.
    provider: Arc<P>,
}

pub struct NoOwner;
pub struct OwnerSet(Arc<str>);

pub struct NoProvider;
pub struct NoRequest;

impl Default for RequestHandler<NoRequest, NoOwner, NoProvider> {
    fn default() -> Self {
        Self {
            request: NoRequest,
            headers: HeaderMap::default(),
            owner: NoOwner,
            provider: Arc::new(NoProvider),
        }
    }
}
impl RequestHandler<NoRequest, NoOwner, NoProvider> {
    /// Create a new `RequestHandler` with no provider.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl<R, O> RequestHandler<R, O, NoProvider> {
    /// Set the provider (transitions typestate)
    pub fn provider<P: Provider>(self, provider: P) -> RequestHandler<R, O, P> {
        RequestHandler {
            request: self.request,
            headers: self.headers,
            owner: self.owner,
            provider: Arc::new(provider),
        }
    }
}

impl<O, P: Provider> RequestHandler<NoRequest, O, P> {
    /// Set the provider (transitions typestate)
    pub fn request<R: Handler<P>>(self, request: R) -> RequestHandler<R, O, P> {
        RequestHandler {
            request,
            headers: HeaderMap::default(),
            owner: self.owner,
            provider: self.provider,
        }
    }
}

impl<R, P> RequestHandler<R, NoOwner, P>
where
    R: Handler<P>,
    P: Provider,
{
    // Internal constructor for creating a `RequestHandler` from a `Client`.
    pub(crate) fn from_client(client: &Client<P>, request: R) -> RequestHandler<R, OwnerSet, P> {
        RequestHandler {
            request,
            headers: HeaderMap::default(),
            owner: OwnerSet(Arc::clone(&client.owner)),
            provider: Arc::clone(&client.provider),
        }
    }

    /// Set the owner
    #[must_use]
    pub fn owner(self, owner: impl Into<String>) -> RequestHandler<R, OwnerSet, P> {
        RequestHandler {
            request: self.request,
            headers: self.headers,
            owner: OwnerSet(Arc::from(owner.into())),
            provider: self.provider,
        }
    }
}

impl<R, O, P> RequestHandler<R, O, P>
where
    R: Handler<P>,
    P: Provider,
{
    /// Set request headers.
    #[must_use]
    pub fn headers(mut self, headers: HeaderMap<String>) -> Self {
        self.headers = headers;
        self
    }
}

impl<R, P> RequestHandler<R, OwnerSet, P>
where
    R: Handler<P>,
    P: Provider,
{
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
    pub async fn handle(self) -> Result<Reply<R::Output>, <R as Handler<P>>::Error> {
        let ctx = Context {
            owner: &self.owner.0,
            provider: &*self.provider,
            headers: &self.headers,
        };
        self.request.handle(ctx).await
    }
}

// Implement [`IntoFuture`] so that the request can be awaited directly (without
// needing to call the `handle` method).
impl<R, P> IntoFuture for RequestHandler<R, OwnerSet, P>
where
    P: Provider + 'static,
    R: Handler<P> + Send + 'static,
{
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send + 'static>>;
    type Output = Result<Reply<R::Output>, <R as Handler<P>>::Error>;

    fn into_future(self) -> Self::IntoFuture
    where
        R::Output: Body,
        <R as Handler<P>>::Error: Send,
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
