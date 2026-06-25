//! # WASI Model Service
//!
//! The host side of the `augentic:model/completion` boundary. Follows the
//! shared host-crate shape verbatim (`wasi-keyvalue` / `wasi-blobstore` are the
//! templates): a `WasiModel` host struct implementing `HasData` + `Host` +
//! (no-op) `Server`, a `WasiModelView` the `Linker<T>` type implements, a
//! `WasiModelCtxView` carrying the backend + resource table, and a
//! `WasiModelCtx` trait the *backend* implements. The one addition over a plain
//! effect host is the per-completion [`ToolHost`] handed to `complete` (§4.2).

mod default_impl;
mod model_impl;
mod replay;
mod types;
mod validate;

mod generated {
    #![allow(missing_docs)]

    pub use self::augentic::model::completion::Error;

    wasmtime::component::bindgen!({
        world: "model",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            // The working-tree `descriptor` (and its transitive deps) resolve to
            // the resources the runtime already owns, exactly as `wasi-blobstore`
            // remaps `wasi:io`. We never add these to our linker — `wasmtime-wasi`
            // provides them; we only borrow the type.
            "wasi:io": wasmtime_wasi::p2::bindings::io,
            "wasi:clocks": wasmtime_wasi::p2::bindings::clocks,
            "wasi:filesystem": wasmtime_wasi::p2::bindings::filesystem,
        },
        trappable_error_type: {
            "augentic:model/completion.error" => Error,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use omnia::FutureResult;
use omnia::{Host, HostDispatch, Runtime, Server};
use wasmtime::component::{HasData, Linker};
use wasmtime_wasi::ResourceTable;

pub use self::default_impl::{ConnectOptions, ModelDefault};
use self::generated::augentic::model::completion;
pub use self::generated::augentic::model::completion::Error;
pub use self::replay::{Fixture, FixtureStore, Recording, canonical_key, write_fixture};
pub use self::types::{
    BackendAnswer, DirEntry, Example, FunctionTool, GenerationParams, JsonSchemaSpec, Message,
    MetadataEntry, Prompt, Reference, ResponseFormat, ResponseFormatKind, Sections, ToolChoice,
    ToolGrants, ToolTurn, Transcript, Variable, VerifyReport,
};
pub use self::validate::{Assembled, RESERVED_TOOL_NAMES, assemble, check_prompt, validate_answer};

/// Host-side service for `wasi-model` (a linked-only effect host).
#[derive(Debug)]
pub struct WasiModel;

impl HasData for WasiModel {
    type Data<'a> = WasiModelCtxView<'a>;
}

impl<T> Host<T> for WasiModel
where
    T: WasiModelView + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        Ok(completion::add_to_linker::<_, Self>(linker, T::model)?)
    }
}

impl<R> Server<R> for WasiModel where R: Runtime {}

/// A trait which provides internal WASI Model state.
///
/// This is implemented by the `T` in `Linker<T>` — a single type shared across
/// all WASI components for the runtime build.
pub trait WasiModelView: Send {
    /// Return a [`WasiModelCtxView`] from a mutable reference to self.
    fn model(&mut self) -> WasiModelCtxView<'_>;
}

/// View into a [`WasiModelCtx`] implementation and the [`ResourceTable`].
pub struct WasiModelCtxView<'a> {
    /// Mutable reference to the WASI Model context (the backend).
    pub ctx: &'a mut dyn WasiModelCtx,

    /// Mutable reference to the table used to manage resources.
    pub table: &'a mut ResourceTable,

    /// Type-erased host→guest dispatcher, used by [`ToolHost::resolve`] to reach
    /// an adapter's `references` shelf. Threaded in by the `runtime!` macro's
    /// store context (inert for backends that never resolve).
    pub host_dispatch: Arc<dyn HostDispatch>,
}

/// The backend trait — the one place a provider's logic lives.
///
/// Implemented by [`ModelDefault`] (replay, in-tree) and by the model backends
/// in the `backends` repo (`omnia_genai::Client`, `omnia_cursor::Client`). It
/// carries no vendor type. `complete` receives the owned [`Prompt`] and a
/// host-built [`ToolHost`] (§4.2) — the latter is the only addition over a plain
/// effect Ctx, and it is just an argument, exactly like `open_bucket`'s
/// `identifier`.
pub trait WasiModelCtx: Debug + Send + Sync + 'static {
    /// Produce an answer for `prompt`, optionally lending the per-completion
    /// [`ToolHost`] to backends that drive an in-process tool loop. The returned
    /// [`BackendAnswer`] is host-only (its transcript is for record/replay); the
    /// guest sees only the validated `answer` string the `complete` binding
    /// derives from it.
    fn complete(&self, prompt: Prompt, tool_host: Arc<dyn ToolHost>)
    -> FutureResult<BackendAnswer>;
}

/// Host-side capabilities for one completion, lent to backends that need them.
///
/// Primarily the genai backend (RFC-59) uses these: each method is a typed
/// callback, and genai turns model tool-calls into them. `ModelDefault` (replay)
/// and the cursor backend ignore it.
///
/// Phase 1 defines the surface (the boundary is final) but wires no capability:
/// `resolve` binds to the guest registry in Phase 2a, and `read`/`list`/`write`
/// to the wasi-filesystem working tree in Phase 2b (RFC-55). Until then the
/// floor lends a [`model_impl`]-built host that fails every call loudly.
pub trait ToolHost: Send + Sync {
    /// `resolve` — host-mediated dynamic linking into the adapter's `references`
    /// export (guest-registry.md §4). Always a fresh instance: a resolve cannot
    /// recursively re-enter the guest that called `complete`.
    fn resolve(&self, reference: Reference) -> FutureResult<Vec<u8>>;

    /// Bounded working-tree read via the lent `wasi:filesystem` capability.
    fn read(&self, path: String) -> FutureResult<Vec<u8>>;

    /// Bounded working-tree listing via the lent `wasi:filesystem` capability.
    fn list(&self, path: String) -> FutureResult<Vec<DirEntry>>;

    /// Accumulate an edit against the session's base tree.
    fn write(&self, path: String, bytes: Vec<u8>) -> FutureResult<()>;

    /// Route a verify request to a closed profile (RFC-60).
    fn verify(&self, check: String) -> FutureResult<VerifyReport>;
}

/// `anyhow::Error` to [`Error`] mapping: an untyped host failure is a
/// `backend` error at the boundary.
impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        Self::Backend(err.to_string())
    }
}

/// Implementation of the `WasiModelView` trait for the store context.
#[macro_export]
macro_rules! omnia_wasi_view {
    ($store_ctx:ty, $field_name:ident) => {
        impl omnia_wasi_model::WasiModelView for $store_ctx {
            fn model(&mut self) -> omnia_wasi_model::WasiModelCtxView<'_> {
                omnia_wasi_model::WasiModelCtxView {
                    ctx: &mut self.$field_name,
                    table: &mut self.table,
                    host_dispatch: ::std::sync::Arc::clone(&self.host_dispatch),
                }
            }
        }
    };
}
