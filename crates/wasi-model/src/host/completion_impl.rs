//! The `create` host binding.
//!
//! Implements the generated `completion` host trait on [`WasiModel`]. It is
//! the host validation gate: it validates the request, mints the session
//! channels (calls stream, reply future) around the backend, pipes the
//! guest's results stream into them, and re-validates the final answer
//! before the reply future resolves. A backend that runs its own repair loop
//! (genai) consumes validation failures internally and only returns once it
//! passes; the host re-validates here.
//!
//! The reply pipeline runs eagerly as a spawned task, since a session guest
//! reads `calls` before awaiting the reply (see `session::ReplyTask`).
//! Cancellation stays structural: the guest dropping its reply future drops
//! the producer awaiting the task, which aborts it — no in-flight backend
//! work outlives the session. The reply future is always resolved with a
//! value; budget and deadline failures are typed `error` values, never a
//! dropped writer (dropping it would trap the guest).

use std::sync::Arc;

use anyhow::anyhow;
use futures::FutureExt as _;
use omnia::HasMounts;
use wasmtime::component::{Accessor, FutureReader, StreamReader};

use crate::host::generated::omnia::model::completion::{
    Host, HostWithStore, Session, Tool, ToolResult,
};
use crate::host::resource::DirEntry;
use crate::host::session::{CallsProducer, ReplyTask, ResultsConsumer, SessionClose, SessionState};
use crate::host::workspace::{self, Workspace};
use crate::host::{Error, FutureResult, Request, ToolHost, WasiModel, WasiModelCtxView, gate};

impl<T> HostWithStore<T> for WasiModel
where
    T: HasMounts,
{
    // The generated trait method is async; opening the session is entirely
    // synchronous store work (the backend runs behind the reply future).
    #[expect(clippy::unused_async_trait_impl)]
    async fn create(
        accessor: &Accessor<T, Self>, mut request: Request, mut results: StreamReader<ToolResult>,
    ) -> Result<Session, Error> {
        // The lent `borrow<descriptor>` cannot survive the backend await, so
        // the host takes it out here to resolve the workspace for `ToolHost`.
        let lent = request.grants.workspace.take();

        if let Err(error) = gate::validate(&request) {
            // A stream not returned to the guest must be disposed explicitly.
            accessor.with(|mut access| results.close(&mut access))?;
            return Err(error);
        }

        let format = request.format.clone();
        // The declared function tools are the only names `call-tool` accepts.
        let allowed: Vec<String> = request
            .tools
            .iter()
            .filter_map(|tool| match tool {
                Tool::Function(function) => Some(function.name.clone()),
                Tool::Mcp(_) => None,
            })
            .collect();

        accessor.with(|mut access| {
            let mounts = access.data_mut().mounts();
            let resolved = {
                let view = access.get();
                workspace::resolve(view.table, &mounts, lent.as_ref())
            };
            let workspace = match resolved {
                Ok(workspace) => workspace,
                Err(error) => {
                    results.close(&mut access)?;
                    return Err(error.into());
                }
            };

            let limits = access.get().ctx.limits();
            let (session, calls_rx) = SessionState::new(limits, allowed);
            let tool_host: Arc<dyn ToolHost> = Arc::new(BoundToolHost {
                session: Arc::clone(&session),
                workspace,
            });
            let backend = access.get().ctx.complete(request, tool_host);

            results.pipe(&mut access, ResultsConsumer::new(Arc::clone(&session)))?;
            let mut calls = StreamReader::new(&mut access, CallsProducer::new(calls_rx))?;

            // The reply pipeline — the backend future piped through the
            // answer gate — runs eagerly as a spawned task (see [`ReplyTask`]
            // for why). It always yields a value; a typed failure recorded by
            // host enforcement wins over whatever the backend then returned.
            // The guard ends the session — completion or cancellation alike —
            // so the guest's calls loop always terminates.
            let close = SessionClose::new(Arc::clone(&session));
            let task = ReplyTask::spawn(async move {
                let _close = close;
                match backend.await {
                    Ok(answer) => {
                        session.take_failure().map_or_else(|| answer.project(&format), Err)
                    }
                    Err(error) => Err(session.take_failure().unwrap_or_else(|| error.into())),
                }
            });
            let reply_future = async move { Ok::<_, wasmtime::Error>(task.join().await) };
            let reply = match FutureReader::new(&mut access, reply_future) {
                Ok(reply) => reply,
                Err(error) => {
                    calls.close(&mut access)?;
                    return Err(error.into());
                }
            };

            Ok(Session { calls, reply })
        })
    }
}

impl Host for WasiModelCtxView<'_> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}

// The bound tool host, built fresh per completion from the request's grants
// and the session channels the `create` binding minted.
struct BoundToolHost {
    session: Arc<SessionState>,
    workspace: Option<Workspace>,
}

impl ToolHost for BoundToolHost {
    fn call_tool(&self, name: String, arguments: String) -> FutureResult<Result<String, String>> {
        Arc::clone(&self.session).call(name, arguments)
    }

    fn read(&self, path: String) -> FutureResult<Vec<u8>> {
        let Some(workspace) = self.workspace.as_ref() else {
            return async move { Err(anyhow!("read(`{path}`) requires grants.workspace")) }.boxed();
        };
        workspace.read(path)
    }

    fn list(&self, path: String) -> FutureResult<Vec<DirEntry>> {
        let Some(workspace) = self.workspace.as_ref() else {
            return async move { Err(anyhow!("list(`{path}`) requires grants.workspace")) }.boxed();
        };
        workspace.list(path)
    }

    fn write(&self, path: String, bytes: Vec<u8>) -> FutureResult<()> {
        let Some(workspace) = self.workspace.as_ref() else {
            return async move { Err(anyhow!("write(`{path}`) requires grants.workspace")) }
                .boxed();
        };
        workspace.write(path, bytes)
    }

    fn local_path(&self) -> Option<&std::path::Path> {
        self.workspace.as_ref().map(Workspace::local_path)
    }
}
