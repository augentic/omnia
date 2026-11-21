use omnia_core::HostView;
use wasmtime::component::{Accessor, Resource};

use crate::generated::omnia::vault::store::{Host, HostWithStore};
use crate::resource::LockerProxy;
use crate::{Result, VaultClient, WasiVault};

impl<T> HostWithStore<T> for WasiVault {
    async fn open(accessor: &Accessor<T, Self>, name: String) -> Result<Resource<LockerProxy>> {
        let locker = accessor.with(|mut store| store.get().client.open_locker(name)).await?;
        Ok(accessor.with(|mut store| store.get().table.push(omnia_core::Proxy(locker)))?)
    }
}

impl Host for HostView<'_, dyn VaultClient> {}
