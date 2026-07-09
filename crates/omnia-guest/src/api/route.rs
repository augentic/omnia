//! Target-neutral route constructors over [`Handler`] implementations.
//!
//! Each constructor returns an [`MethodRouter`] whose state is a [`Client`]:
//! the owner and provider arrive as router state rather than being
//! constructed per request, so one router definition serves both the wasm
//! guest (bridged through `omnia_wasi_http::serve`) and a native listener
//! (`axum::serve` or `tower::ServiceExt::oneshot`).
//!
//! Extraction is typed: path parameters, query pairs (GET), and the JSON
//! body (POST) merge into one flat map and deserialize into
//! [`Handler::Input`] via serde, so the same flat `Input` struct is
//! reachable from every transport with shared field names.

use axum::extract::{RawPathParams, RawQuery, State};
use axum::routing::{self, MethodRouter};
use http::HeaderMap;
use serde::de::DeserializeOwned;

use crate::api::reply::Reply;
use crate::api::{Client, Handler, HttpError, HttpResult, IntoBody, Provider};

/// A GET route for `R`: path parameters and query pairs merge into one
/// url-encoded map and deserialize into `R::Input`.
pub fn get<R, P>() -> MethodRouter<Client<P>>
where
    R: Handler<P> + Send + 'static,
    R::Input: DeserializeOwned + Send,
    R::Output: IntoBody + 'static,
    R::Error: Into<HttpError> + Send,
    P: Provider + 'static,
{
    routing::get(
        |State(client): State<Client<P>>,
         params: RawPathParams,
         RawQuery(query): RawQuery,
         headers: HeaderMap| async move {
            let input = query_input::<R::Input>(&params, query.as_deref())?;
            run::<R, P>(&client, headers, input).await
        },
    )
}

/// A POST route for `R`: the JSON body (an object; absent bodies read as
/// `{}`) plus path parameters deserialize into `R::Input`.
pub fn post<R, P>() -> MethodRouter<Client<P>>
where
    R: Handler<P> + Send + 'static,
    R::Input: DeserializeOwned + Send,
    R::Output: IntoBody + 'static,
    R::Error: Into<HttpError> + Send,
    P: Provider + 'static,
{
    routing::post(
        |State(client): State<Client<P>>,
         params: RawPathParams,
         headers: HeaderMap,
         body: axum::body::Bytes| async move {
            let input = body_input::<R::Input>(&params, &body)?;
            run::<R, P>(&client, headers, input).await
        },
    )
}

// Drive one parsed input through the handler with the client's owner,
// provider, and the inbound request headers, projecting the handler error
// onto the HTTP error surface.
async fn run<R, P>(
    client: &Client<P>, headers: HeaderMap, input: R::Input,
) -> HttpResult<Reply<R::Output>>
where
    R: Handler<P>,
    R::Error: Into<HttpError>,
    P: Provider,
{
    let request = R::from_input(input).map_err(Into::into)?;
    client.request(request).headers(headers).handle().await.map_err(Into::into)
}

fn invalid(description: String) -> HttpError {
    crate::Error::BadRequest {
        code: "invalid_request".to_string(),
        description,
    }
    .into()
}

// Merge path parameters and query pairs, then deserialize the flat
// url-encoded map into `T` (numbers and bools parse from their string
// forms; repeated keys are unsupported — use a POST body for sequences).
fn query_input<T: DeserializeOwned>(
    params: &RawPathParams, query: Option<&str>,
) -> Result<T, HttpError> {
    let mut pairs: Vec<(String, String)> =
        params.iter().map(|(key, value)| (key.to_owned(), value.to_owned())).collect();
    if let Some(query) = query {
        let parsed: Vec<(String, String)> = serde_urlencoded::from_str(query)
            .map_err(|e| invalid(format!("malformed query string: {e}")))?;
        pairs.extend(parsed);
    }
    let encoded = serde_urlencoded::to_string(&pairs)
        .map_err(|e| invalid(format!("cannot encode request parameters: {e}")))?;
    serde_urlencoded::from_str(&encoded)
        .map_err(|e| invalid(format!("invalid request parameters: {e}")))
}

