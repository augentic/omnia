//! Host surface for `omnia:plugins`.

mod loader_impl;
mod types_impl;

mod generated {
    pub use omnia_wasi_types::Error;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: store | tracing | trappable,
        },
        with: {
            "omnia:types/types": omnia_wasi_types::types,
        },
        trappable_error_type: {
            "omnia:types/types.error" => Error,
        },
    });
}

use std::sync::Arc;

use omnia_core::{Backends, Host, StoreCtx};
use wasmtime::component::{HasData, Linker};

use self::generated::omnia::plugins::{loader, types};
use crate::loader::Plugins;

/// Host-side service for `omnia:plugins` — the loader capability this crate
/// implements over the runtime's admission seam.
#[derive(Debug)]
pub struct WasiPlugins;

impl HasData for WasiPlugins {
    type Data<'a> = WasiPluginsView;
}

impl<B: Backends> Host<B> for WasiPlugins {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        omnia_wasi_types::types::add_to_linker::<_, Self>(linker, view)?;
        types::add_to_linker::<_, Self>(linker, view)?;
        Ok(loader::add_to_linker::<_, Self>(linker, view)?)
    }
}

fn view<B: Backends>(store: &mut StoreCtx<B>) -> WasiPluginsView {
    WasiPluginsView {
        plugins: store.extensions().get::<Plugins>(),
    }
}

/// View over the store's plugin-load capability.
pub struct WasiPluginsView {
    /// The runtime's plugin-load capability; `None` when the deployment
    /// installed no [`Plugins`] extension, where every load answers
    /// `unsupported`.
    pub plugins: Option<Arc<Plugins>>,
}
