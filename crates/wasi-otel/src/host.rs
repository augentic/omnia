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

pub use omnia_backend::otel::{OtelClient, OtelDefault};
pub use omnia_core::FutureResult;
use omnia_core::{Host, Server, StoreView};
use wasmtime::component::{HasData, Linker};

use self::generated::omnia::otel::{metrics, resource, tracing, types};

/// Host-side service for `wasi:otel`.
#[derive(Debug)]
pub struct WasiOtel;

impl HasData for WasiOtel {
    type Data<'a> = WasiOtelView<'a>;
}

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

omnia_core::wasi_view!(Otel);
