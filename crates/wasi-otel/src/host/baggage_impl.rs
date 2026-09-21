//! Host side of `omnia:otel/baggage`: the calling guest's chain metadata,
//! read and extended on its store as its baggage.

use omnia_core::HasChain;
use wasmtime::component::Access;

use crate::WasiOtel;
use crate::host::WasiOtelCtxView;
use crate::host::generated::omnia::otel::baggage::{self as wasi, HostWithStore};

impl<T> HostWithStore<T> for WasiOtel
where
    T: HasChain + 'static,
{
    fn baggage(mut host: Access<T, Self>) -> wasmtime::Result<Vec<(String, String)>> {
        Ok(host
            .data_mut()
            .chain()
            .metadata()
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect())
    }

    fn set_baggage(
        mut host: Access<T, Self>, entries: Vec<(String, String)>,
    ) -> wasmtime::Result<()> {
        host.data_mut().chain_mut().metadata_mut().extend(entries);
        Ok(())
    }
}

impl wasi::Host for WasiOtelCtxView<'_> {}
