use anyhow::Context;
use wasmtime::component::{Access, Accessor, Resource};

use crate::host::generated::wasi::vault::vault::Error;
use crate::host::resource::LockerProxy;
use crate::host::vault::{Host, HostLocker, HostLockerWithStore, HostWithStore};
use crate::host::{Result, WasiVault, WasiVaultCtxView};

impl HostWithStore for WasiVault {
    async fn open<T>(
        accessor: &Accessor<T, Self>, locker_id: String,
    ) -> Result<Resource<LockerProxy>> {
        let locker = accessor.with(|mut store| store.get().ctx.open_locker(locker_id)).await?;
        let proxy = LockerProxy(locker);
        Ok(accessor.with(|mut store| store.get().table.push(proxy))?)
    }
}

impl HostLockerWithStore for WasiVault {
    async fn get<T>(
        accessor: &Accessor<T, Self>, self_: Resource<LockerProxy>, secret_id: String,
    ) -> Result<Option<Vec<u8>>> {
        let locker = get_locker(accessor, &self_)?;
        let value = locker.get(secret_id).await.context("issue getting value")?;
        Ok(value)
    }

    async fn set<T>(
        accessor: &Accessor<T, Self>, self_: Resource<LockerProxy>, secret_id: String,
        value: Vec<u8>,
    ) -> Result<(), Error> {
        let locker = get_locker(accessor, &self_)?;
        locker.set(secret_id, value).await.context("issue setting value")?;
        Ok(())
    }

    async fn delete<T>(
        accessor: &Accessor<T, Self>, self_: Resource<LockerProxy>, secret_id: String,
    ) -> Result<()> {
        let locker = get_locker(accessor, &self_)?;
        locker.delete(secret_id).await.context("issue deleting value")?;
        Ok(())
    }

    async fn exists<T>(
        accessor: &Accessor<T, Self>, self_: Resource<LockerProxy>, secret_id: String,
    ) -> Result<bool> {
        let locker = get_locker(accessor, &self_)?;
        let value = locker.get(secret_id).await.context("issue getting value")?;
        Ok(value.is_some())
    }

    async fn list_ids<T>(
        accessor: &Accessor<T, Self>, self_: Resource<LockerProxy>,
    ) -> Result<Vec<String>> {
        let locker = get_locker(accessor, &self_)?;
        let secret_ids = locker.list_ids().await.context("issue getting value")?;
        Ok(secret_ids)
    }

    fn drop<T>(
        mut accessor: Access<'_, T, Self>, rep: Resource<LockerProxy>,
    ) -> wasmtime::Result<()> {
        accessor.get().table.delete(rep).map(|_| Ok(()))?
    }
}

impl Host for WasiVaultCtxView<'_> {
    fn convert_error(&mut self, err: Error) -> wasmtime::Result<Error> {
        Ok(err)
    }
}

impl HostLocker for WasiVaultCtxView<'_> {}

pub fn get_locker<T>(
    accessor: &Accessor<T, WasiVault>, self_: &Resource<LockerProxy>,
) -> Result<LockerProxy> {
    accessor.with(|mut store| {
        let locker = store.get().table.get(self_)?;
        Ok::<_, Error>(locker.clone())
    })
}
