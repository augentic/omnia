use anyhow::Context;
use omnia_core::HostView;
use wasmtime::component::{Access, Accessor, Resource};

use crate::host::generated::omnia::vault::vault::Error;
use crate::host::resource::LockerProxy;
use crate::host::vault::{Host, HostLocker, HostLockerWithStore, HostWithStore};
use crate::host::{Result, VaultClient, WasiVault};

impl<T> HostWithStore<T> for WasiVault {
    async fn open(
        accessor: &Accessor<T, Self>, locker_id: String,
    ) -> Result<Resource<LockerProxy>> {
        let locker = accessor.with(|mut store| store.get().client.open_locker(locker_id)).await?;
        let proxy = omnia_core::Proxy(locker);
        Ok(accessor.with(|mut store| store.get().table.push(proxy))?)
    }
}

impl<T> HostLockerWithStore<T> for WasiVault {
    async fn get(
        accessor: &Accessor<T, Self>, self_: Resource<LockerProxy>, secret_id: String,
    ) -> Result<Option<Vec<u8>>> {
        let locker = get_locker(accessor, &self_)?;
        let value = locker.get(secret_id).await.context("issue getting value")?;
        Ok(value)
    }

    async fn set(
        accessor: &Accessor<T, Self>, self_: Resource<LockerProxy>, secret_id: String,
        value: Vec<u8>,
    ) -> Result<()> {
        let locker = get_locker(accessor, &self_)?;
        locker.set(secret_id, value).await.context("issue setting value")?;
        Ok(())
    }

    async fn delete(
        accessor: &Accessor<T, Self>, self_: Resource<LockerProxy>, secret_id: String,
    ) -> Result<()> {
        let locker = get_locker(accessor, &self_)?;
        locker.delete(secret_id).await.context("issue deleting value")?;
        Ok(())
    }

    async fn exists(
        accessor: &Accessor<T, Self>, self_: Resource<LockerProxy>, secret_id: String,
    ) -> Result<bool> {
        let locker = get_locker(accessor, &self_)?;
        let exists = locker.exists(secret_id).await.context("issue checking existence")?;
        Ok(exists)
    }

    async fn list_ids(
        accessor: &Accessor<T, Self>, self_: Resource<LockerProxy>,
    ) -> Result<Vec<String>> {
        let locker = get_locker(accessor, &self_)?;
        let secret_ids = locker.list_ids().await.context("issue getting value")?;
        Ok(secret_ids)
    }

    fn drop(mut accessor: Access<'_, T, Self>, rep: Resource<LockerProxy>) -> wasmtime::Result<()> {
        accessor.get().table.delete(rep).map(|_| Ok(()))?
    }
}

impl Host for HostView<'_, dyn VaultClient> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}

impl HostLocker for HostView<'_, dyn VaultClient> {}

pub fn get_locker<T>(
    accessor: &Accessor<T, WasiVault>, self_: &Resource<LockerProxy>,
) -> Result<LockerProxy> {
    Ok(omnia_core::get_cloned(accessor, self_)?)
}
