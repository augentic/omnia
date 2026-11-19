//! [`Model`] over `omnia:model`.

use std::future::Future;

use omnia_wasi_model::omnia::{completion, types};
use omnia_wasi_model::wit_stream;
use wasip3::filesystem::preopens;

use super::lend::resolve_lend;
use crate::model::{
    CHECK_TOOL, Error, Format, Function, Generation, Message, Reply, Request, Role, Tool, ToolCall,
    Usage, WorkerGrant,
};
use crate::{Model, WasiProvider};

const DEFAULT_MODEL: &str = "default";

impl Model for WasiProvider {
    async fn complete_with<H, F>(&self, request: Request, mut handler: H) -> Result<Reply, Error>
    where
        H: FnMut(ToolCall) -> F + Send,
        F: Future<Output = Result<String, String>> + Send,
    {
        // the lent workspace borrows a descriptor, so the table outlives `complete`
        let directories =
            if request.workspace.is_some() { preopens::get_directories() } else { vec![] };
        let workspace = match request.workspace.as_deref() {
            None => None,
            Some(path) => match resolve_lend(&directories, path) {
                Some((root, subpath)) => Some(types::Location {
                    root,
                    subpath: subpath.to_string(),
                }),
                None => {
                    return Err(Error::InvalidRequest(format!(
                        "workspace lend `{path}` matches no preopen"
                    )));
                }
            },
        };

        // the guest's tools, refusing the name a check arrives under
        let mut tools = Vec::new();
        for Tool::Function(function) in request.tools {
            if function.name == CHECK_TOOL {
                return Err(Error::InvalidRequest(format!("reserved tool name: {CHECK_TOOL}")));
            }
            tools.push(function.into());
        }

        let wire = completion::Request {
            system: request.system,
            messages: request.messages.into_iter().map(Into::into).collect(),
            generation: request.generation.map(Into::into),
            format: request.format.into(),
            tools,
            grants: types::Grants {
                workspace,
                worker: request.worker.map(Into::into),
            },
            check: request.check,
        };

        // begin the completion on the model the request names
        let name = request.model.unwrap_or_else(|| DEFAULT_MODEL.to_owned());
        let model = completion::open(name).await?;
        let (mut results, results_rx) = wit_stream::new();
        let completion::Session { mut calls, reply } = model.complete(wire, results_rx).await?;

        // serial by design: each result feeds the same model turn; a rejected
        // write means the host stopped reading, and the closed stream ends the loop
        let mut correction = None;
        let calls_loop = async {
            while let Some(call) = calls.next().await {
                let result = match call {
                    types::Call::Tool(call) => {
                        correction = None;
                        let id = call.id.clone();
                        let output = handler(ToolCall {
                            id: call.id,
                            name: call.name,
                            arguments: call.arguments,
                        })
                        .await;
                        types::CallResult::Tool(types::ToolResult { id, output })
                    }
                    types::Call::Check(candidate) => {
                        let id = candidate.id.clone();
                        let outcome = handler(ToolCall {
                            id: candidate.id,
                            name: CHECK_TOOL.to_owned(),
                            arguments: candidate.answer,
                        })
                        .await
                        .map(drop);
                        correction = outcome.clone().err();
                        types::CallResult::Check(types::Verdict { id, outcome })
                    }
                };
                let _ = results.write_one(result).await;
            }
        };

        // the host always resolves the reply, so the join cannot hang
        let ((), outcome) = futures::join!(calls_loop, std::future::IntoFuture::into_future(reply));
        match outcome? {
            Ok(reply) => Ok(reply.into()),
            Err(exhausted) => Err(Error::BudgetExhausted(correction.unwrap_or(exhausted.detail))),
        }
    }
}

impl From<Message> for types::Message {
    fn from(message: Message) -> Self {
        Self {
            role: message.role.into(),
            content: message.content,
        }
    }
}

impl From<Role> for types::Role {
    fn from(role: Role) -> Self {
        match role {
            Role::System => Self::System,
            Role::User => Self::User,
            Role::Assistant => Self::Assistant,
        }
    }
}

impl From<Generation> for types::Generation {
    fn from(generation: Generation) -> Self {
        Self {
            temperature: generation.temperature,
            top_p: generation.top_p,
            max_tokens: generation.max_tokens,
            stop_sequences: generation.stop,
            seed: generation.seed,
            effort: generation.effort.as_ref().map(|effort| effort.as_str().to_owned()),
        }
    }
}

impl From<Format> for types::Format {
    fn from(format: Format) -> Self {
        match format {
            Format::Text => Self::Text,
            Format::Json => Self::Json,
            Format::Schema(s) => Self::Schema(types::Schema {
                name: s.name,
                schema: s.schema,
            }),
        }
    }
}

impl From<Function> for types::Tool {
    fn from(function: Function) -> Self {
        Self {
            name: function.name,
            description: Some(function.description).filter(|description| !description.is_empty()),
            input_schema: function.parameters,
        }
    }
}

impl From<WorkerGrant> for types::WorkerGrant {
    fn from(grant: WorkerGrant) -> Self {
        Self {
            egress: grant.egress,
            read: grant.read,
            env: grant.env,
        }
    }
}

impl From<types::Reply> for Reply {
    fn from(reply: types::Reply) -> Self {
        Self {
            answer: reply.answer,
            usage: reply.usage.map(Into::into),
        }
    }
}

impl From<types::Usage> for Usage {
    fn from(usage: types::Usage) -> Self {
        Self {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            reasoning_tokens: usage.reasoning_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            cache_write_tokens: usage.cache_write_tokens,
        }
    }
}

impl From<types::Error> for Error {
    fn from(error: types::Error) -> Self {
        match error {
            types::Error::Invalid(detail) => Self::InvalidRequest(detail),
            types::Error::NotFound(detail) => Self::NotFound(detail),
            types::Error::Denied(detail) => Self::Denied(detail),
            types::Error::Unsupported(detail) => Self::Unsupported(detail),
            types::Error::Unavailable(detail) => Self::Unavailable(detail),
            types::Error::Other(detail) => Self::Backend(detail),
        }
    }
}
