//! Typed HTTP routing over application operations.

use std::any::TypeId;
use std::fmt;

use axum::Router as AxumRouter;
use axum::extract::{RawPathParams, RawQuery, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{self, MethodRouter};
use http::header::CONTENT_TYPE;
use http::{HeaderMap, HeaderValue, Method, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::api::{Client, Handler, Metadata};

/// Result type for HTTP handlers.
pub type HttpResult<T, E = HttpError> = Result<T, E>;

/// Projects one operation's result onto an HTTP response.
pub trait Projector<H, P>: Clone + Send + Sync + 'static
where
    H: Handler<P>,
    P: Send + Sync + 'static,
{
    /// Project a successful operation output.
    fn output(&self, output: H::Output) -> Response;

    /// Project an operation error.
    fn error(&self, error: H::Error) -> Response;

    /// Project a transport input-decoding failure.
    ///
    /// The default renders the standard bad-request [`HttpError`].
    fn decode(&self, error: DecodeError) -> Response {
        HttpError::from(error).into_response()
    }
}

/// A request that could not be converted to operation input.
#[derive(Debug)]
pub struct DecodeError {
    description: String,
}

impl DecodeError {
    /// Describe why the request could not be decoded.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.description)
    }
}

impl std::error::Error for DecodeError {}

impl From<DecodeError> for HttpError {
    fn from(error: DecodeError) -> Self {
        crate::Error::BadRequest {
            code: "invalid_request".to_string(),
            description: error.description,
        }
        .into()
    }
}

/// Projects successful outputs as JSON and errors through [`HttpError`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Json;

impl<H, P> Projector<H, P> for Json
where
    H: Handler<P>,
    H::Output: Serialize,
    H::Error: Into<HttpError>,
    P: Send + Sync + 'static,
{
    fn output(&self, output: H::Output) -> Response {
        match serde_json::to_vec(&output) {
            Ok(body) => (
                StatusCode::OK,
                [(CONTENT_TYPE, HeaderValue::from_static("application/json"))],
                body,
            )
                .into_response(),
            Err(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(CONTENT_TYPE, HeaderValue::from_static("application/json"))],
                serde_json::json!({
                    "error": "encoding",
                    "message": format!("body encoding error: {error}"),
                })
                .to_string(),
            )
                .into_response(),
        }
    }

    fn error(&self, error: H::Error) -> Response {
        Into::<HttpError>::into(error).into_response()
    }
}

/// An HTTP error response.
#[derive(Debug)]
pub struct HttpError {
    status: StatusCode,
    error: String,
}

impl HttpError {
    /// Create an HTTP error.
    #[must_use]
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            error: message.into(),
        }
    }
}

impl From<crate::Error> for HttpError {
    fn from(error: crate::Error) -> Self {
        Self::new(error.status(), error.to_string())
    }
}

impl From<anyhow::Error> for HttpError {
    fn from(error: anyhow::Error) -> Self {
        if error.downcast_ref::<crate::Error>().is_some() {
            let error: crate::Error = error.into();
            return Self::from(error);
        }

        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{error}, caused by: {}", error.root_cause()),
        )
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        (self.status, self.error).into_response()
    }
}

/// Read-only metadata for one registered HTTP route.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteInfo {
    method: Method,
    path: String,
    operation: TypeId,
}

impl RouteInfo {
    /// Return the registered HTTP method.
    #[must_use]
    pub const fn method(&self) -> &Method {
        &self.method
    }

    /// Return the registered path.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Return the process-local operation type identity.
    #[must_use]
    pub const fn operation(&self) -> TypeId {
        self.operation
    }
}

/// A typed HTTP method route awaiting a path.
pub struct MethodRoute<P: Send + Sync + 'static> {
    method: Method,
    operation: TypeId,
    inner: MethodRouter<Client<P>>,
}

/// A per-request, inventory-bearing wrapper over [`axum::Router`].
///
/// Construct one inside each WASI HTTP `handle` call with exactly one
/// provider-owning [`Client`]. Axum route-state clones share that client's
/// provider allocation; no guest state is retained across WASI instances.
/// Durable state belongs in host-side capabilities.
pub struct Router<P: Send + Sync + 'static> {
    inner: AxumRouter<Client<P>>,
    client: Client<P>,
    inventory: Vec<RouteInfo>,
}

impl<P: Send + Sync + 'static> Router<P> {
    /// Create an empty per-request router backed by one client.
    #[must_use]
    pub fn new(client: Client<P>) -> Self {
        Self {
            inner: AxumRouter::new(),
            client,
            inventory: Vec::new(),
        }
    }

    /// Register one typed method route.
    #[must_use]
    pub fn route(mut self, path: &str, route: MethodRoute<P>) -> Self {
        self.inventory.push(RouteInfo {
            method: route.method,
            path: path.to_owned(),
            operation: route.operation,
        });
        self.inner = self.inner.route(path, route.inner);
        self
    }

    /// Return registered routes in registration order.
    #[must_use]
    pub fn inventory(&self) -> &[RouteInfo] {
        &self.inventory
    }

    /// Finish the router for Axum or a WASI HTTP adapter.
    pub fn into_axum(self) -> AxumRouter {
        self.inner.with_state(self.client)
    }
}

