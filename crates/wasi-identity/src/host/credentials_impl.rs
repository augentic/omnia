use anyhow::Context;
use wasmtime::component::{Access, Accessor, Resource};

use crate::host::generated::omnia::identity::credentials::{
    AccessToken, Host, HostIdentity, HostIdentityWithStore, HostWithStore,
};
use crate::host::generated::omnia::identity::types::Error;
use crate::host::resource::IdentityProxy;
use crate::host::{Result, WasiIdentity, WasiIdentityCtxView};

impl<T> HostWithStore<T> for WasiIdentity {
    async fn get_identity(
        accessor: &Accessor<T, Self>, name: String,
    ) -> Result<Resource<IdentityProxy>> {
        let identity = accessor.with(|mut store| store.get().ctx.get_identity(name)).await?;
        let proxy = IdentityProxy(identity);
        Ok(accessor.with(|mut store| store.get().table.push(proxy))?)
    }
}

impl<T> HostIdentityWithStore<T> for WasiIdentity {
    async fn get_token(
        accessor: &Accessor<T, Self>, self_: Resource<IdentityProxy>, scopes: Vec<String>,
    ) -> Result<AccessToken> {
        let identity = accessor.with(|mut store| {
            store.get().table.get(&self_).cloned().map_err(|_e| Error::NoSuchIdentity)
        })?;

        let token = identity.0.get_token(scopes).await.context("issue getting access token")?;
        Ok(token)
    }

    fn drop(
        mut accessor: Access<'_, T, Self>, rep: Resource<IdentityProxy>,
    ) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl Host for WasiIdentityCtxView<'_> {}
impl HostIdentity for WasiIdentityCtxView<'_> {}
