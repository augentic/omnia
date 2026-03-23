//! `wasi:jsondb` `store` interface.

use wasmtime::component::Accessor;

use crate::host::generated::wasi::jsondb::store::{
    Document, Host as StoreHost, HostWithStore, QueryOptions, QueryResult,
};
use crate::host::{JsonDbError, QueryOpts, WasiJsonDb, WasiJsonDbCtxView};

fn map_err(e: &anyhow::Error) -> JsonDbError {
    JsonDbError::Other(e.to_string())
}

impl HostWithStore for WasiJsonDb {
    async fn get<T>(
        accessor: &Accessor<T, Self>, collection: String, id: String,
    ) -> Result<Option<Document>, JsonDbError> {
        let fut = accessor.with(|mut store| store.get().ctx.get(collection, id));
        fut.await.map_err(|e| map_err(&e))
    }

    async fn insert<T>(
        accessor: &Accessor<T, Self>, collection: String, doc: Document,
    ) -> Result<(), JsonDbError> {
        let fut = accessor.with(|mut store| store.get().ctx.insert(collection, doc));
        fut.await.map_err(|e| map_err(&e))
    }

    async fn put<T>(
        accessor: &Accessor<T, Self>, collection: String, doc: Document,
    ) -> Result<(), JsonDbError> {
        let fut = accessor.with(|mut store| store.get().ctx.put(collection, doc));
        fut.await.map_err(|e| map_err(&e))
    }

    async fn delete<T>(
        accessor: &Accessor<T, Self>, collection: String, id: String,
    ) -> Result<bool, JsonDbError> {
        let fut = accessor.with(|mut store| store.get().ctx.delete(collection, id));
        fut.await.map_err(|e| map_err(&e))
    }

    #[allow(clippy::needless_pass_by_value)] // Matches generated `HostWithStore::query` signature.
    async fn query<T>(
        accessor: &Accessor<T, Self>, collection: String, options: QueryOptions,
    ) -> Result<QueryResult, JsonDbError> {
        let QueryOptions {
            filter,
            order_by,
            limit,
            offset,
            continuation,
        } = options;

        let filter_tree = filter
            .map(|res| {
                accessor
                    .with(|mut store| store.get().table.delete(res))
                    .map(|fp| fp.0)
                    .map_err(|e| JsonDbError::Other(format!("resource table: {e}")))
            })
            .transpose()?;

        let opts = QueryOpts {
            order_by,
            limit,
            offset,
            continuation,
        };

        let fut = accessor.with(|mut store| store.get().ctx.query(collection, filter_tree, opts));
        fut.await.map_err(|e| map_err(&e))
    }
}

impl StoreHost for WasiJsonDbCtxView<'_> {}