/// Consume a per-request router through the WASI HTTP export.
///
/// Omnia creates one component instance per HTTP request, so callers should
/// construct the router and its provider-owning client in the export's
/// `handle` method. Durable state belongs in host-side capabilities.
///
/// # Errors
///
/// Returns the WASI HTTP transport error.
#[cfg(target_arch = "wasm32")]
pub async fn serve<P: Send + Sync + 'static>(
    router: Router<P>, request: wasip3::http::types::Request,
) -> Result<wasip3::http::types::Response, wasip3::http::types::ErrorCode> {
    omnia_wasi_http::serve(router.into_axum(), request).await
}

/// Create a GET route with the default JSON projector.
#[must_use]
pub fn get<H, P>() -> MethodRoute<P>
where
    H: Handler<P> + DeserializeOwned + 'static,
    H::Output: Serialize,
    H::Error: Into<HttpError>,
    P: Send + Sync + 'static,
{
    get_with::<H, P, Json>(Json)
}

/// Create a GET route with an explicit projector.
pub fn get_with<H, P, J>(projector: J) -> MethodRoute<P>
where
    H: Handler<P> + DeserializeOwned + 'static,
    P: Send + Sync + 'static,
    J: Projector<H, P>,
{
    MethodRoute {
        method: Method::GET,
        operation: TypeId::of::<H>(),
        inner: routing::get(
            |State(client): State<Client<P>>,
             params: RawPathParams,
             RawQuery(query): RawQuery,
             headers: HeaderMap| async move {
                let input = query_input::<H>(&params, query.as_deref());
                invoke::<H, P, J>(&client, headers, input, projector).await
            },
        ),
    }
}

/// Create a POST route with the default JSON projector.
#[must_use]
pub fn post<H, P>() -> MethodRoute<P>
where
    H: Handler<P> + DeserializeOwned + 'static,
    H::Output: Serialize,
    H::Error: Into<HttpError>,
    P: Send + Sync + 'static,
{
    post_with::<H, P, Json>(Json)
}

/// Create a POST route with an explicit projector.
pub fn post_with<H, P, J>(projector: J) -> MethodRoute<P>
where
    H: Handler<P> + DeserializeOwned + 'static,
    P: Send + Sync + 'static,
    J: Projector<H, P>,
{
    MethodRoute {
        method: Method::POST,
        operation: TypeId::of::<H>(),
        inner: routing::post(
            |State(client): State<Client<P>>,
             params: RawPathParams,
             headers: HeaderMap,
             body: axum::body::Bytes| async move {
                let input = body_input::<H>(&params, &body);
                invoke::<H, P, J>(&client, headers, input, projector).await
            },
        ),
    }
}

async fn invoke<H, P, J>(
    client: &Client<P>, headers: HeaderMap, input: Result<H, DecodeError>, projector: J,
) -> Response
where
    H: Handler<P>,
    P: Send + Sync + 'static,
    J: Projector<H, P>,
{
    let input = match input {
        Ok(input) => input,
        Err(error) => return projector.decode(error),
    };
    let metadata = Metadata::from_lookup(|name| {
        headers.get(format!("x-{name}")).and_then(|value| value.to_str().ok()).map(str::to_owned)
    });
    match client.call(input, &metadata).await {
        Ok(output) => projector.output(output),
        Err(error) => projector.error(error),
    }
}

const fn invalid(description: String) -> DecodeError {
    DecodeError { description }
}

fn query_input<T: DeserializeOwned>(
    params: &RawPathParams, query: Option<&str>,
) -> Result<T, DecodeError> {
    let mut pairs: Vec<(String, String)> =
        params.iter().map(|(key, value)| (key.to_owned(), value.to_owned())).collect();
    if let Some(query) = query {
        let parsed: Vec<(String, String)> = serde_urlencoded::from_str(query)
            .map_err(|error| invalid(format!("malformed query string: {error}")))?;
        pairs.extend(parsed);
    }
    let encoded = serde_urlencoded::to_string(&pairs)
        .map_err(|error| invalid(format!("cannot encode request parameters: {error}")))?;
    serde_urlencoded::from_str(&encoded)
        .map_err(|error| invalid(format!("invalid request parameters: {error}")))
}

fn body_input<T: DeserializeOwned>(params: &RawPathParams, body: &[u8]) -> Result<T, DecodeError> {
    let mut value = if body.is_empty() {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_slice(body)
            .map_err(|error| invalid(format!("malformed JSON body: {error}")))?
    };
    let serde_json::Value::Object(object) = &mut value else {
        return Err(invalid("the request body must be a JSON object".to_string()));
    };
    for (key, param) in params {
        object.insert(key.to_owned(), serde_json::Value::String(param.to_owned()));
    }
    serde_json::from_value(value).map_err(|error| invalid(format!("invalid request body: {error}")))
}
