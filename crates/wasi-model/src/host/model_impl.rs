//! The `complete` host binding.
//!
//! Implements the generated `completion` host trait on [`WasiModel`]. It is the
//! host validation gate — it assembles the prompt (§3.1.1), hands the resulting
//! [`PreparedPrompt`] and a per-completion [`ToolHost`] to the backend, then
//! *re-validates* the returned answer before mapping it to the guest-visible
//! `answer` string. A backend that
//! runs its own repair loop (genai) consumes validation failures internally and
//! only returns once it passes; the host re-validates here.

use std::sync::Arc;

use anyhow::{Context as _, anyhow, bail};
use futures::FutureExt as _;
use omnia::{Dispatcher, GuestId, HasDispatcher, HasMounts};
use wasmtime::component::{Accessor, StreamReader, Val};

use crate::host::generated::augentic::model::completion::{
    Host, HostWithStore, Prompt, StreamEvent,
};
use crate::host::types::{Answer, PreparedPrompt, DirEntry, Reference, VerifyReport};
use crate::host::workspace::{self, Workspace};
use crate::host::{Error, FutureResult, ToolHost, WasiModel, WasiModelCtxView};

impl<T> HostWithStore<T> for WasiModel
where
    T: HasMounts + HasDispatcher,
{
    async fn complete(accessor: &Accessor<T, Self>, mut prompt: Prompt) -> Result<String, Error> {
        // The lent `borrow<descriptor>` cannot survive the backend await, so the
        // host takes it out here to resolve the workspace for `ToolHost`.
        let lent = prompt.grants.workspace.take();
        let request = PreparedPrompt::try_from(prompt)?;

        let kind = request.prompt.response_format.kind;
        let references = request.prompt.grants.references.clone();
        let verify_allowed = request.prompt.grants.verify.clone();

        let answer = accessor
            .with(|mut store| {
                let mounts = store.data_mut().mounts();
                let dispatcher = store.data_mut().dispatcher();
                let view = store.get();
                let workspace = workspace::resolve(view.table, &mounts, lent.as_ref())?;
                let tool_host: Arc<dyn ToolHost> = Arc::new(BoundToolHost {
                    dispatcher,
                    references,
                    verify_allowed,
                    workspace,
                });
                Ok::<_, Error>(view.ctx.complete(request, tool_host))
            })?
            .await?;

        Answer::check(&answer.value, kind)?;

        serde_json::to_string(&answer.value)
            .map_err(|e| Error::InvalidAnswer(format!("answer is not serializable JSON: {e}")))
    }

    async fn complete_stream(
        _accessor: &Accessor<T, Self>, _prompt: Prompt,
    ) -> Result<StreamReader<StreamEvent>, Error> {
        Err(Error::Backend("streaming unsupported".to_owned()))
    }
}

impl Host for WasiModelCtxView<'_> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}

// The bound tool host, built fresh per completion from the prompt's grants.
struct BoundToolHost {
    dispatcher: Arc<dyn Dispatcher>,
    references: Option<String>,
    verify_allowed: Vec<String>,
    workspace: Option<Workspace>,
}

// Convert a `resolve` export's return value into raw bytes.
fn vals_to_bytes(results: Vec<Val>) -> anyhow::Result<Vec<u8>> {
    let first = results.into_iter().next().context("resolve export returned no value")?;
    match first {
        Val::List(items) => items
            .into_iter()
            .map(|value| match value {
                Val::U8(byte) => Ok(byte),
                other => bail!("resolve result list element is not a u8: {other:?}"),
            })
            .collect(),
        Val::String(text) => Ok(text.into_bytes()),
        other => bail!("resolve export must return list<u8> or string, got {other:?}"),
    }
}

impl ToolHost for BoundToolHost {
    fn resolve(&self, reference: Reference) -> FutureResult<Vec<u8>> {
        let Some(target) = self.references.clone() else {
            return async move {
                Err(anyhow!("resolve(`{}`) requires grants.references", reference.name))
            }
            .boxed();
        };
        let dispatcher = Arc::clone(&self.dispatcher);
        async move {
            let results = dispatcher
                .invoke(
                    GuestId::from(target),
                    None,
                    "resolve".to_owned(),
                    vec![Val::String(reference.name)],
                )
                .await?;
            vals_to_bytes(results)
        }
        .boxed()
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

    fn verify(&self, check: String) -> FutureResult<VerifyReport> {
        let granted = self.verify_allowed.contains(&check);
        async move {
            if !granted {
                return Err(anyhow!("verify profile `{check}` is not in grants.verify"));
            }
            Ok(VerifyReport {
                ok: false,
                detail: format!(
                    "verify profile `{check}` is granted and routed; profile \
                     execution is not yet implemented"
                ),
            })
        }
        .boxed()
    }
}
