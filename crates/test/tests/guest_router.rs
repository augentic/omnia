//! `omnia_test::guest::{send, deliver}` over a native router.

#![cfg(not(target_arch = "wasm32"))]

use axum::Json;
use axum::body::Body;
use axum::extract::Path;
use axum::routing::get;
use http::{Request, StatusCode};
use omnia_sdk::StateStore as _;
use omnia_sdk::api::messaging::DeliveryError;
use omnia_sdk::api::{Client, Context, Handlers, Router};
use omnia_test::guest::{Provider, deliver, send};

async fn shout(Path(word): Path<String>) -> Json<String> {
    Json(word.to_uppercase())
}

async fn note(context: Context<Provider>, text: String) -> Result<(), omnia_sdk::Error> {
    context.state().set("note", text.as_bytes(), None).await?;
    Ok(())
}

fn router() -> (Provider, Router<Provider>) {
    let provider = Provider::default();
    let router = Handlers::new()
        .route("/shout/{word}", get(shout))
        .topic("notes", note)
        .with_state(Client::new(provider.clone()));
    (provider, router)
}

#[tokio::test]
async fn send_json_route() {
    let (_, router) = router();
    let request = Request::get("/shout/hi").body(Body::empty()).expect("build request");

    let response = send(&router, request).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "application/json");
    assert_eq!(response.body().as_ref(), br#""HI""#);
}

#[tokio::test]
async fn send_unknown_path() {
    let (_, router) = router();
    let request = Request::get("/missing").body(Body::empty()).expect("build request");

    let response = send(&router, request).await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deliver_topic() {
    let (provider, router) = router();

    deliver(&router, "notes", br#""remember""#).await.expect("the topic is handled");

    assert_eq!(provider.storage.state("note").as_deref(), Some(b"remember".as_slice()));
}

#[tokio::test]
async fn deliver_unhandled_topic() {
    let (_, router) = router();

    assert_eq!(
        deliver(&router, "other", b"{}").await,
        Err(DeliveryError::UnhandledTopic("other".to_owned()))
    );
}
