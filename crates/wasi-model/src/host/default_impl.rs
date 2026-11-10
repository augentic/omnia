//! Echo default implementation of [`ModelClient`].

use std::sync::Arc;

use anyhow::{Result, anyhow};
use futures::FutureExt as _;
use omnia_backend::model::{Answer, Error, Format, ModelClient, Request, ToolHost};
use omnia_core::{Connect, FutureResult, NoOptions};
use serde_json::json;

/// Echo default implementation of [`ModelClient`].
#[derive(Clone, Copy, Debug, Default)]
pub struct ModelDefault;

impl Connect for ModelDefault {
    type ConnectOptions = NoOptions;

    fn connect_with(
        _options: Self::ConnectOptions,
    ) -> impl std::future::Future<Output = Result<Self>> {
        std::future::ready(Ok(Self))
    }
}

impl ModelClient for ModelDefault {
    fn complete(&self, request: Request, tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        let answer = echo(&request);
        let check = request.check;
        async move {
            let answer = answer?;
            if check {
                tool_host
                    .check(answer.clone())
                    .await?
                    .map_err(|correction| anyhow!(Error::BudgetExhausted(correction)))?;
            }
            Ok(Answer::from(answer))
        }
        .boxed()
    }
}

fn echo(request: &Request) -> Result<String> {
    let prompt = request.messages.last().map(|message| message.content.clone()).unwrap_or_default();
    match &request.format {
        Format::Text => Ok(prompt),
        Format::Json => Ok(json!({ "echo": prompt }).to_string()),
        Format::Schema(_) => Err(anyhow!(
            "the default echo backend cannot satisfy format::schema: bind a real model backend"
        )),
    }
}
