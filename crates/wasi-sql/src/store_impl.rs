use omnia_core::HostView;
use wasmtime::component::{Accessor, Resource};

use crate::database_impl::DatabaseHandle;
use crate::generated::omnia::sql::store::{Host, HostWithStore};
use crate::{Result, SqlClient, WasiSql};

impl<T> HostWithStore<T> for WasiSql {
    async fn open(accessor: &Accessor<T, Self>, name: String) -> Result<Resource<DatabaseHandle>> {
        let connection = accessor.with(|mut store| store.get().client.open(name.clone())).await?;
        DatabaseHandle::push(accessor, name, connection)
    }
}

impl Host for HostView<'_, dyn SqlClient> {}
