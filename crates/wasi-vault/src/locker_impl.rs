use omnia_core::HostView;
use wasmtime::component::{Access, Accessor, Resource};

use crate::host::generated::omnia::vault::locker::{Host, HostLocker, HostLockerWithStore, Names};
use crate::host::resource::LockerProxy;
use crate::host::{Error, Result, VaultClient, WasiVault};

const PAGE_LIMIT: u32 = 1_000;

impl<T> HostLockerWithStore<T> for WasiVault {
    async fn get(
        accessor: &Accessor<T, Self>, self_: Resource<LockerProxy>, name: String,
    ) -> Result<Option<Vec<u8>>> {
        Ok(get_locker(accessor, &self_)?.get(name).await?)
    }

    async fn exists(
        accessor: &Accessor<T, Self>, self_: Resource<LockerProxy>, name: String,
    ) -> Result<bool> {
        Ok(get_locker(accessor, &self_)?.exists(name).await?)
    }

    async fn set(
        accessor: &Accessor<T, Self>, self_: Resource<LockerProxy>, name: String, value: Vec<u8>,
    ) -> Result<()> {
        Ok(get_locker(accessor, &self_)?.set(name, value).await?)
    }

    async fn delete(
        accessor: &Accessor<T, Self>, self_: Resource<LockerProxy>, name: String,
    ) -> Result<()> {
        Ok(get_locker(accessor, &self_)?.delete(name).await?)
    }

    async fn list_secrets(
        accessor: &Accessor<T, Self>, self_: Resource<LockerProxy>, prefix: String,
        after: Option<String>, limit: Option<u32>,
    ) -> Result<Names> {
        let limit = match limit {
            Some(0) => return Err(Error::Invalid("a listing limit must be at least 1".to_owned())),
            Some(limit) => limit.min(PAGE_LIMIT) as usize,
            None => PAGE_LIMIT as usize,
        };
        let names = get_locker(accessor, &self_)?.list_ids().await?;
        Ok(page(names, &prefix, after.as_deref(), limit))
    }

    fn drop(mut accessor: Access<'_, T, Self>, rep: Resource<LockerProxy>) -> wasmtime::Result<()> {
        accessor.get().table.delete(rep)?;
        Ok(())
    }
}

fn get_locker<T>(
    accessor: &Accessor<T, WasiVault>, locker: &Resource<LockerProxy>,
) -> Result<LockerProxy> {
    Ok(omnia_core::get_cloned(accessor, locker)?)
}

// A backend lists every name at once, so the host filters, orders, and pages.
fn page(mut names: Vec<String>, prefix: &str, after: Option<&str>, limit: usize) -> Names {
    names
        .retain(|name| name.starts_with(prefix) && after.is_none_or(|after| name.as_str() > after));
    names.sort_unstable();
    let next = (names.len() > limit).then(|| names[limit - 1].clone());
    names.truncate(limit);
    Names { secrets: names, next }
}

impl Host for HostView<'_, dyn VaultClient> {}

impl HostLocker for HostView<'_, dyn VaultClient> {}
