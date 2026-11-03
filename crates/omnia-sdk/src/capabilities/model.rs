//! Prompt-completion (model) capability.
//!
//! Target-independent mirrors of the `omnia:model/completion` records. The
//! one record that cannot cross off `wasm32` is the `grants.workspace`
//! descriptor lend — a `wasi:filesystem` resource that only exists on
//! `wasm32` — so a guest names the directory with the plain
//! [`Request::workspace`] path and the `wasm32` default body resolves it
//! against the guest's preopens at the call site, by the lend rule every
//! lending capability shares: the longest preopen whose name prefixes the
//! path becomes the lent root descriptor and the remainder rides as the
//! grant's subpath.
//!
//! Acceptance of an answer is the guest's: a request with [`Request::check`]
//! set receives each candidate as a tool call named `check` and answers
//! `Ok` to finish or `Err(correction)` to send the model round again. The
//! typed [`Question`] runs that exchange for a `JsonSchema + Deserialize`
//! answer type.

mod question;

use std::future::Future;

pub use omnia_backend::model::{
    CHECK_TOOL, Effort, Error, Format, Function, Generation, McpGrant, Message, Reply, Request,
    Role, SchemaFormat, Tool, ToolCall, Usage, WorkerGrant,
};

pub use self::question::{Findings, Question, ToolFuture, Tools};

/// Prompt completion (Omnia Model).
///
/// Default WASM implementations delegate to `omnia:model/completion` via
/// `omnia-wasi-model`, opening the completion session and answering the
/// model's tool calls with the supplied closure; off `wasm32` the signatures
/// are bare so hosts and tests supply their own provider.
pub trait Model: Send + Sync {
    cfg_select! {
        not(all(target_arch = "wasm32", target_os = "wasi")) => {
            /// Single-shot completion returning one reply. Any tool call the model
            /// issues fails back to it; declare tools (or set `check`) and answer
            /// them through [`Model::complete_with`] instead.
            fn complete(
                &self, request: Request,
            ) -> impl Future<Output = Result<Reply, Error>> + Send;

            /// Completion with a tool closure: the model's tool calls arrive as
            /// [`ToolCall`] values answered serially by `handler` (over the caller's
            /// own locals and authority), and each result feeds the same model turn.
            /// Results correlate by id, so parallel handling stays available through
            /// the raw `omnia_wasi_model::completion` session bindings.
            fn complete_with<H, F>(
                &self, request: Request, handler: H,
            ) -> impl Future<Output = Result<Reply, Error>> + Send
            where
                H: FnMut(ToolCall) -> F + Send,
                F: Future<Output = Result<String, String>> + Send;
        }
        _ => {
            /// Single-shot completion returning one reply. Any tool call the model
            /// issues fails back to it; declare tools (or set `check`) and answer
            /// them through [`Model::complete_with`] instead.
            fn complete(
                &self, request: Request,
            ) -> impl Future<Output = Result<Reply, Error>> + Send {
                self.complete_with(request, |call: ToolCall| async move {
                    Err::<String, String>(format!(
                        "tool `{}` has no handler: answer tool calls with complete_with",
                        call.name
                    ))
                })
            }

            /// Completion with a tool closure: the model's tool calls arrive as
            /// [`ToolCall`] values answered serially by `handler` (over the caller's
            /// own locals and authority), and each result feeds the same model turn.
            /// Results correlate by id, so parallel handling stays available through
            /// the raw `omnia_wasi_model::completion` session bindings.
            fn complete_with<H, F>(
                &self, request: Request, mut handler: H,
            ) -> impl Future<Output = Result<Reply, Error>> + Send
            where
                H: FnMut(ToolCall) -> F + Send,
                F: Future<Output = Result<String, String>> + Send,
            {
                use omnia_wasi_model::{completion, wit_stream};
                use wasip3::filesystem::preopens;

                use super::lend::resolve_lend;

                async move {
                    // the lent workspace borrows a descriptor, so the table outlives `create`
                    let directories = if request.workspace.is_some() {
                        preopens::get_directories()
                    } else {
                        vec![]
                    };
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
                    let session =
                        completion::create(wire, results_rx).await.map_err(Error::from)?;
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
                    let ((), outcome) =
                        futures::join!(calls_loop, std::future::IntoFuture::into_future(reply));
                    outcome.map(Into::into).map_err(Into::into)
                }
            }
        }
    }
}

delegate_deref!(Model {
    fn complete(&self, request: Request) -> impl Future<Output = Result<Reply, Error>> + Send {
        (**self).complete(request)
    }

    fn complete_with<H, F>(
        &self, request: Request, handler: H,
    ) -> impl Future<Output = Result<Reply, Error>> + Send
    where
        H: FnMut(ToolCall) -> F + Send,
        F: Future<Output = Result<String, String>> + Send,
    {
        (**self).complete_with(request, handler)
    }
});

/// Deprecated alias for [`WasiProvider`](crate::WasiProvider).
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
#[deprecated(note = "use `omnia_sdk::WasiProvider`")]
pub type WasiModel = crate::WasiProvider;
