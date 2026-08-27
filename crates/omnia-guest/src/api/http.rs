//! Typed HTTP routing over application handlers.

use axum::Router as AxumRouter;
use axum::body::Bytes;
use axum::extract::{RawPathParams, RawQuery, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{self, MethodRouter};
use http::header::CONTENT_TYPE;
use http::{HeaderMap, HeaderValue, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;

pub use crate::api::DecodeError;
use crate::api::{Client, Handler, Metadata};

/// Result type for HTTP handlers.
pub type HttpResult<T, E = HttpError> = Result<T, E>;

impl From<DecodeError> for HttpError {
    fn from(error: DecodeError) -> Self {
        crate::Error::BadRequest {
            code: "invalid_request".to_string(),
            description: error.description().to_owned(),
        }
        .into()
    }
}

/// An HTTP error response.
#[derive(Debug)]
pub struct HttpError {
    status: StatusCode,
    error: String,
    body: Option<(HeaderValue, Vec<u8>)>,
}

impl HttpError {
    /// Create a plain-text HTTP error.
    #[must_use]
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            error: message.into(),
            body: None,
        }
    }

    /// Create an HTTP error carrying a preformatted wire body.
    #[must_use]
    pub const fn with_body(status: StatusCode, content_type: HeaderValue, body: Vec<u8>) -> Self {
        Self {
            status,
            error: String::new(),
            body: Some((content_type, body)),
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
        match self.body {
            Some((content_type, body)) => {
                (self.status, [(CONTENT_TYPE, content_type)], body).into_response()
            }
            None => (self.status, self.error).into_response(),
        }
    }
}

/// A typed HTTP method route awaiting a path.
pub struct MethodRoute<P: Send + Sync + 'static> {
    inner: MethodRouter<Client<P>>,
}

/// A per-request wrapper over [`axum::Router`].
///
/// Construct one inside each WASI HTTP `handle` call with exactly one
/// provider-owning [`Client`]. Axum route-state clones share that client's
/// provider allocation; no guest state is retained across WASI instances.
/// Durable state belongs in host-side capabilities.
pub struct Router<P: Send + Sync + 'static> {
    inner: AxumRouter<Client<P>>,
    client: Client<P>,
}

impl<P: Send + Sync + 'static> Router<P> {
    /// Create an empty per-request router backed by one client.
    #[must_use]
    pub fn new(client: Client<P>) -> Self {
        Self {
            inner: AxumRouter::new(),
            client,
        }
    }

    /// Register one typed method route.
    #[must_use]
    pub fn route(mut self, path: &str, route: MethodRoute<P>) -> Self {
        self.inner = self.inner.route(path, route.inner);
        self
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

/// A borrowed raw-request view handed to route decoders.
#[derive(Debug)]
pub struct RawRequest<'a> {
    /// Path parameters in path order.
    pub path_params: &'a [(String, String)],

    /// The raw query string, when present.
    pub query: Option<&'a str>,

    /// The request headers.
    pub headers: &'a HeaderMap,

    /// The raw request body (empty for typical GETs).
    pub body: &'a [u8],
}

