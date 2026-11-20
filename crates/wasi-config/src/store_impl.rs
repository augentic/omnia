use omnia_core::HostView;
use wasmtime::component::Accessor;

use crate::generated::omnia::config::store::{Host, HostWithStore};
use crate::{ConfigClient, Result, WasiConfig};

impl<T> HostWithStore<T> for WasiConfig {
    async fn get(accessor: &Accessor<T, Self>, key: String) -> Result<Option<String>> {
        Ok(accessor.with(|mut store| store.get().client.get(key)).await?)
    }
}

impl Host for HostView<'_, dyn ConfigClient> {}
