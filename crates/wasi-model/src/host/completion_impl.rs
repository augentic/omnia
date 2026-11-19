//! The `completion` host bindings, and the session every completion begins.

use std::fmt;
use std::sync::Arc;

use anyhow::anyhow;
use futures::{FutureExt as _, future};
use omnia_core::{HostView, StoreCtx};
use wasmtime::component::{Access, Accessor, FutureReader, Resource, StreamReader};

use crate::host::generated::omnia::model::completion::{
    self, Host, HostModel, HostModelWithStore, HostWithStore, Session,
};
use crate::host::generated::omnia::model::mcp::Server;
use crate::host::generated::omnia::model::types::CallResult;
use crate::host::session::{
    CallsProducer, Halt, Replied, ReplyTask, ResultsConsumer, SessionClose, ToolSession,
};
use crate::host::tool_host::DirEntry;
use crate::host::workspace::{self, Workspace};
use crate::host::{
    Answer, Error, Exhausted, FutureResult, ModelClient, Request, ToolHost, ToolOutcome, WasiModel,
};

// The host's side of a guest's `model` handle: the name the guest opened,
// which its backend receives as `Request::model`.
#[derive(Debug)]
pub struct ModelHandle {
    name: String,
}

impl<B: Send + 'static> HostWithStore<StoreCtx<B>> for WasiModel {
    fn open(
        accessor: &Accessor<StoreCtx<B>, Self>, name: String,
    ) -> impl Future<Output = Result<Resource<ModelHandle>, Error>> {
        std::future::ready(if name.is_empty() {
            Err(Error::Invalid("a model name is empty".to_owned()))
        } else {
            accessor.with(|mut store| Ok(store.get().table.push(ModelHandle { name })?))
        })
    }
}

impl<B: Send + 'static> HostModelWithStore<StoreCtx<B>> for WasiModel {
    fn complete(
        accessor: &Accessor<StoreCtx<B>, Self>, self_: Resource<ModelHandle>,
        request: completion::Request, results: StreamReader<CallResult>,
    ) -> impl Future<Output = Result<Session, Error>> {
        std::future::ready(
            accessor
                .with(|mut access| Self::begin(&mut access, &self_, request, Vec::new(), results)),
        )
    }

    fn drop(
        mut accessor: Access<'_, StoreCtx<B>, Self>, rep: Resource<ModelHandle>,
    ) -> wasmtime::Result<()> {
        accessor.get().table.delete(rep)?;
        Ok(())
    }
}

impl WasiModel {
    // Begin a completion of `request` by the model `handle` names, with the
    // tools of `servers` beside the request's own. A request the host
    // refuses fails here, and closes `results`.
    pub(crate) fn begin<B: Send + 'static>(
        access: &mut Access<'_, StoreCtx<B>, Self>, handle: &Resource<ModelHandle>,
        request: completion::Request, servers: Vec<Server>, mut results: StreamReader<CallResult>,
    ) -> Result<Session, Error> {
        let (request, workspace) = match Self::admit(access, handle, request, servers) {
            Ok(admitted) => admitted,
            Err(error) => {
                results.close(&mut *access)?;
                return Err(error);
            }
        };

        // call model backend with request and tool host "closure"
        let limits = access.get().client.limits();
        let (session, calls_rx) = ToolSession::new(limits, request.tool_names());
        let tool_host: Arc<dyn ToolHost> = Arc::new(BoundToolHost {
            session: Arc::clone(&session),
            workspace,
        });
        let answer = access.get().client.complete(request, tool_host);

        results.pipe(&mut *access, ResultsConsumer::new(Arc::clone(&session)))?;
        let producer = CallsProducer::new(calls_rx, Arc::clone(&session));
        let mut calls = StreamReader::new(&mut *access, producer)?;

        // extract reply from answer, preferring the session's halt
        let close = SessionClose::new(Arc::clone(&session));
        let reply_task = ReplyTask::spawn(async move {
            let _close = close;
            let answered = answer.await;
            session.take_halt().map_or_else(|| reply(answered), Halt::into_reply)
        });

        // map the reply task to a future
        let reply_fut = reply_task.join().map(Ok::<_, wasmtime::Error>);
        let reply = match FutureReader::new(&mut *access, reply_fut) {
            Ok(reply) => reply,
            Err(error) => {
                calls.close(&mut *access)?;
                return Err(error.into());
            }
        };

        Ok(Session { calls, reply })
    }

    // The request as its backend receives it, and the workspace it lends.
    fn admit<B>(
        access: &mut Access<'_, StoreCtx<B>, Self>, handle: &Resource<ModelHandle>,
        request: completion::Request, servers: Vec<Server>,
    ) -> Result<(Request, Option<Workspace>), Error> {
        let name = access.get().table.get(handle)?.name.clone();
        let request = Request::new(name, request, servers)?;
        request.validate()?;

        let workspace = match &request.grants.workspace {
            Some(location) => {
                let mounts = access.data_mut().mounts();
                let descriptor = access.get().table.get(&location.root)?;
                Some(workspace::resolve(descriptor, &mounts, location)?)
            }
            None => None,
        };
        Ok((request, workspace))
    }
}

// The reply a backend's answer makes. An `Exhausted` is the `exhausted`
// outcome, and any other failure keeps its case or is `other`.
fn reply(answered: anyhow::Result<Answer>) -> Replied {
    match answered {
        Ok(answer) => Ok(Ok(answer.into())),
        Err(error) => match error.downcast::<Exhausted>() {
            Ok(exhausted) => Ok(Err(exhausted.into())),
            Err(error) => Err(error.into()),
        },
    }
}

impl Host for HostView<'_, dyn ModelClient> {}

impl HostModel for HostView<'_, dyn ModelClient> {}

// The bound tool host, built fresh per completion from the request's grants
// and the session channels its `complete` binding minted.
struct BoundToolHost {
    session: Arc<ToolSession>,
    workspace: Option<Workspace>,
}

impl BoundToolHost {
    // Run `op` against the lent workspace, or fail when none was granted.
    fn with_workspace<R: Send + 'static>(
        &self, op: &str, path: String, f: impl FnOnce(&Workspace, String) -> FutureResult<R>,
    ) -> FutureResult<R> {
        match &self.workspace {
            Some(workspace) => f(workspace, path),
            None => future::err(anyhow!("{op}(`{path}`) requires grants.workspace")).boxed(),
        }
    }
}

impl ToolHost for BoundToolHost {
    fn call_tool(&self, name: String, arguments: String) -> FutureResult<ToolOutcome> {
        Arc::clone(&self.session).call(name, arguments)
    }

    fn check(&self, candidate: String) -> FutureResult<Result<(), String>> {
        Arc::clone(&self.session).check(candidate)
    }

    fn read(&self, path: String) -> FutureResult<Vec<u8>> {
        self.with_workspace("read", path, Workspace::read)
    }

    fn list(&self, path: String) -> FutureResult<Vec<DirEntry>> {
        self.with_workspace("list", path, Workspace::list)
    }

    fn write(&self, path: String, bytes: Vec<u8>) -> FutureResult<()> {
        self.with_workspace("write", path, move |workspace, path| workspace.write(path, bytes))
    }

    fn local_path(&self) -> Option<&std::path::Path> {
        self.workspace.as_ref().map(Workspace::local_path)
    }
}

// Manual because the session and workspace internals (channels, capability
// handles) carry no useful state to print; the lent path is the identity.
impl fmt::Debug for BoundToolHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoundToolHost")
            .field("workspace", &self.workspace.as_ref().map(Workspace::local_path))
            .finish_non_exhaustive()
    }
}
