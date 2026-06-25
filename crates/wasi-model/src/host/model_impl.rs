//! The `complete` host binding (§3.4).
//!
//! This is the analogue of `wasi-keyvalue/src/host/store_impl.rs`: it implements
//! the generated `completion` host trait on [`WasiModel`]. It is the floor's
//! final gate — it applies the pre-checks (§3.1.1–§3.1.2), hands the owned prompt
//! and a per-completion [`ToolHost`] to the backend, then *re-validates* the
//! returned answer (§3.1.3) before mapping it to the guest-visible `answer`
//! string. A backend that runs its own repair loop (genai) consumes validation
//! failures internally and only returns once it passes; the floor re-gates here.

use std::sync::Arc;

use futures::FutureExt as _;
use wasmtime::component::{Accessor, StreamReader};

use super::generated::augentic::model::completion as genc;
use super::generated::augentic::model::completion::{Host, HostWithStore};
use super::types::Prompt;
use super::validate::{check_prompt, validate_answer};
use super::{Error, FutureResult, ToolHost, WasiModel, WasiModelCtxView};
use crate::host::types::{DirEntry, Reference, VerifyReport};

impl<T> HostWithStore<T> for WasiModel {
    async fn complete(
        accessor: &Accessor<T, Self>, prompt: genc::Prompt,
    ) -> Result<String, Error> {
        let owned: Prompt = prompt.into();

        // Floor pre-checks: reserved tool names and empty prompts never reach a
        // backend (§3.1.1–§3.1.2).
        check_prompt(&owned)?;

        // `kind` is needed for the final validation gate after the backend
        // returns; capture it before the owned prompt moves into the backend.
        let kind = owned.response_format.kind;

        // Build the per-completion tool host. Phase 1 lends an unbound host:
        // `resolve` is wired in Phase 2a, `read`/`list`/`write` in Phase 2b.
        // `ModelDefault` (replay) ignores it.
        let tool_host: Arc<dyn ToolHost> = Arc::new(UnboundToolHost);

        let backend_answer =
            accessor.with(|mut store| store.get().ctx.complete(owned, tool_host)).await?;

        // Final validation gate: a backend answer that does not validate is a
        // backend failure, never a guest-visible answer (§3.1.3).
        validate_answer(&backend_answer.value, kind)?;

        serde_json::to_string(&backend_answer.value)
            .map_err(|e| Error::InvalidAnswer(format!("answer is not serializable JSON: {e}")))
    }

    async fn complete_stream(
        _accessor: &Accessor<T, Self>, _prompt: genc::Prompt,
    ) -> Result<StreamReader<genc::StreamEvent>, Error> {
        // The binding is generated so the 0.1.0 boundary is final and `bindgen!`
        // is confirmed to compile the native `stream<>` type (§3.1.4); host-side
        // stream production lands in Phase 3.
        Err(Error::Backend("streaming unsupported".to_owned()))
    }
}

impl Host for WasiModelCtxView<'_> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}

/// The Phase 1 floor tool host: defined so the [`ToolHost`] surface is final,
/// but bound to nothing yet. Every capability fails loudly until the registry
/// (`resolve`, Phase 2a) and working tree (`read`/`list`/`write`, Phase 2b) are
/// wired. `ModelDefault` ignores it, so the Phase 1 replay path never calls it.
#[derive(Debug)]
struct UnboundToolHost;

/// A future that immediately fails with an "unwired in Phase 1" error.
fn unwired<R: Send + 'static>(tool: &'static str) -> FutureResult<R> {
    async move { Err(anyhow::anyhow!("tool `{tool}` is not wired in phase 1")) }.boxed()
}

impl ToolHost for UnboundToolHost {
    fn resolve(&self, _reference: Reference) -> FutureResult<Vec<u8>> {
        unwired("resolve")
    }

    fn read(&self, _path: String) -> FutureResult<Vec<u8>> {
        unwired("read")
    }

    fn list(&self, _path: String) -> FutureResult<Vec<DirEntry>> {
        unwired("list")
    }

    fn write(&self, _path: String, _bytes: Vec<u8>) -> FutureResult<()> {
        unwired("write")
    }

    fn verify(&self, _check: String) -> FutureResult<VerifyReport> {
        unwired("verify")
    }
}
