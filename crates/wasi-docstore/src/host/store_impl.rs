//! `wasi:docstore` `store` interface.

use wasmtime::component::Accessor;

use crate::host::generated::wasi::docstore::store::{
    Document, Host as StoreHost, HostWithStore, QueryOptions, QueryResult,
};
use crate::host::{QueryOpts, Result, WasiDocStore, WasiDocStoreCtxView};

impl<T> HostWithStore<T> for WasiDocStore {
    async fn get(
        accessor: &Accessor<T, Self>, collection: String, id: String,
    ) -> Result<Option<Document>> {
        let fut = accessor.with(|mut store| store.get().ctx.get(collection, id));
        Ok(fut.await?)
    }

    async fn insert(accessor: &Accessor<T, Self>, collection: String, doc: Document) -> Result<()> {
        let fut = accessor.with(|mut store| store.get().ctx.insert(collection, doc));
        Ok(fut.await?)
    }

    async fn put(accessor: &Accessor<T, Self>, collection: String, doc: Document) -> Result<()> {
        let fut = accessor.with(|mut store| store.get().ctx.put(collection, doc));
        Ok(fut.await?)
    }

    async fn delete(accessor: &Accessor<T, Self>, collection: String, id: String) -> Result<bool> {
        let fut = accessor.with(|mut store| store.get().ctx.delete(collection, id));
        Ok(fut.await?)
    }

    #[allow(clippy::needless_pass_by_value)] // Matches generated `HostWithStore::query` signature.
    async fn query(
        accessor: &Accessor<T, Self>, collection: String, options: QueryOptions,
    ) -> Result<QueryResult> {
        let QueryOptions {
            filter,
            order_by,
            limit,
            offset,
            continuation,
        } = options;

        let filter_tree = filter
            .map(|res| accessor.with(|mut store| store.get().table.delete(res)))
            .transpose()?
            .map(|fp| fp.0);

        let opts = QueryOpts {
            order_by,
            limit,
            offset,
            continuation,
        };

        let fut = accessor.with(|mut store| store.get().ctx.query(collection, filter_tree, opts));
        Ok(fut.await?)
    }
}

impl StoreHost for WasiDocStoreCtxView<'_> {}
