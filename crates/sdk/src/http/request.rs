//! A builder for one outbound request, sent through [`fetch`](super::fetch).

use std::fmt::Display;
use std::time::Duration;

use bytes::Bytes;
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderName, HeaderValue, Method, Request, Response, Uri};
use serde::Serialize;
use serde::de::DeserializeOwned;
use wasip3::http::types::{RequestOptions, RequestOptionsError};
use wasip3::http_compat::RequestOptionsExtension;

use crate::{Error, Result};

/// Start a `GET` request to `uri`.
pub fn get<T>(uri: T) -> RequestBuilder
where
    T: TryInto<Uri>,
    <T as TryInto<Uri>>::Error: Into<http::Error>,
{
    RequestBuilder::new(Method::GET, uri)
}

/// Start a `POST` request to `uri`.
pub fn post<T>(uri: T) -> RequestBuilder
where
    T: TryInto<Uri>,
    <T as TryInto<Uri>>::Error: Into<http::Error>,
{
    RequestBuilder::new(Method::POST, uri)
}

/// Start a `PUT` request to `uri`.
pub fn put<T>(uri: T) -> RequestBuilder
where
    T: TryInto<Uri>,
    <T as TryInto<Uri>>::Error: Into<http::Error>,
{
    RequestBuilder::new(Method::PUT, uri)
}

/// Start a `PATCH` request to `uri`.
pub fn patch<T>(uri: T) -> RequestBuilder
where
    T: TryInto<Uri>,
    <T as TryInto<Uri>>::Error: Into<http::Error>,
{
    RequestBuilder::new(Method::PATCH, uri)
}

/// Start a `DELETE` request to `uri`.
pub fn delete<T>(uri: T) -> RequestBuilder
where
    T: TryInto<Uri>,
    <T as TryInto<Uri>>::Error: Into<http::Error>,
{
    RequestBuilder::new(Method::DELETE, uri)
}

/// One outbound request under construction.
///
/// A part that does not parse, such as a URI or a header value, fails the
/// request when it is built or sent, so a chain ends in one `?`.
///
/// # Examples
///
/// ```rust,no_run
/// use std::time::Duration;
///
/// use omnia_sdk::http;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Serialize)]
/// struct Order {
///     sku: String,
/// }
///
/// #[derive(Deserialize)]
/// struct Placed {
///     id: String,
/// }
///
/// # async fn place(token: &str, order: &Order) -> omnia_sdk::Result<()> {
/// let placed: Placed = http::post("https://orders.example/orders")
///     .bearer(token)
///     .json(order)
///     .timeout(Duration::from_secs(5))
///     .fetch_json()
///     .await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
#[must_use = "a request is sent only by `fetch` or `fetch_json`"]
pub struct RequestBuilder {
    request: http::request::Builder,
    body: Bytes,
    timeout: Option<Duration>,
    error: Option<Error>,
}

impl RequestBuilder {
    /// Start a request with `method` to `uri`.
    pub fn new<T>(method: Method, uri: T) -> Self
    where
        T: TryInto<Uri>,
        <T as TryInto<Uri>>::Error: Into<http::Error>,
    {
        Self {
            request: Request::builder().method(method).uri(uri),
            body: Bytes::new(),
            timeout: None,
            error: None,
        }
    }

    /// Append a header.
    pub fn header<K, V>(mut self, name: K, value: V) -> Self
    where
        K: TryInto<HeaderName>,
        <K as TryInto<HeaderName>>::Error: Into<http::Error>,
        V: TryInto<HeaderValue>,
        <V as TryInto<HeaderValue>>::Error: Into<http::Error>,
    {
        self.request = self.request.header(name, value);
        self
    }

    /// Authorize the request with `token` as a bearer credential.
    ///
    /// The `Authorization` value is marked sensitive, so a log of the
    /// request's headers redacts it.
    pub fn bearer(mut self, token: impl Display) -> Self {
        match HeaderValue::try_from(format!("Bearer {token}")) {
            Ok(mut value) => {
                value.set_sensitive(true);
                self.request = self.request.header(AUTHORIZATION, value);
            }
            Err(err) => self.fail(format!("the bearer token is not a header value: {err}")),
        }
        self
    }

    /// Send `body` as the request's JSON body.
    ///
    /// The `Content-Type` is `application/json` unless one is already set.
    pub fn json<T: Serialize + ?Sized>(mut self, body: &T) -> Self {
        match serde_json::to_vec(body) {
            Ok(bytes) => {
                self.body = Bytes::from(bytes);
                if let Some(headers) = self.request.headers_mut() {
                    headers
                        .entry(CONTENT_TYPE)
                        .or_insert(HeaderValue::from_static("application/json"));
                }
            }
            Err(err) => self.fail(format!("encoding request body: {err}")),
        }
        self
    }

    /// Send `body` as the request's body.
    pub fn body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = body.into();
        self
    }

    /// Bound each wait of the exchange by `timeout`.
    ///
    /// The host bounds the connect, the first byte of the response, and every
    /// gap between body bytes, each on its own. A wait that runs over fails
    /// the request as an upstream that does not answer fails it.
    pub const fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Build the request, to send it some other way, such as through an
    /// [`HttpCache`](crate::http::cache::HttpCache).
    ///
    /// # Errors
    ///
    /// - [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid` when a
    ///   part does not parse or the JSON body does not encode.
    /// - [`ServerError`](crate::ErrorKind::ServerError) with the code `unsupported` when
    ///   the host takes no timeout.
    pub fn build(self) -> Result<Request<Bytes>> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let mut request = self
            .request
            .body(self.body)
            .map_err(|err| Error::server_error("invalid", format!("building request: {err}")))?;
        if let Some(timeout) = self.timeout {
            request.extensions_mut().insert(RequestOptionsExtension(options(timeout)?));
        }
        Ok(request)
    }

    /// Send the request through [`fetch`](super::fetch).
    ///
    /// # Errors
    ///
    /// As [`build`](Self::build), then as [`fetch`](super::fetch).
    pub async fn fetch(self) -> Result<Response<Bytes>> {
        super::fetch(self.build()?).await
    }

    /// Send the request through [`fetch_json`](super::fetch_json).
    ///
    /// # Errors
    ///
    /// As [`build`](Self::build), then as [`fetch_json`](super::fetch_json).
    pub async fn fetch_json<O: DeserializeOwned>(self) -> Result<O> {
        super::fetch_json(self.build()?).await
    }

    // the first fault is the one the request fails with, and the inner
    // builder holds its own until `body`
    fn fail(&mut self, description: String) {
        if self.error.is_none() && self.request.headers_ref().is_some() {
            self.error = Some(Error::server_error("invalid", description));
        }
    }
}

fn options(timeout: Duration) -> Result<RequestOptions> {
    let nanos = u64::try_from(timeout.as_nanos()).unwrap_or(u64::MAX);
    let refused = |err: RequestOptionsError| {
        Error::server_error("unsupported", format!("setting a request timeout: {err}"))
    };
    let options = RequestOptions::new();
    options.set_connect_timeout(Some(nanos)).map_err(refused)?;
    options.set_first_byte_timeout(Some(nanos)).map_err(refused)?;
    options.set_between_bytes_timeout(Some(nanos)).map_err(refused)?;
    Ok(options)
}