/// Create a GET route with a custom decoder and encoder.
#[must_use]
pub fn get_with<H, P, D, E>(decode: D, encode: E) -> MethodRoute<P>
where
    H: Handler<P> + 'static,
    H::Error: Into<HttpError>,
    D: Fn(RawRequest<'_>) -> Result<H, DecodeError> + Clone + Send + Sync + 'static,
    E: Fn(H::Output) -> Response + Clone + Send + Sync + 'static,
    P: Send + Sync + 'static,
{
    MethodRoute {
        inner: routing::get(
            move |state: State<Client<P>>,
                  params: RawPathParams,
                  query: RawQuery,
                  headers: HeaderMap,
                  body: Bytes| {
                dispatch(decode, encode, state, params, query, headers, body)
            },
        ),
    }
}

/// Create a POST route with a custom decoder and encoder.
#[must_use]
pub fn post_with<H, P, D, E>(decode: D, encode: E) -> MethodRoute<P>
where
    H: Handler<P> + 'static,
    H::Error: Into<HttpError>,
    D: Fn(RawRequest<'_>) -> Result<H, DecodeError> + Clone + Send + Sync + 'static,
    E: Fn(H::Output) -> Response + Clone + Send + Sync + 'static,
    P: Send + Sync + 'static,
{
    MethodRoute {
        inner: routing::post(
            move |state: State<Client<P>>,
                  params: RawPathParams,
                  query: RawQuery,
                  headers: HeaderMap,
                  body: Bytes| {
                dispatch(decode, encode, state, params, query, headers, body)
            },
        ),
    }
}

/// Create a GET route decoding path and query parameters as JSON input.
#[must_use]
pub fn get<H, P>() -> MethodRoute<P>
where
    H: Handler<P> + DeserializeOwned + 'static,
    H::Output: Serialize,
    H::Error: Into<HttpError>,
    P: Send + Sync + 'static,
{
    get_with(|raw: RawRequest<'_>| query_input::<H>(raw.path_params, raw.query), json)
}

/// Create a POST route decoding a JSON body merged with path parameters.
#[must_use]
pub fn post<H, P>() -> MethodRoute<P>
where
    H: Handler<P> + DeserializeOwned + 'static,
    H::Output: Serialize,
    H::Error: Into<HttpError>,
    P: Send + Sync + 'static,
{
    post_with(|raw: RawRequest<'_>| body_input::<H>(raw.path_params, raw.body), json)
}

async fn dispatch<H, P, D, E>(
    decode: D, encode: E, State(client): State<Client<P>>, params: RawPathParams,
    RawQuery(query): RawQuery, headers: HeaderMap, body: Bytes,
) -> Response
where
    H: Handler<P>,
    H::Error: Into<HttpError>,
    D: Fn(RawRequest<'_>) -> Result<H, DecodeError>,
    E: Fn(H::Output) -> Response,
    P: Send + Sync + 'static,
{
    let path_params: Vec<(String, String)> =
        params.iter().map(|(key, value)| (key.to_owned(), value.to_owned())).collect();
    let raw = RawRequest {
        path_params: &path_params,
        query: query.as_deref(),
        headers: &headers,
        body: &body,
    };
    let input = match decode(raw) {
        Ok(input) => input,
        Err(error) => return HttpError::from(error).into_response(),
    };
    let metadata = Metadata::from_lookup(|name| {
        headers.get(format!("x-{name}")).and_then(|value| value.to_str().ok()).map(str::to_owned)
    });
    match client.call(input, &metadata).await {
        Ok(output) => encode(output),
        Err(error) => Into::<HttpError>::into(error).into_response(),
    }
}

/// Encode a handler output as a JSON response.
#[must_use]
pub fn json<T: Serialize>(output: T) -> Response {
    match serde_json::to_vec(&output) {
        Ok(body) => {
            (StatusCode::OK, [(CONTENT_TYPE, HeaderValue::from_static("application/json"))], body)
                .into_response()
        }
        Err(error) => {
            let error = crate::server_error!("body encoding error: {}", error);
            let body =
                serde_json::to_vec(&error).unwrap_or_else(|_| error.to_string().into_bytes());
            (error.status(), [(CONTENT_TYPE, HeaderValue::from_static("application/json"))], body)
                .into_response()
        }
    }
}

fn invalid(description: String) -> DecodeError {
    DecodeError::new(description)
}

fn query_input<T: DeserializeOwned>(
    params: &[(String, String)], query: Option<&str>,
) -> Result<T, DecodeError> {
    let mut pairs: Vec<(String, String)> = params.to_vec();
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

fn body_input<T: DeserializeOwned>(
    params: &[(String, String)], body: &[u8],
) -> Result<T, DecodeError> {
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
        object.insert(key.clone(), serde_json::Value::String(param.clone()));
    }
    serde_json::from_value(value).map_err(|error| invalid(format!("invalid request body: {error}")))
}
