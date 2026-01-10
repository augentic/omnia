//! # Request Handler API
//!
//! This module provides a type-safe API for handling requests using the
//! [typestate pattern](https://cliffle.com/blog/rust-typestate/).
//!
//! ## Core Types
//!
//! - [`Handler`]: Trait implemented by request types to process requests and produce [`Reply`]
//! - [`RequestHandler`]: Type-state builder for configuring and executing requests
//! - [`Context`]: Request-scoped context passed to handlers
//!
//! ## Typestate Pattern
//!
//! The type system ensures requests are properly configured at compile time,
//! preventing execution without required components (owner, provider, request).
//!
//! ### Valid State Transitions
//!
//! ```text
//! RequestHandler<NoRequest, NoOwner, NoProvider>
//!   → .owner("alice")  → RequestHandler<NoRequest, OwnerSet, NoProvider>
//!   → .provider(p)     → RequestHandler<NoRequest, OwnerSet, ProviderSet<P>>
//!   → .request(r)      → RequestHandler<RequestSet<R,P>, OwnerSet, ProviderSet<P>>
//!   → .handle().await  → Result<Reply<R::Output>, R::Error>
//! ```
//!
//! Methods can be called in any order (except `.handle()` which must be last).
//! The `.headers()` method can be called at any point in the chain.
//!
//! ## Usage Examples
//!
//! ### Example: From raw input (Recommended)
//!
//! The [`Handler::handler()`] method provides a convenient API for a 'oneshot'
//! builder pattern:
//!
//! ```rust,ignore
//! 
//! // Simple request - await directly (IntoFuture)
//! let response = MyRequest::handler(bytes)?
//!     .owner("alice")
//!     .provider(my_provider)
//!     .await?;
//!
//! // Or explicitly call `handle()`
//! let response = Request::handler(body.to_vec())?
//!        .provider(my_provider)
//!        .owner("owner")
//!        .handle()
//!        .await?;
//! ```
//!
//! ### Example: Using [`RequestHandler`] Directly
//!
//! ```rust,ignore
//! use yetti_sdk::api::{RequestHandler, Handler};
//!
//! // Manual construction with typestate safety
//! let response = RequestHandler::new()
//!     .owner("alice")
//!     .provider(my_provider)
//!     .request(my_request)
//!     .headers(my_headers)  // Optional
//!     .handle()
//!     .await?;
//! ```
//!
//! ### Example: Using Client
//!
//! The [`Client`] provides a more convenient API that sets owner and provider upfront:
//! ```rust,ignore
//! use yetti_sdk::Client;
//!
//! // Create a client with owner and provider
//! let client = Client::new("alice").provider(my_provider);
//!
//! // Simple request - await directly (IntoFuture)
//! let response = client.request(my_request).await?;
//!
//! // Request with headers
//! let response = client
//!     .request(my_request)
//!     .headers(my_headers)
//!     .await?;
//!
//! // Or explicitly call handle()
//! let response = client
//!     .request(my_request)
//!     .headers(my_headers)
//!     .handle()
//!     .await?;
//! ```
//!
//! ## Compile-Time Safety
//!
//! The typestate pattern ensures these errors are caught at compile time:
//! ```rust,compile_fail,ignore
//! 
//! // ❌ Cannot handle without all required fields
//! RequestHandler::new().handle().await?;  // Won't compile!
//!
//! // ❌ Cannot handle without provider
//! RequestHandler::new()
//!     .owner("alice")
//!     .request(my_request)
//!     .handle().await?;  // Won't compile!
//! ```
//!
//! Only when all required fields are set does `.handle()` become available.

use std::error::Error;
use std::fmt::Debug;
use std::future::{Future, IntoFuture};
use std::marker::PhantomData;
use std::pin::Pin;

use http::HeaderMap;

use crate::api::reply::Reply;
use crate::api::{Body, Client, Provider};

pub type Request<R, P> = RequestHandler<RequestSet<R, P>, NoOwner, NoProvider>;

/// Trait to provide a common interface for request handling.
pub trait Handler<P: Provider>: Sized {
    /// The raw input type of the handler.
    type Input;

    /// The output type of the handler.
    type Output: Body;

    /// The error type returned by the handler.
    type Error: Error + Send + Sync;

    /// Parse the input into a `[Handler]` instance.
    ///
    /// # Errors
    ///
    /// Returns an error if the input cannot be parsed.
    fn from_input(input: Self::Input) -> Result<Self, Self::Error>;

    /// Initialize a `[RequestHandler]` from raw request.
    ///
    /// # Errors
    ///
    /// Returns an error if the message cannot be decoded.
    fn handler(
        input: Self::Input,
    ) -> Result<RequestHandler<RequestSet<Self, P>, NoOwner, NoProvider>, Self::Error> {
        let request = Self::from_input(input)?;
        let handler = RequestHandler::new().request(request);
        Ok(handler)
    }

