//! # WASI Model Service
//!
//! The host side of the `augentic:model/completion` boundary. Follows the
//! shared host-crate shape verbatim (`wasi-keyvalue` / `wasi-blobstore` are the
//! templates): a `WasiModel` host struct implementing `HasData` + `Host` +
//! (no-op) `Server`, a `WasiModelView` the `Linker<T>` type implements, a
//! `WasiModelCtxView` carrying the backend + resource table, and a
//! `WasiModelCtx` trait the *backend* implements. The one addition over a plain
//! effect host is the per-completion [`ToolHost`] handed to `complete`, which
//! `complete` assembles from the store's mounts and dispatcher rather than from
//! the view.

mod default_impl;
mod model_impl;
mod prompt;
mod replay;
mod types;
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
use omnia::{HasDispatcher, HasMounts, Host, Server};
use wasmtime::component::{HasData, Linker, ResourceTable};

pub use self::default_impl::{ConnectOptions, ModelDefault};
use self::generated::augentic::model::completion;
use self::generated::augentic::model::completion::Error;
pub use self::replay::Recording;
pub use self::types::{
    BackendAnswer, DirEntry, JsonSchemaSpec, PreparedPrompt, Prompt, Reference, ResponseFormat,
    ResponseFormatKind, Sections, ToolGrants, ToolTurn, Transcript, VerifyReport,
};

/// Host-side service for `wasi-model` (a linked-only effect host).
#[derive(Debug)]
pub struct WasiModel;

impl HasData for WasiModel {
    type Data<'a> = WasiModelCtxView<'a>;
}

impl<T> Host<T> for WasiModel
where
    T: WasiModelView + HasMounts + HasDispatcher + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        Ok(completion::add_to_linker::<_, Self>(linker, T::model)?)
    }
}

impl<B> Server<B> for WasiModel {}

/// A trait which provides internal WASI Model state. Implemented by the `T` in
/// `Linker<T>` during the runtime build.
pub trait WasiModelView: Send {
    /// Return a [`WasiModelCtxView`] from a mutable reference to self.
    fn model(&mut self) -> WasiModelCtxView<'_>;
}

/// View into a [`WasiModelCtx`] implementation and the [`ResourceTable`].
pub struct WasiModelCtxView<'a> {
    /// WASI Model context.
    pub ctx: &'a mut dyn WasiModelCtx,

    /// Resource table.
    pub table: &'a mut ResourceTable,
}

/// The backend trait — the one place a provider's logic lives.
pub trait WasiModelCtx: Debug + Send + Sync + 'static {
    /// Produce an answer for `request`, optionally lending the per-completion
    /// [`ToolHost`] to backends that drive an in-process tool loop.
    fn complete(
        &self, request: PreparedPrompt, tool_host: Arc<dyn ToolHost>,
    ) -> FutureResult<BackendAnswer>;
}

/// Forward the backend trait.
impl WasiModelCtx for Box<dyn WasiModelCtx> {
    fn complete(
        &self, request: PreparedPrompt, tool_host: Arc<dyn ToolHost>,
    ) -> FutureResult<BackendAnswer> {
        (**self).complete(request, tool_host)
    }
}

/// Host-side capabilities for one completion, lent to backends that need them.
pub trait ToolHost: Send + Sync {
    /// Host-mediated dynamic linking into the adapter's `references` export.
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
    fn local_path(&self) -> Option<&std::path::Path> {
        None
    }
}

/// An untyped host failure is a `backend` error at the boundary.
impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        Self::Backend(err.to_string())
    }
}

/// A trait which provides internal WASI Model context.
pub trait HasModel: Send {
    /// Borrow the `wasi-model` backend context.
    fn model_ctx(&mut self) -> &mut dyn WasiModelCtx;
}

impl<B: HasModel + Send + 'static> WasiModelView for omnia::StoreCtx<B> {
    fn model(&mut self) -> WasiModelCtxView<'_> {
        WasiModelCtxView {
            ctx: self.backends.model_ctx(),
            table: &mut self.base.table,
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
