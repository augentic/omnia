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

use anyhow::{Context as _, bail};
use futures::FutureExt as _;
use omnia::{GuestId, HostDispatch};
use wasmtime::component::{Accessor, StreamReader, Val};

use super::generated::augentic::model::completion as genc;
use super::generated::augentic::model::completion::{Host, HostWithStore};
use super::types::Prompt;
use super::validate::{check_prompt, validate_answer};
use super::{Error, FutureResult, ToolHost, WasiModel, WasiModelCtxView};
use crate::host::types::{DirEntry, Reference, VerifyReport};

impl<T> HostWithStore<T> for WasiModel {
    async fn complete(accessor: &Accessor<T, Self>, prompt: genc::Prompt) -> Result<String, Error> {
        let owned: Prompt = prompt.into();

        // Floor pre-checks: reserved tool names and empty prompts never reach a
        // backend (§3.1.1–§3.1.2).
        check_prompt(&owned)?;

        // `kind` is needed for the final validation gate after the backend
        // returns; capture it before the owned prompt moves into the backend.
        let kind = owned.response_format.kind;

        // Build the per-completion tool host (§4.2) from the prompt's grants.
        // `resolve` reaches the host→guest dispatcher threaded into the store
        // ctx; `verify` is routing-only; `read`/`list`/`write` are deferred to
        // Phase 2b. `ModelDefault` (replay) ignores it. Capture the grants the
        // host needs before the owned prompt moves into the backend.
        let references = owned.grants.references.clone();
        let verify_allowed = owned.grants.verify.clone();

        let backend_answer = accessor
            .with(|mut store| {
                let view = store.get();
                let tool_host: Arc<dyn ToolHost> = Arc::new(BoundToolHost {
                    dispatch: Arc::clone(&view.host_dispatch),
                    references,
                    verify_allowed,
                });
                view.ctx.complete(owned, tool_host)
            })
            .await?;

        // Final validation gate: a backend answer that does not validate is a
        // backend failure, never a guest-visible answer (§3.1.3).
        validate_answer(&backend_answer.value, kind)?;

        serde_json::to_string(&backend_answer.value)
            .map_err(|e| Error::InvalidAnswer(format!("answer is not serializable JSON: {e}")))
    }

    async fn complete_stream(
        _accessor: &Accessor<T, Self>, _prompt: genc::Prompt,
    ) -> Result<StreamReader<genc::StreamEvent>, Error> {
        // The binding is generated so `bindgen!` is confirmed to compile the
        // native `stream<>` type at the 0.1.0 boundary (§3.1.4); host-side
        // stream production lands in Phase 3.
        Err(Error::Backend("streaming unsupported".to_owned()))
    }
}

impl Host for WasiModelCtxView<'_> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}

/// The Phase 2a floor tool host, built fresh per completion from the prompt's
/// grants. `resolve` is wired to the host→guest dispatcher (§4.1); `verify` is
/// routing-only (an allow-list check against `grants.verify` — profiles and
/// their execution are RFC-60); `read`/`list`/`write` stay loud stubs until the
/// wasi-filesystem working tree lands (Phase 2b, RFC-55), as does the
/// `wasi:keyvalue` cross-turn session state that backs their visibility.
/// `ModelDefault` (replay) ignores the whole host.
struct BoundToolHost {
    /// Type-erased host→guest dispatcher threaded in via the store ctx.
    dispatch: Arc<dyn HostDispatch>,
    /// `grants.references`: the guest id whose `references` shelf `resolve`
    /// targets. `None` keeps `resolve` failing loudly (no reference granted).
    references: Option<String>,
    /// `grants.verify`: the closed verification profiles the model may route to.
    verify_allowed: Vec<String>,
}

/// A future that immediately fails because a capability lands in Phase 2b.
fn deferred<R: Send + 'static>(tool: &'static str) -> FutureResult<R> {
    async move {
        Err(anyhow::anyhow!("tool `{tool}` is not wired until Phase 2b (RFC-55 working tree)"))
    }
    .boxed()
}

/// The conventional export-function name a `references` shelf exposes for
/// host-mediated `resolve` (`examples/model/wit/world.wit` invokes it by
/// convention, without naming the interface). `wasi-model` owns this verb name
/// and the bytes shape below; the floor's [`HostDispatch::invoke`] stays generic.
const RESOLVE_FUNC: &str = "resolve";

/// Convert a `resolve` export's return value into raw bytes. Accepts `list<u8>`
/// (the canonical shape) or `string` (a convenience for text shelves).
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
        // `resolve` is only valid when the prompt granted a reference target.
        let Some(target) = self.references.clone() else {
            return async move {
                Err(anyhow::anyhow!(
                    "resolve(`{}`) requires grants.references, but none was granted",
                    reference.name
                ))
            }
            .boxed();
        };
        let dispatch = Arc::clone(&self.dispatch);
        async move {
            let results = dispatch
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

    fn read(&self, _path: String) -> FutureResult<Vec<u8>> {
        deferred("read")
    }

    fn list(&self, _path: String) -> FutureResult<Vec<DirEntry>> {
        deferred("list")
    }

    fn write(&self, _path: String, _bytes: Vec<u8>) -> FutureResult<()> {
        deferred("write")
    }

    fn verify(&self, check: String) -> FutureResult<VerifyReport> {
        // Routing-only: validate the requested profile is in `grants.verify`,
        // then acknowledge the route. Profile definitions, sandboxing, and
        // execution are owned by RFC-60 and are not implemented here.
        let granted = self.verify_allowed.contains(&check);
        async move {
            if !granted {
                return Err(anyhow::anyhow!("verify profile `{check}` is not in grants.verify"));
            }
            Ok(VerifyReport {
                ok: false,
                detail: format!(
                    "verify profile `{check}` is granted and routed; profile \
                     execution is RFC-60 (not yet implemented)"
                ),
            })
        }
        .boxed()
    }
}
