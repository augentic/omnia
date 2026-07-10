//! Typed HTTP routing over application operations.

use std::any::TypeId;

use axum::Router as AxumRouter;
use axum::extract::{RawPathParams, RawQuery, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{self, MethodRouter};
use http::header::{CONTENT_TYPE, HeaderName};
use http::{HeaderMap, HeaderValue, Method, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::api::{Invocation, Invoker, Metadata, Operation, Provider};

const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");
const CORRELATION_ID: HeaderName = HeaderName::from_static("x-correlation-id");
const CAUSATION_ID: HeaderName = HeaderName::from_static("x-causation-id");

/// Projects one operation's result onto an HTTP response.
pub trait Projector<O, P>: Clone + Send + Sync + 'static
where
    O: Operation<P>,
    P: Provider,
{
    /// Project a successful operation output.
    fn output(&self, output: O::Output) -> Response;

    /// Project an operation error.
    fn error(&self, error: O::Error) -> Response;
}

/// Projects successful outputs as JSON and errors through [`HttpError`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Json;

impl<O, P> Projector<O, P> for Json
where
    O: Operation<P>,
    O::Output: Serialize,
    O::Error: Into<HttpError>,
    P: Provider,
{
    fn output(&self, output: O::Output) -> Response {
        match serde_json::to_vec(&output) {
            Ok(body) => (
                StatusCode::OK,
                [(CONTENT_TYPE, HeaderValue::from_static("application/json"))],
                body,
            )
                .into_response(),
            Err(error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("body encoding error: {error}"))
                    .into_response()
            }
        }
    }

    fn error(&self, error: O::Error) -> Response {
        Into::<HttpError>::into(error).into_response()
    }
}

/// An HTTP error response.
#[derive(Debug)]
pub struct HttpError {
    status: StatusCode,
    error: String,
    content_type: Option<HeaderValue>,
}

impl From<crate::Error> for HttpError {
    fn from(error: crate::Error) -> Self {
        if let Some(body) = error.json_body() {
            return Self {
                status: error.status(),
                error: serde_json::to_string(&body).unwrap_or_else(|_| error.to_string()),
                content_type: Some(HeaderValue::from_static("application/json")),
            };
        }

        Self {
            status: error.status(),
            error: error.to_string(),
            content_type: None,
        }
    }
}

impl From<anyhow::Error> for HttpError {
    fn from(error: anyhow::Error) -> Self {
        if error.downcast_ref::<crate::Error>().is_some() {
            let error: crate::Error = error.into();
            return Self::from(error);
        }

        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            error: format!("{error}, caused by: {}", error.root_cause()),
            content_type: None,
        }
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        match self.content_type {
            Some(content_type) => {
                let mut headers = HeaderMap::new();
                headers.insert(CONTENT_TYPE, content_type);
                (self.status, headers, self.error).into_response()
            }
            None => (self.status, self.error).into_response(),
        }
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
pub struct MethodRoute<P: Provider> {
    method: Method,
    operation: TypeId,
    inner: MethodRouter<Invoker<P>>,
}

/// A per-request, inventory-bearing wrapper over [`axum::Router`].
///
/// Construct one inside each WASI HTTP `handle` call with exactly one
/// provider-owning [`Invoker`]. Axum route-state clones share that invoker's
/// provider allocation; no guest state is retained across WASI instances.
/// Durable state belongs in host-side capabilities.
pub struct Router<P: Provider> {
    inner: AxumRouter<Invoker<P>>,
    invoker: Invoker<P>,
    inventory: Vec<RouteInfo>,
}

impl<P: Provider> Router<P> {
    /// Create an empty per-request router backed by one invoker.
    #[must_use]
    pub fn new(invoker: Invoker<P>) -> Self {
        Self {
            inner: AxumRouter::new(),
            invoker,
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
        self.inner.with_state(self.invoker)
    }
}

/// Consume a per-request router through the WASI HTTP export.
///
/// Omnia creates one component instance per HTTP request, so callers should
/// construct the router and its provider-owning invoker in the export's
/// `handle` method. Durable state belongs in host-side capabilities.
///
/// # Errors
///
/// Returns the WASI HTTP transport error.
#[cfg(target_arch = "wasm32")]
pub async fn serve<P: Provider>(
    router: Router<P>, request: wasip3::http::types::Request,
) -> Result<wasip3::http::types::Response, wasip3::http::types::ErrorCode> {
    omnia_wasi_http::serve(router.into_axum(), request).await
}

/// Create a GET route with the default JSON projector.
#[must_use]
pub fn get<O, P>() -> MethodRoute<P>
where
    O: Operation<P>,
    O::Input: DeserializeOwned,
    O::Output: Serialize,
    O::Error: Into<HttpError>,
    P: Provider,
{
    get_with::<O, P, Json>(Json)
}

/// Create a GET route with an explicit projector.
pub fn get_with<O, P, J>(projector: J) -> MethodRoute<P>
where
    O: Operation<P>,
    O::Input: DeserializeOwned,
    P: Provider,
    J: Projector<O, P>,
{
    MethodRoute {
        method: Method::GET,
        operation: TypeId::of::<O>(),
        inner: routing::get(
            |State(invoker): State<Invoker<P>>,
             params: RawPathParams,
             RawQuery(query): RawQuery,
             headers: HeaderMap| async move {
                let input = match query_input::<O::Input>(&params, query.as_deref()) {
                    Ok(input) => input,
                    Err(error) => return error.into_response(),
                };
                invoke::<O, P, J>(&invoker, headers, input, projector).await
            },
        ),
    }
}

/// Create a POST route with the default JSON projector.
#[must_use]
pub fn post<O, P>() -> MethodRoute<P>
where
    O: Operation<P>,
    O::Input: DeserializeOwned,
    O::Output: Serialize,
    O::Error: Into<HttpError>,
    P: Provider,
{
    post_with::<O, P, Json>(Json)
}

/// Create a POST route with an explicit projector.
pub fn post_with<O, P, J>(projector: J) -> MethodRoute<P>
where
    O: Operation<P>,
    O::Input: DeserializeOwned,
    P: Provider,
    J: Projector<O, P>,
{
    MethodRoute {
        method: Method::POST,
        operation: TypeId::of::<O>(),
        inner: routing::post(
            |State(invoker): State<Invoker<P>>,
             params: RawPathParams,
             headers: HeaderMap,
             body: axum::body::Bytes| async move {
                let input = match body_input::<O::Input>(&params, &body) {
                    Ok(input) => input,
                    Err(error) => return error.into_response(),
                };
                invoke::<O, P, J>(&invoker, headers, input, projector).await
            },
        ),
    }
}

async fn invoke<O, P, J>(
    invoker: &Invoker<P>, headers: HeaderMap, input: O::Input, projector: J,
) -> Response
where
    O: Operation<P>,
    P: Provider,
    J: Projector<O, P>,
{
    let request_id = header(&headers, REQUEST_ID);
    let metadata = Metadata {
        correlation_id: header(&headers, CORRELATION_ID).or_else(|| request_id.clone()),
        request_id,
        causation_id: header(&headers, CAUSATION_ID),
        deadline: None,
    };
    match invoker.invoke::<O>(Invocation::new(input).metadata(metadata)).await {
        Ok(output) => projector.output(output),
        Err(error) => projector.error(error),
    }
}

fn header(headers: &HeaderMap, name: HeaderName) -> Option<String> {
    headers.get(name).and_then(|value| value.to_str().ok()).map(str::to_owned)
}

fn invalid(description: String) -> HttpError {
    crate::Error::BadRequest {
        code: "invalid_request".to_string(),
        description,
    }
    .into()
}

fn query_input<T: DeserializeOwned>(
    params: &RawPathParams, query: Option<&str>,
) -> Result<T, HttpError> {
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

fn body_input<T: DeserializeOwned>(params: &RawPathParams, body: &[u8]) -> Result<T, HttpError> {
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
