//! # Typed Guest API Wasm Guest
//!
//! This module demonstrates the explicit handler and HTTP router API.

#![cfg(target_arch = "wasm32")]

use axum::Router;
use omnia_guest::api::http::{get, post, serve};
use omnia_guest::api::{Client, Context, Handler};
use omnia_guest::{Error, wasip3};
use serde::{Deserialize, Serialize};

struct Provider;

#[derive(Debug, Deserialize)]
struct GreetArgs {
    name: String,
}

#[derive(Debug, Serialize)]
struct Greeting {
    message: String,
    owner: String,
    request_id: String,
}

impl Handler<Provider> for GreetArgs {
    type Error = Error;
    type Output = Greeting;

    async fn handle(self, context: Context<'_, Provider>) -> Result<Self::Output, Self::Error> {
        Ok(Greeting {
            message: format!("Hello, {}!", self.name),
            owner: context.owner.to_string(),
            request_id: context.metadata.correlation_id.as_deref().unwrap_or("none").to_string(),
        })
    }
}

fn router() -> Router {
    Router::new()
        .route("/greet/{name}", get::<GreetArgs, Provider>())
        .route("/greet", post::<GreetArgs, Provider>())
        .with_state(Client::new("examples", Provider))
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
