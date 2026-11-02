//! The `create` host binding and its reply pipeline.

use std::sync::Arc;

use anyhow::{Context as _, anyhow};
use cap_std::fs::{Metadata, MetadataExt as _};
use futures::FutureExt as _;
use omnia_core::{HasMounts, MountRegistry};
use wasmtime::component::{Accessor, FutureReader, StreamReader};
use wasmtime_wasi::filesystem::Descriptor;

use crate::host::generated::omnia::model::completion::{
    Effort as WitEffort, Error as WitError, Format as WitFormat, Function as WitFunction,
    Generation as WitGeneration, Host, HostWithStore, Mcp as WitMcp, Message as WitMessage,
    Reply as WitReply, Request as WitRequest, Role as WitRole, Schema as WitSchema, Session,
    Tool as WitTool, ToolResult as WitToolResult, Usage as WitUsage, WorkerGrant as WitWorkerGrant,
    WorkspaceGrant,
};
use crate::host::session::{CallsProducer, ReplyTask, ResultsConsumer};
use crate::host::{
    BoundToolHost, Effort, Error, Format, Function, Generation, McpGrant, Message, Request, Role,
    SchemaFormat, SessionClose, Tool, ToolHost, ToolSession, Usage, WasiModel, WasiModelView,
    WorkerGrant, Workspace,
};

impl<T> HostWithStore<T> for WasiModel
where
    T: HasMounts,
{
    fn create(
        accessor: &Accessor<T, Self>, wit_request: WitRequest,
        mut results: StreamReader<WitToolResult>,
    ) -> impl std::future::Future<Output = Result<Session, WitError>> {
        std::future::ready(accessor.with(|mut access| {
            let (request, workspace_grant) = split_wit_request(wit_request);
            if let Err(error) = request.validate() {
                results.close(&mut access)?;
                return Err(to_wit_error(error));
            }

            let workspace = if let Some(grant) = workspace_grant.as_ref() {
                let mounts = access.data_mut().mounts();
                let descriptor = access.get().table.get(&grant.root)?;

                match resolve_descriptor(descriptor, &mounts, grant) {
                    Ok(workspace) => Some(workspace),
                    Err(error) => {
                        results.close(&mut access)?;
                        return Err(error.into());
                    }
                }
            } else {
                None
            };

            let limits = access.get().client.limits();
            let allowed = request.tool_names();

            let (session, calls_rx) = ToolSession::new(limits, allowed);
            let tool_host: Arc<dyn ToolHost> =
                Arc::new(BoundToolHost::new(Arc::clone(&session), workspace));
            let answer = access.get().client.complete(request, tool_host);

            results.pipe(&mut access, ResultsConsumer::new(Arc::clone(&session)))?;
            let mut calls = StreamReader::new(&mut access, CallsProducer::new(calls_rx))?;

            let close = SessionClose::new(Arc::clone(&session));
            let reply_task = ReplyTask::spawn(async move {
                let _close = close;
                match answer.await {
                    Ok(answer) => session.take_error().map_or_else(
                        || {
                            Ok(WitReply {
                                answer: answer.answer,
                                usage: answer.usage.map(to_wit_usage),
                            })
                        },
                        |e| Err(to_wit_error(e)),
                    ),
                    Err(error) => {
                        if let Some(session_err) = session.take_error() {
                            return Err(to_wit_error(session_err));
                        }
                        match error.downcast::<Error>() {
                            Ok(err) => Err(to_wit_error(err)),
                            Err(error) => match error.downcast::<WitError>() {
                                Ok(wit_err) => Err(wit_err),
                                Err(other) => Err(WitError::from(other)),
                            },
                        }
                    }
                }
            });

            let reply_fut = reply_task.join().map(Ok::<_, wasmtime::Error>);
            let reply = match FutureReader::new(&mut access, reply_fut) {
                Ok(reply) => reply,
                Err(error) => {
                    calls.close(&mut access)?;
                    return Err(error.into());
                }
            };

            Ok(Session { calls, reply })
        }))
    }
}

impl Host for WasiModelView<'_> {
    fn convert_error(&mut self, err: WitError) -> wasmtime::Result<WitError> {
        Ok(err)
    }
}

fn resolve_descriptor(
    descriptor: &Descriptor, registry: &MountRegistry, grant: &WorkspaceGrant,
) -> anyhow::Result<Workspace> {
    let Descriptor::Dir(dir) = descriptor else {
        return Err(anyhow!("grants.workspace root must be a directory"));
    };

    let meta =
        Metadata::from_file(&dir.dir).context("reading lent workspace directory metadata")?;
    let entry = registry
        .match_identity(meta.dev(), meta.ino())
        .context("lent workspace root is not an authorized mount")?;
    Workspace::from_mount(entry, &grant.subpath)
}

fn split_wit_request(req: WitRequest) -> (Request, Option<WorkspaceGrant>) {
    let workspace = req.grants.workspace.as_ref().map(|g| g.subpath.clone());
    let grant = req.grants.workspace;
    let request = Request {
        model: req.model,
        system: req.system,
        messages: req
            .messages
            .into_iter()
            .map(|WitMessage { role, content }| Message {
                role: match role {
                    WitRole::System => Role::System,
                    WitRole::User => Role::User,
                    WitRole::Assistant => Role::Assistant,
                },
                content,
            })
            .collect(),
        generation: req.generation.map(
            |WitGeneration {
                 temperature,
                 top_p,
                 max_tokens,
                 stop,
                 seed,
                 effort,
             }| Generation {
                temperature,
                top_p,
                max_tokens,
                stop,
                seed,
                effort: effort.map(|e| match e {
                    WitEffort::Minimal => Effort::Minimal,
                    WitEffort::Low => Effort::Low,
                    WitEffort::Medium => Effort::Medium,
                    WitEffort::High => Effort::High,
                }),
            },
        ),
        format: match req.format {
            WitFormat::Text => Format::Text,
            WitFormat::Json => Format::Json,
            WitFormat::Schema(WitSchema { name, schema }) => {
                Format::Schema(SchemaFormat { name, schema })
            }
        },
        tools: req
            .tools
            .into_iter()
            .map(|t| match t {
                WitTool::Function(WitFunction {
                    name,
                    description,
                    parameters,
                }) => Tool::Function(Function {
                    name,
                    description,
                    parameters,
                }),
                WitTool::Mcp(WitMcp { name, tools, url }) => {
                    Tool::Mcp(McpGrant { name, tools, url })
                }
            })
            .collect(),
        check: req.check,
        workspace,
        worker: req.grants.worker.map(|WitWorkerGrant { egress, read, env }| WorkerGrant {
            egress,
            read,
            env,
        }),
    };
    (request, grant)
}

const fn to_wit_usage(u: Usage) -> WitUsage {
    WitUsage {
        input_tokens: u.input_tokens,
        output_tokens: u.output_tokens,
        reasoning_tokens: u.reasoning_tokens,
    }
}

fn to_wit_error(err: Error) -> WitError {
    match err {
        Error::InvalidRequest(d) => WitError::InvalidRequest(d),
        Error::BudgetExhausted(d) => WitError::BudgetExhausted(d),
        Error::ToolFailed(d) => WitError::ToolFailed(d),
        Error::Backend(d) => WitError::Backend(d),
    }
}
