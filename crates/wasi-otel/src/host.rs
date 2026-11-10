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
use omnia_core::{Host, HostCtx, Provides, StoreCtx};
use wasmtime::component::Linker;

use self::generated::omnia::otel::{metrics, resource, tracing, types};

/// Host-side service for `omnia:otel`.
#[derive(Debug, HostCtx)]
#[host_ctx(client = OtelClient)]
pub struct WasiOtel;

impl<B: Provides<Self>> Host<B> for WasiOtel {
    fn add_to_linker(linker: &mut Linker<StoreCtx<B>>) -> anyhow::Result<()> {
        tracing::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        metrics::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        types::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?;
        Ok(resource::add_to_linker::<_, Self>(linker, StoreCtx::view::<Self>)?)
    }
}
