//! # Typed Guest API Wasm Guest
//!
//! This module demonstrates the explicit operation and HTTP router API.

#![cfg(target_arch = "wasm32")]

use omnia_guest::api::http::{Router, get, post, serve};
use omnia_guest::api::{CallContext, Invoker, Operation};
use omnia_guest::{Error, wasip3};
use serde::{Deserialize, Serialize};

struct Provider;

#[derive(Debug, Deserialize)]
struct GreetArgs {
    name: String,
}

struct Greet;

#[derive(Debug, Serialize)]
struct Greeting {
    message: String,
    owner: String,
    request_id: String,
}

impl Operation<Provider> for Greet {
    type Error = Error;
    type Input = GreetArgs;
    type Output = Greeting;

    async fn call(
        input: Self::Input, context: CallContext<'_, Provider>,
    ) -> Result<Self::Output, Self::Error> {
        Ok(Greeting {
            message: format!("Hello, {}!", input.name),
            owner: context.owner.to_string(),
            request_id: context.metadata.correlation_id.as_deref().unwrap_or("none").to_string(),
        })
    }
}

fn router() -> Router<Provider> {
    Router::new(Invoker::new("examples", Provider))
        .route("/greet/{name}", get::<Greet, Provider>())
        .route("/greet", post::<Greet, Provider>())
}

struct Http;
wasip3::http::service::export!(Http);

impl wasip3::exports::http::handler::Guest for Http {
    async fn handle(
        request: wasip3::http::types::Request,
    ) -> Result<wasip3::http::types::Response, wasip3::http::types::ErrorCode> {
        serve(router(), request).await
    }
}
