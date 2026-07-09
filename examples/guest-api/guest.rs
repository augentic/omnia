//! # Typed Guest API Wasm Guest
//!
//! This module demonstrates the `guest!` macro and the typed `Handler` API.
//! Instead of hand-writing the WASI export and an axum router, the macro
//! generates both from a declarative route table; each request type
//! implements [`Handler`] to parse its input and produce a [`Reply`].

#![cfg(target_arch = "wasm32")]

use omnia_guest::{Context, Error, Handler, IntoBody, Reply, guest};
use serde::{Deserialize, Serialize};

#[derive(Default)]
struct Provider;

guest!({
    owner: "examples",
    provider: Provider,
    http: [
        "/greet/{name}": get(Greet, Greeting),
        "/greet": post(Greet, Greeting),
    ],
});

/// Path parameters, query pairs, and the JSON body merge into this flat
/// input before `from_input` runs.
#[derive(Debug, Deserialize)]
struct GreetArgs {
    name: String,
}

#[derive(Debug)]
struct Greet {
    args: GreetArgs,
}

#[derive(Debug, Serialize)]
struct Greeting {
    message: String,
    owner: String,
    request_id: String,
}

impl IntoBody for Greeting {
    fn into_body(self) -> anyhow::Result<Vec<u8>> {
        Ok(serde_json::to_vec(&self)?)
    }
}

impl Handler<Provider> for Greet {
    type Error = Error;
    type Input = GreetArgs;
    type Output = Greeting;

    fn from_input(input: GreetArgs) -> Result<Self, Error> {
        Ok(Self { args: input })
    }

    async fn handle(self, ctx: Context<'_, Provider>) -> Result<Reply<Greeting>, Error> {
        let request_id = ctx
            .headers
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("none")
            .to_string();

        Ok(Reply::ok(Greeting {
            message: format!("Hello, {}!", self.args.name),
            owner: ctx.owner.to_string(),
            request_id,
        }))
    }
}
