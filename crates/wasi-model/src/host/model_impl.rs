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

use anyhow::{Context as _, bail};
use futures::FutureExt as _;
use omnia::{Dispatcher, GuestId, HasDispatcher, HasMounts};
use wasmtime::component::{Accessor, StreamReader, Val};

use super::gate::check_answer;
use super::generated::augentic::model::completion as genc;
use super::generated::augentic::model::completion::{Host, HostWithStore};
use super::types::{PreparedPrompt, Prompt};
use super::workspace::{self, Workspace};
use super::{Error, FutureResult, ToolHost, WasiModel, WasiModelCtxView};
use crate::host::types::{DirEntry, Reference, VerifyReport};

impl<T> HostWithStore<T> for WasiModel
where
    T: HasMounts + HasDispatcher,
{
    async fn complete(
        accessor: &Accessor<T, Self>, mut prompt: genc::Prompt,
    ) -> Result<String, Error> {
        let lent = prompt.grants.workspace.take();
        let mut owned: Prompt = prompt.into();
        owned.grants.workspace_lent = lent.is_some();

        let request = PreparedPrompt::try_from(owned)?;

        let kind = request.prompt.response_format.kind;
        let references = request.prompt.grants.references.clone();
        let verify_allowed = request.prompt.grants.verify.clone();

        let backend_answer = accessor
            .with(|mut store| {
                // Clone the store-level handles out before borrowing the view:
                // `store.get()` reborrows the store mutably, so the mount
                // registry and dispatcher cannot be held as references across it.
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

        check_answer(&backend_answer.value, kind)?;

        serde_json::to_string(&backend_answer.value)
            .map_err(|e| Error::InvalidAnswer(format!("answer is not serializable JSON: {e}")))
    }

    async fn complete_stream(
        _accessor: &Accessor<T, Self>, _prompt: genc::Prompt,
    ) -> Result<StreamReader<genc::StreamEvent>, Error> {
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

// Export-function name a `references` exposes for host-mediated `resolve`.
const RESOLVE_FUNC: &str = "resolve";

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
                Err(anyhow::anyhow!(
                    "resolve(`{}`) requires grants.references, but none was granted",
                    reference.name
                ))
            }
            .boxed();
        };
        let dispatcher = Arc::clone(&self.dispatcher);
        async move {
            let results = dispatcher
                .invoke(
                    GuestId::from(target),
                    None,
                    RESOLVE_FUNC.to_owned(),
                    vec![Val::String(reference.name)],
                )
                .await?;
            vals_to_bytes(results)
        }
        .boxed()
    }

    fn read(&self, path: String) -> FutureResult<Vec<u8>> {
        workspace::with_workspace(self.workspace.as_ref(), |ws| ws.read(path))
    }

    fn list(&self, path: String) -> FutureResult<Vec<DirEntry>> {
        workspace::with_workspace(self.workspace.as_ref(), |ws| ws.list(path))
    }

    fn write(&self, path: String, bytes: Vec<u8>) -> FutureResult<()> {
        workspace::with_workspace(self.workspace.as_ref(), |ws| ws.write(path, bytes))
    }

    fn local_path(&self) -> Option<&std::path::Path> {
        self.workspace.as_ref().map(Workspace::local_path)
    }

    fn verify(&self, check: String) -> FutureResult<VerifyReport> {
        let granted = self.verify_allowed.contains(&check);
        async move {
            if !granted {
                return Err(anyhow::anyhow!("verify profile `{check}` is not in grants.verify"));
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
