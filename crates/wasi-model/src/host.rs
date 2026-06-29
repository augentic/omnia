//! # WASI Model Service
//!
//! The host side of the `augentic:model/completion` boundary. Follows the
//! shared host-crate shape verbatim (`wasi-keyvalue` / `wasi-blobstore` are the
//! templates): a `WasiModel` host struct implementing `HasData` + `Host` +
//! (no-op) `Server`, a `WasiModelView` the `Linker<T>` type implements, a
//! `WasiModelCtxView` carrying the backend + resource table, and a
//! `WasiModelCtx` trait the *backend* implements. The one addition over a plain
//! effect host is the per-completion [`ToolHost`] handed to `complete`.

mod default_impl;
mod model_impl;
mod replay;
mod types;
mod validate;
mod working_tree;

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
            // The working-tree `descriptor` (and its transitive `wasi:clocks`
            // dep) resolve to the p3 resources the runtime already owns via
            // `wasmtime_wasi::p3::add_to_linker`. We never add these to our
            // linker — `wasmtime-wasi` provides them; we only borrow the type.
            // p3 filesystem reads use native component-model `stream`/`future`,
            // so the p2 `wasi:io` remap is no longer pulled in.
            "wasi:clocks": wasmtime_wasi::p3::bindings::clocks,
            "wasi:filesystem": wasmtime_wasi::p3::bindings::filesystem,
        },
        trappable_error_type: {
            "augentic:model/completion.error" => Error,
        },
    });
}

use std::fmt::Debug;
use std::sync::Arc;

pub use omnia::FutureResult;
use omnia::{Host, HostDispatch, Runtime, Server, WorkingTreeRegistry};
use wasmtime::component::{HasData, Linker, ResourceTable};

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

// Hand-written rather than via `omnia::scaffold!`: the model view threads
// `host_dispatch` and `working_trees` from the store base, beyond the canonical
// `(ctx, table)` shape the macro generates.

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

    /// The host-side working-tree registry. The floor reads it to resolve a lent
    /// `grants.working-tree` `borrow<descriptor>` to an authorized mount by
    /// directory identity. Threaded in by `omnia_wasi_view!` from the store base;
    /// empty unless the deployment configures mounts.
    pub working_trees: &'a WorkingTreeRegistry,
}

/// The backend trait — the one place a provider's logic lives.
///
/// Implemented by [`ModelDefault`] (replay, in-tree) and by the model backends
/// in the `backends` repo (`omnia_genai::Client`, `omnia_cursor::Client`). It
/// carries no vendor type. `complete` receives the owned [`Prompt`] and a
/// host-built [`ToolHost`] — the latter is the only addition over a plain effect
/// Ctx, and it is just an argument, exactly like `open_bucket`'s `identifier`.
pub trait WasiModelCtx: Debug + Send + Sync + 'static {
    /// Produce an answer for `prompt`, optionally lending the per-completion
    /// [`ToolHost`] to backends that drive an in-process tool loop. The returned
    /// [`BackendAnswer`] is host-only (its transcript is for record/replay); the
    /// guest sees only the validated `answer` string the `complete` binding
    /// derives from it.
    fn complete(&self, prompt: Prompt, tool_host: Arc<dyn ToolHost>)
    -> FutureResult<BackendAnswer>;
}

/// Forward the backend trait through a boxed trait object so a backend bundle
/// can hold a swappable `Box<dyn WasiModelCtx>` field and still implement
/// [`HasModel`] — exactly what a record-vs-replay test runtime's bundle needs.
impl WasiModelCtx for Box<dyn WasiModelCtx> {
    fn complete(
        &self, prompt: Prompt, tool_host: Arc<dyn ToolHost>,
    ) -> FutureResult<BackendAnswer> {
        (**self).complete(prompt, tool_host)
    }
}

/// Host-side capabilities for one completion, lent to backends that need them.
///
/// Primarily the genai backend uses these: each method is a typed callback, and
/// genai turns model tool-calls into them. `ModelDefault` (replay) and the
/// cursor backend ignore it.
pub trait ToolHost: Send + Sync {
    /// `resolve` — host-mediated dynamic linking into the adapter's `references`
    /// export. Always a fresh instance: a resolve cannot recursively re-enter
    /// the guest that called `complete`.
    fn resolve(&self, reference: Reference) -> FutureResult<Vec<u8>>;

    /// Bounded working-tree read via the lent `wasi:filesystem` capability.
    fn read(&self, path: String) -> FutureResult<Vec<u8>>;

    /// Bounded working-tree listing via the lent `wasi:filesystem` capability.
    fn list(&self, path: String) -> FutureResult<Vec<DirEntry>>;

    /// Accumulate an edit against the session's base tree.
    fn write(&self, path: String, bytes: Vec<u8>) -> FutureResult<()>;

    /// Route a verify request to a closed profile.
    fn verify(&self, check: String) -> FutureResult<VerifyReport>;

    /// The absolute host path of the lent working tree, when one was lent for
    /// this completion and resolved to an authorized mount.
    ///
    /// Backends that spawn a node-local agent (cursor) source their
    /// `--workspace` from this. The default is `None` — no local tree, e.g.
    /// replay, or a node that does not carry the mount — which such a backend
    /// treats as "no local tree on this node".
    fn local_path(&self) -> Option<&std::path::Path> {
        None
    }
}

/// `anyhow::Error` to [`Error`] mapping: an untyped host failure is a
/// `backend` error at the boundary.
impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        Self::Backend(err.to_string())
    }
}

/// A backend bundle that can yield the `wasi-model` backend for a store.
///
/// The blanket [`WasiModelView`] impl below turns this accessor into the
/// linker-facing view on `omnia::StoreCtx<B>`, threading in the `host_dispatch`
/// and working-tree registry from the store base; the `runtime!` macro generates
/// the bundle-side impl via [`omnia_wasi_view!`].
pub trait HasModel: Send {
    /// Borrow the `wasi-model` backend context.
    fn model_ctx(&mut self) -> &mut dyn WasiModelCtx;
}

impl<B: HasModel + Send + 'static> WasiModelView for omnia::StoreCtx<B> {
    fn model(&mut self) -> WasiModelCtxView<'_> {
        WasiModelCtxView {
            ctx: self.backends.model_ctx(),
            table: &mut self.base.table,
            host_dispatch: Arc::clone(&self.base.host_dispatch),
            working_trees: &self.base.working_trees,
        }
    }
}

/// Generates the bundle's [`HasModel`] impl for a `runtime!` deployment.
#[macro_export]
macro_rules! omnia_wasi_view {
    ($bundle:ty, $field_name:ident) => {
        impl $crate::HasModel for $bundle {
            fn model_ctx(&mut self) -> &mut dyn $crate::WasiModelCtx {
                &mut self.$field_name
            }
        }
    };
}
