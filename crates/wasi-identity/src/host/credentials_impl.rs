use anyhow::Context;
use omnia_core::HostView;
use wasmtime::component::{Access, Accessor, Resource};

use crate::host::generated::omnia::identity::credentials::{
    AccessToken as WitAccessToken, Host, HostIdentity, HostIdentityWithStore, HostWithStore,
};
use crate::host::generated::omnia::identity::types::Error;
use crate::host::resource::IdentityProxy;
use crate::host::{IdentityClient, Result, WasiIdentity};

impl<T> HostWithStore<T> for WasiIdentity {
    async fn get_identity(
        accessor: &Accessor<T, Self>, name: String,
    ) -> Result<Resource<IdentityProxy>> {
        let identity = accessor.with(|mut store| store.get().client.get_identity(name)).await?;
        let proxy = omnia_core::Proxy(identity);
        Ok(accessor.with(|mut store| store.get().table.push(proxy))?)
    }
}

impl<T> HostIdentityWithStore<T> for WasiIdentity {
    async fn get_token(
        accessor: &Accessor<T, Self>, self_: Resource<IdentityProxy>, scopes: Vec<String>,
    ) -> Result<WitAccessToken> {
        let identity =
            omnia_core::get_cloned(accessor, &self_).map_err(|_stale| Error::NoSuchIdentity)?;

        let token = identity.0.get_token(scopes).await.context("issue getting access token")?;
        Ok(WitAccessToken {
            token: token.token,
            expires_in: token.expires_in,
        })
    }

    fn drop(
        mut accessor: Access<'_, T, Self>, rep: Resource<IdentityProxy>,
    ) -> wasmtime::Result<()> {
        accessor.get().table.delete(rep)?;
        Ok(())
    }
}

impl Host for HostView<'_, dyn IdentityClient> {}
impl HostIdentity for HostView<'_, dyn IdentityClient> {}
