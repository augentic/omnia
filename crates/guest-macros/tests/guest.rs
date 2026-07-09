//! Native expansion check for `guest!`: the generated `http_router` must
//! build target-neutrally (no wasm toolchain) and dispatch through the
//! [`omnia_guest::api::Client`] router state.

use axum::body::{Body, to_bytes};
use http::{Method, Request, StatusCode};
use omnia_guest::api::{Context, Handler, IntoBody, Provider, Reply};
use serde::{Deserialize, Serialize};
use tower::ServiceExt as _;

#[derive(Debug, Default)]
struct MyProvider;

#[derive(Debug, Deserialize)]
struct DetectionArgs {
    id: String,
    threshold: Option<u32>,
}

#[derive(Debug, Serialize)]
struct DetectionReply {
    id: String,
    threshold: u32,
}

impl IntoBody for DetectionReply {
    fn into_body(self) -> anyhow::Result<Vec<u8>> {
        Ok(serde_json::to_vec(&self)?)
    }
}

#[derive(Debug)]
struct DetectionRequest {
    args: DetectionArgs,
}

impl<P: Provider> Handler<P> for DetectionRequest {
    type Error = omnia_guest::Error;
    type Input = DetectionArgs;
    type Output = DetectionReply;

    fn from_input(input: DetectionArgs) -> Result<Self, Self::Error> {
        Ok(Self { args: input })
    }

    async fn handle(self, _ctx: Context<'_, P>) -> Result<Reply<DetectionReply>, Self::Error> {
        Ok(Reply::ok(DetectionReply {
            id: self.args.id,
            threshold: self.args.threshold.unwrap_or(10),
        }))
    }
}

omnia_guest::guest!({
    owner: "at",
    provider: MyProvider,
    http: [
        "/jobs/{id}": get(DetectionRequest, DetectionReply),
        "/jobs": post(DetectionRequest with_body, DetectionReply),
    ]
});

fn router() -> axum::Router {
    http_router(omnia_guest::api::Client::new("at").provider(MyProvider))
}

#[tokio::test]
async fn get_route() {
    let request = Request::builder()
        .method(Method::GET)
        .uri("/jobs/abc?threshold=3")
        .body(Body::empty())
        .expect("build request");
    let response = router().oneshot(request).await.expect("router serves the request");
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = to_bytes(response.into_body(), usize::MAX).await.expect("collect body");
    let value: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
    assert_eq!(value, serde_json::json!({ "id": "abc", "threshold": 3 }));
}

#[tokio::test]
async fn post_route() {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/jobs")
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"id":"abc"}"#))
        .expect("build request");
    let response = router().oneshot(request).await.expect("router serves the request");
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = to_bytes(response.into_body(), usize::MAX).await.expect("collect body");
    let value: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
    assert_eq!(value, serde_json::json!({ "id": "abc", "threshold": 10 }));
}
