//! [`Model`] over `omnia:model/completion`.

use std::future::Future;

use omnia_wasi_model::{completion, wit_stream};
use wasip3::filesystem::preopens;

use super::lend::resolve_lend;
use crate::model::{
    Effort, Error, Format, Function, Generation, McpGrant, Message, Reply, Request, Role, Tool,
    ToolCall, Usage, WorkerGrant,
};
use crate::{Model, WasiProvider};

impl Model for WasiProvider {
    async fn complete_with<H, F>(&self, request: Request, mut handler: H) -> Result<Reply, Error>
    where
        H: FnMut(ToolCall) -> F + Send,
        F: Future<Output = Result<String, String>> + Send,
    {
        // the lent workspace borrows a descriptor, so the table outlives `create`
        let directories =
            if request.workspace.is_some() { preopens::get_directories() } else { vec![] };
        let workspace = match request.workspace.as_deref() {
            None => None,
            Some(path) => match resolve_lend(&directories, path) {
                Some((root, subpath)) => Some(completion::WorkspaceGrant {
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

        let wire = completion::Request {
            model: request.model,
            system: request.system,
            messages: request.messages.into_iter().map(Into::into).collect(),
            generation: request.generation.map(Into::into),
            format: request.format.into(),
            tools: request.tools.into_iter().map(Into::into).collect(),
            grants: completion::Grants {
                workspace,
                worker: request.worker.map(Into::into),
            },
            check: request.check,
        };

        let (mut results, results_rx) = wit_stream::new();
        let session = completion::create(wire, results_rx).await.map_err(Error::from)?;
        let completion::Session { mut calls, reply } = session;

        // serial by design: each result feeds the same model turn; a rejected
        // write means the host stopped reading, and the closed stream ends the loop
        let calls_loop = async {
            while let Some(call) = calls.next().await {
                let id = call.id.clone();
                let output = handler(ToolCall {
                    id: call.id,
                    name: call.name,
                    arguments: call.arguments,
                })
                .await;
                let _ = results.write_one(completion::ToolResult { id, output }).await;
            }
        };

        // the host always resolves the reply, so the join cannot hang
        let ((), outcome) = futures::join!(calls_loop, std::future::IntoFuture::into_future(reply));
        outcome.map(Into::into).map_err(Into::into)
    }
}

impl From<Message> for completion::Message {
    fn from(message: Message) -> Self {
        Self {
            role: message.role.into(),
            content: message.content,
        }
    }
}

impl From<Role> for completion::Role {
    fn from(role: Role) -> Self {
        match role {
            Role::System => Self::System,
            Role::User => Self::User,
            Role::Assistant => Self::Assistant,
        }
    }
}

impl From<Generation> for completion::Generation {
    fn from(generation: Generation) -> Self {
        Self {
            temperature: generation.temperature,
            top_p: generation.top_p,
            max_tokens: generation.max_tokens,
            stop: generation.stop,
            seed: generation.seed,
            effort: generation.effort.map(Into::into),
        }
    }
}

impl From<Effort> for completion::Effort {
    fn from(effort: Effort) -> Self {
        match effort {
            Effort::Minimal => Self::Minimal,
            Effort::Low => Self::Low,
            Effort::Medium => Self::Medium,
            Effort::High => Self::High,
        }
    }
}

impl From<Format> for completion::Format {
    fn from(format: Format) -> Self {
        match format {
            Format::Text => Self::Text,
            Format::Json => Self::Json,
            Format::Schema(s) => Self::Schema(completion::Schema {
                name: s.name,
                schema: s.schema,
            }),
        }
    }
}

impl From<Tool> for completion::Tool {
    fn from(tool: Tool) -> Self {
        match tool {
            Tool::Function(Function {
                name,
                description,
                parameters,
            }) => Self::Function(completion::Function {
                name,
                description,
                parameters,
            }),
            Tool::Mcp(McpGrant { name, tools, url }) => {
                Self::Mcp(completion::Mcp { name, tools, url })
            }
        }
    }
}

impl From<WorkerGrant> for completion::WorkerGrant {
    fn from(grant: WorkerGrant) -> Self {
        Self {
            egress: grant.egress,
            read: grant.read,
            env: grant.env,
        }
    }
}

impl From<completion::Reply> for Reply {
    fn from(reply: completion::Reply) -> Self {
        Self {
            answer: reply.answer,
            usage: reply.usage.map(Into::into),
        }
    }
}

impl From<completion::Usage> for Usage {
    fn from(usage: completion::Usage) -> Self {
        Self {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            reasoning_tokens: usage.reasoning_tokens,
        }
    }
}

impl From<completion::Error> for Error {
    fn from(error: completion::Error) -> Self {
        match error {
            completion::Error::InvalidRequest(detail) => Self::InvalidRequest(detail),
            completion::Error::BudgetExhausted(detail) => Self::BudgetExhausted(detail),
            completion::Error::ToolFailed(detail) => Self::ToolFailed(detail),
            completion::Error::Backend(detail) => Self::Backend(detail),
        }
    }
}
