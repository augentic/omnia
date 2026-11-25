//! `ModelDefault` — the crate's default, deterministic (echo) backend.
//!
//! It serves the models its [`ConnectOptions`] declares and echoes the
//! request's last message back as the answer, shaped to the request's
//! `format`, so guest wiring and prompt assembly can be smoke-tested with no
//! live model. What an echo cannot honour is `unsupported`: a
//! `format::schema` (no fabricated value can conform to an arbitrary guest
//! schema) or a generation control. A request that asks for a `check` gets
//! exactly one: the echo has no second candidate, so a rejection is
//! `exhausted`.
//! Deployments bind a real backend (`omnia-genai`, `omnia-cursor`); tests
//! define inline canned `ModelClient` impls that return a fixed answer.

use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::Result;
use futures::FutureExt as _;
use omnia_core::{Connect, FromEnv};
use serde_json::json;

use crate::{
    Answer, Budget, Error, Exhausted, Format, FutureResult, Generation, Limits, ModelClient,
    Request, ToolHost,
};

/// Options used to connect to the echo model backend.
///
/// This struct is used to load connection options from environment variables.
#[derive(Debug, Clone, FromEnv)]
pub struct ConnectOptions {
    /// The models a guest may open (`MODEL_NAMES`, comma-separated, default: `default`).
    #[env(from = "MODEL_NAMES", default = "default", with = omnia_core::env::list)]
    pub models: Vec<String>,
}

/// Echo default implementation of `omnia:model`: it answers every
/// completion with its own prompt.
///
/// It serves the models its [`ConnectOptions`] declares, and refuses every
/// other name with [`Error::NotFound`].
#[derive(Clone, Debug)]
pub struct ModelDefault {
    models: Arc<BTreeSet<String>>,
}

impl Connect for ModelDefault {
    type ConnectOptions = ConnectOptions;

    fn connect_with(
        options: Self::ConnectOptions,
    ) -> impl std::future::Future<Output = Result<Self>> {
        std::future::ready(Ok(Self {
            models: Arc::new(options.models.into_iter().collect()),
        }))
    }
}

impl ModelClient for ModelDefault {
    fn complete(&self, request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        let answer = self.echo(&request);
        let check = request.check;
        async move {
            let answer = answer?;
            if check && tool_host.check(answer.clone()).await?.is_err() {
                return Err(
                    Exhausted::new(Budget::Responses, "the echo proposes one candidate").into()
                );
            }
            Ok(Answer::from(answer))
        }
        .boxed()
    }

    fn limits(&self) -> Limits {
        Limits::default()
    }
}

impl ModelDefault {
    // Echo the last message's content, shaped to the request's `format`.
    fn echo(&self, request: &Request) -> Result<String, Error> {
        let name = &request.model;
        if !self.models.contains(name) {
            return Err(Error::NotFound(format!("model `{name}` is not declared")));
        }

        let prompt = request.messages.last().map_or_default(|message| message.content.clone());
        let answer = match &request.format {
            Format::Text => prompt,
            Format::Json => json!({ "echo": prompt }).to_string(),
            Format::Schema(_) => return Err(unsupported("cannot satisfy format::schema")),
        };
        if request.generation.as_ref().is_some_and(Generation::is_set) {
            return Err(unsupported("ignores generation controls"));
        }
        Ok(answer)
    }
}

fn unsupported(what: &str) -> Error {
    Error::Unsupported(format!("the default echo backend {what}: bind a real model backend"))
}
