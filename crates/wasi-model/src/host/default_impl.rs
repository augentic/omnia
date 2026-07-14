//! `ModelDefault` — the crate's default, deterministic (echo) backend.
//!
//! It connects with zero configuration and echoes the request's last message
//! back as the answer, shaped to the request's `format`, so guest wiring and
//! prompt assembly can be smoke-tested with no live model. `format::schema`
//! cannot be satisfied by an echo (no fabricated value can conform to an
//! arbitrary guest schema), so those completions fail loud. Deployments bind
//! a real backend (`omnia-genai`, `omnia-cursor`); tests inject
//! `omnia-testkit`'s scripted double.

use std::fmt::Debug;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use futures::FutureExt as _;
use omnia::Backend;
use serde_json::{Value, json};

use crate::host::generated::omnia::model::completion::{Format, Request};
use crate::host::types::Answer;
use crate::host::{FutureResult, ToolHost, WasiModelCtx};

/// Options used to connect the default backend — none are needed.
#[derive(Clone, Copy, Debug, Default)]
pub struct ConnectOptions;

impl omnia::FromEnv for ConnectOptions {
    fn from_env() -> Result<Self> {
        Ok(Self)
    }
}

/// Echo default implementation of `wasi-model`: it starts without
/// configuration and answers every completion with its own prompt.
#[derive(Clone, Copy, Debug, Default)]
pub struct ModelDefault;

impl Backend for ModelDefault {
    type ConnectOptions = ConnectOptions;

    async fn connect_with(_options: Self::ConnectOptions) -> Result<Self> {
        Ok(Self)
    }
}

impl WasiModelCtx for ModelDefault {
    fn complete(&self, request: Request, _tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        let answer = echo(&request);
        async move { answer }.boxed()
    }
}

// Echo the last message's content, shaped to the request's `format` so the
// answer passes the host's validation gate.
fn echo(request: &Request) -> Result<Answer> {
    let prompt = request.messages.last().map(|message| message.content.clone()).unwrap_or_default();
    let value = match &request.format {
        Format::Text => Value::String(prompt),
        Format::Json => json!({ "echo": prompt }),
        Format::Schema(_) => {
            return Err(anyhow!(
                "the default echo backend cannot satisfy format::schema: bind a real model \
                 backend"
            ));
        }
    };
    Ok(Answer {
        value,
        usage: None,
        transcript: None,
    })
}