// Fold path parameters into the JSON body object, then deserialize into `T`.
fn body_input<T: DeserializeOwned>(params: &RawPathParams, body: &[u8]) -> Result<T, HttpError> {
    let mut value = if body.is_empty() {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_slice(body).map_err(|e| invalid(format!("malformed JSON body: {e}")))?
    };
    let serde_json::Value::Object(object) = &mut value else {
        return Err(invalid("the request body must be a JSON object".to_string()));
    };
    for (key, param) in params {
        object.insert(key.to_owned(), serde_json::Value::String(param.to_owned()));
    }
    serde_json::from_value(value).map_err(|e| invalid(format!("invalid request body: {e}")))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::{Body, to_bytes};
    use http::{Method, Request, StatusCode};
    use serde::{Deserialize, Serialize};
    use tower::ServiceExt as _;

    use super::*;
    use crate::api::Context;

    #[derive(Debug, Deserialize)]
    struct EchoArgs {
        name: String,
        count: Option<u32>,
    }

    #[derive(Debug, Serialize)]
    struct EchoBody {
        name: String,
        count: u32,
    }

    impl IntoBody for EchoBody {
        fn into_body(self) -> anyhow::Result<Vec<u8>> {
            Ok(serde_json::to_vec(&self)?)
        }
    }

    #[derive(Debug)]
    struct Echo {
        args: EchoArgs,
    }

    impl<P: Provider> Handler<P> for Echo {
        type Error = crate::Error;
        type Input = EchoArgs;
        type Output = EchoBody;

        fn from_input(input: EchoArgs) -> Result<Self, Self::Error> {
            Ok(Self { args: input })
        }

        async fn handle(self, _ctx: Context<'_, P>) -> Result<Reply<EchoBody>, Self::Error> {
            Ok(Reply::ok(EchoBody {
                name: self.args.name,
                count: self.args.count.unwrap_or(1),
            }))
        }
    }

    fn router() -> Router {
        Router::new()
            .route("/echo", get::<Echo, ()>().merge(post::<Echo, ()>()))
            .route("/echo/{name}", get::<Echo, ()>().merge(post::<Echo, ()>()))
            .with_state(Client::new("test").provider(()))
    }

    async fn send(request: Request<Body>) -> (StatusCode, serde_json::Value) {
        let response = router().oneshot(request).await.expect("router serves the request");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.expect("collect body");
        let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, value)
    }

    #[tokio::test]
    async fn get_query() {
        let request = Request::builder()
            .method(Method::GET)
            .uri("/echo?name=plan&count=3")
            .body(Body::empty())
            .expect("build request");
        let (status, value) = send(request).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value, serde_json::json!({ "name": "plan", "count": 3 }));
    }

    #[tokio::test]
    async fn get_path_and_query_merge() {
        let request = Request::builder()
            .method(Method::GET)
            .uri("/echo/slice?count=2")
            .body(Body::empty())
            .expect("build request");
        let (status, value) = send(request).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value, serde_json::json!({ "name": "slice", "count": 2 }));
    }

    #[tokio::test]
    async fn get_missing_required_field() {
        let request = Request::builder()
            .method(Method::GET)
            .uri("/echo?count=3")
            .body(Body::empty())
            .expect("build request");
        let (status, _) = send(request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn post_body_and_path_merge() {
        let request = Request::builder()
            .method(Method::POST)
            .uri("/echo/slice")
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"count":7}"#))
            .expect("build request");
        let (status, value) = send(request).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value, serde_json::json!({ "name": "slice", "count": 7 }));
    }

    #[tokio::test]
    async fn post_empty_body_reads_as_object() {
        let request = Request::builder()
            .method(Method::POST)
            .uri("/echo/slice")
            .body(Body::empty())
            .expect("build request");
        let (status, value) = send(request).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value, serde_json::json!({ "name": "slice", "count": 1 }));
    }

    #[tokio::test]
    async fn post_non_object_body_rejected() {
        let request = Request::builder()
            .method(Method::POST)
            .uri("/echo/slice")
            .body(Body::from("[1,2]"))
            .expect("build request");
        let (status, _) = send(request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