    /// Implemented by the request handler to process the request.
    fn handle(
        self, ctx: Context<P>,
    ) -> impl Future<Output = Result<Reply<Self::Output>, Self::Error>> + Send;
}

/// Marker type indicating no owner has been set yet.
pub struct NoOwner;

/// Marker type indicating the owner has been set.
pub struct OwnerSet<'a>(&'a str);

/// Marker type indicating no provider has been set yet.
pub struct NoProvider;

/// Marker type indicating the provider has been set.
pub struct ProviderSet<'a, P: Provider>(&'a P);

/// Marker type indicating no request has been set yet.
pub struct NoRequest;

/// Marker type wrapping a request that has been set.
pub struct RequestSet<R: Handler<P>, P: Provider>(R, PhantomData<P>);

/// Request router.
///
/// The router is used to route a request to the appropriate handler with the
/// owner and headers set.
/// ```
#[derive(Debug)]
pub struct RequestHandler<R, O, P> {
    request: R,
    headers: HeaderMap<String>,
    owner: O,
    provider: P,
}

impl Default for RequestHandler<NoRequest, NoOwner, NoProvider> {
    fn default() -> Self {
        Self {
            request: NoRequest,
            headers: HeaderMap::default(),
            owner: NoOwner,
            provider: NoProvider,
        }
    }
}

// ----------------------------------------------
// New builder
// ----------------------------------------------
impl RequestHandler<NoRequest, NoOwner, NoProvider> {
    /// Create a new (default) `RequestHandler`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // Internal constructor for creating a `RequestHandler` from a `Client`.
    pub(crate) fn from_client<R, P>(
        client: &Client<P>, request: R,
    ) -> RequestHandler<RequestSet<R, P>, OwnerSet<'_>, ProviderSet<'_, P>>
    where
        R: Handler<P>,
        P: Provider,
    {
        RequestHandler {
            request: RequestSet(request, PhantomData),
            headers: HeaderMap::default(),
            owner: OwnerSet(&client.owner),
            provider: ProviderSet(&client.provider),
        }
    }
}

// ----------------------------------------------
// Set Owner
// ----------------------------------------------
impl<R, P> RequestHandler<R, NoOwner, P> {
    /// Set the owner (transitions typestate).
    #[must_use]
    pub fn owner(self, owner: &str) -> RequestHandler<R, OwnerSet<'_>, P> {
        RequestHandler {
            request: self.request,
            headers: self.headers,
            owner: OwnerSet(owner),
            provider: self.provider,
        }
    }
}

// ----------------------------------------------
// Set Provider
// ----------------------------------------------
impl<R, O> RequestHandler<R, O, NoProvider> {
    /// Set the provider (transitions typestate).
    pub fn provider<P: Provider>(self, provider: &P) -> RequestHandler<R, O, ProviderSet<'_, P>> {
        RequestHandler {
            request: self.request,
            headers: self.headers,
            owner: self.owner,
            provider: ProviderSet(provider),
        }
    }
}

// ----------------------------------------------
// Set Request
// ----------------------------------------------
impl<O, P> RequestHandler<NoRequest, O, P> {
    /// Set the request (transitions typestate).
    pub fn request<R, Pr>(self, request: R) -> RequestHandler<RequestSet<R, Pr>, O, P>
    where
        R: Handler<Pr>,
        Pr: Provider,
    {
        RequestHandler {
            request: RequestSet(request, PhantomData),
            headers: self.headers,
            owner: self.owner,
            provider: self.provider,
        }
    }
}

// ----------------------------------------------
// Headers
// ----------------------------------------------
impl<R, O, P> RequestHandler<R, O, P> {
    /// Set request headers.
    #[must_use]
    pub fn headers(mut self, headers: HeaderMap<String>) -> Self {
        self.headers = headers;
        self
    }
}

// ----------------------------------------------
// Handle the request
// ----------------------------------------------
impl<R, P> RequestHandler<RequestSet<R, P>, OwnerSet<'_>, ProviderSet<'_, P>>
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
            owner: self.owner.0,
            provider: self.provider.0,
            headers: &self.headers,
        };
        self.request.0.handle(ctx).await
    }
}

// Implement [`IntoFuture`] so that the request can be awaited directly (without
// needing to call the `handle` method).
impl<'a, R, P> IntoFuture for RequestHandler<RequestSet<R, P>, OwnerSet<'a>, ProviderSet<'a, P>>
where
    P: Provider + 'a,
    R: Handler<P> + Send + 'a,
{
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send + 'a>>;
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
