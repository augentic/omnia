mod default_impl;
mod metrics_impl;
mod resource_impl;
mod tracing_impl;
mod types_impl;

mod generated {
    pub use self::omnia::otel::types::Error;

    wasmtime::component::bindgen!({
        world: "imports",
        path: "wit",
        imports: {
            default: tracing | trappable,
        },
        with: {
            "wasi:clocks": wasmtime_wasi::p2::bindings::clocks,
        },
        trappable_error_type: {
            "omnia:otel/types.error" => Error,
        }
    });
}

use std::fmt::Debug;

pub use default_impl::OtelDefault;
pub use omnia_backend::otel::OtelClient;
pub use omnia_core::FutureResult;
use omnia_core::{Host, HostCtx, Server, StoreView};
use wasmtime::component::Linker;

use self::generated::omnia::otel::{metrics, resource, tracing, types};

/// Host-side service for `wasi:otel`.
#[derive(Debug, HostCtx)]
pub struct WasiOtel;

impl<T> Host<T> for WasiOtel
where
    T: StoreView<Self> + 'static,
{
    fn add_to_linker(linker: &mut Linker<T>) -> anyhow::Result<()> {
        tracing::add_to_linker::<_, Self>(linker, T::view)?;
        metrics::add_to_linker::<_, Self>(linker, T::view)?;
        types::add_to_linker::<_, Self>(linker, T::view)?;
        Ok(resource::add_to_linker::<_, Self>(linker, T::view)?)
    }
}

impl<B> Server<B> for WasiOtel {}
