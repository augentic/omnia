use anyhow::Context;
use wasmtime::component::{Access, Accessor, Resource, ResourceTableError};

use crate::host::generated::wasi::identity::credentials::{
    AccessToken, Host, HostIdentity, HostIdentityWithStore, HostWithStore,
};
use crate::host::generated::wasi::identity::types::Error;
use crate::host::resource::IdentityProxy;
use crate::host::{Result, WasiIdentity, WasiIdentityCtxView};

impl HostWithStore for WasiIdentity {
    async fn get_identity<T>(
        accessor: &Accessor<T, Self>, name: String,
    ) -> Result<Resource<IdentityProxy>> {
        let identity = accessor.with(|mut store| store.get().ctx.get_identity(name)).await?;
        let proxy = IdentityProxy(identity);
        Ok(accessor.with(|mut store| store.get().table.push(proxy))?)
    }
}

impl HostIdentityWithStore for WasiIdentity {
    async fn get_token<T>(
        accessor: &Accessor<T, Self>, self_: Resource<IdentityProxy>, scopes: Vec<String>,
    ) -> Result<AccessToken> {
        let identity = get_identity(accessor, &self_)?;
        let token = identity.0.get_token(scopes).await.context("issue getting access token")?;
        Ok(token)
    }

    fn drop<T>(
        mut accessor: Access<'_, T, Self>, rep: Resource<IdentityProxy>,
    ) -> wasmtime::Result<()> {
        Ok(accessor.get().table.delete(rep).map(|_| ())?)
    }
}

impl Host for WasiIdentityCtxView<'_> {}
impl HostIdentity for WasiIdentityCtxView<'_> {}

pub fn get_identity<T>(
    accessor: &Accessor<T, WasiIdentity>, self_: &Resource<IdentityProxy>,
) -> Result<IdentityProxy> {
    accessor
        .with(|mut store| store.get().table.get(self_).cloned().map_err(|_e| Error::NoSuchIdentity))
}

impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        Self::InternalFailure(err.to_string())
    }
}

impl From<ResourceTableError> for Error {
    fn from(err: ResourceTableError) -> Self {
        Self::InternalFailure(err.to_string())
    }
}
