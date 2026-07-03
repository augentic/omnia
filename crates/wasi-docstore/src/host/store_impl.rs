//! `wasi:docstore` `store` interface.

use wasmtime::component::Accessor;

use crate::host::generated::wasi::docstore::store::{
    Document, Host as StoreHost, HostWithStore, QueryOptions, QueryResult,
};
use crate::host::{DocStoreError, QueryOpts, WasiDocStore, WasiDocStoreCtxView};

fn map_err(e: &anyhow::Error) -> DocStoreError {
    DocStoreError::Other(format!("{e:#}"))
}

impl<T> HostWithStore<T> for WasiDocStore {
    async fn get(
        accessor: &Accessor<T, Self>, collection: String, id: String,
    ) -> Result<Option<Document>, DocStoreError> {
        let fut = accessor.with(|mut store| store.get().ctx.get(collection, id));
        fut.await.map_err(|e| map_err(&e))
    }

    async fn insert(
        accessor: &Accessor<T, Self>, collection: String, doc: Document,
    ) -> Result<(), DocStoreError> {
        let fut = accessor.with(|mut store| store.get().ctx.insert(collection, doc));
        fut.await.map_err(|e| map_err(&e))
    }

    async fn put(
        accessor: &Accessor<T, Self>, collection: String, doc: Document,
    ) -> Result<(), DocStoreError> {
        let fut = accessor.with(|mut store| store.get().ctx.put(collection, doc));
        fut.await.map_err(|e| map_err(&e))
    }

    async fn delete(
        accessor: &Accessor<T, Self>, collection: String, id: String,
    ) -> Result<bool, DocStoreError> {
        let fut = accessor.with(|mut store| store.get().ctx.delete(collection, id));
        fut.await.map_err(|e| map_err(&e))
    }

    #[allow(clippy::needless_pass_by_value)] // Matches generated `HostWithStore::query` signature.
    async fn query(
        accessor: &Accessor<T, Self>, collection: String, options: QueryOptions,
    ) -> Result<QueryResult, DocStoreError> {
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
                    .map_err(|e| DocStoreError::Other(format!("resource table: {e}")))
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

impl StoreHost for WasiDocStoreCtxView<'_> {}
