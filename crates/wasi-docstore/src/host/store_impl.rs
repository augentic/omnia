//! `omnia:docstore` `store` interface.

use wasmtime::component::Accessor;

use crate::host::generated::omnia::docstore::store::{
    Document as WitDocument, Host as StoreHost, HostWithStore, QueryOptions as WitQueryOptions,
    QueryResult as WitQueryResult,
};
use crate::host::generated::omnia::docstore::types::SortField as WitSortField;
use crate::host::{Document, QueryOptions, Result, SortField, WasiDocStore, WasiDocStoreView};

impl<T> HostWithStore<T> for WasiDocStore {
    async fn get(
        accessor: &Accessor<T, Self>, collection: String, id: String,
    ) -> Result<Option<WitDocument>> {
        let fut = accessor.with(|mut store| store.get().client.get(collection, id));
        Ok(fut.await?.map(to_wit_document))
    }

    async fn insert(
        accessor: &Accessor<T, Self>, collection: String, doc: WitDocument,
    ) -> Result<()> {
        let fut = accessor
            .with(|mut store| store.get().client.insert(collection, from_wit_document(doc)));
        Ok(fut.await?)
    }

    async fn put(accessor: &Accessor<T, Self>, collection: String, doc: WitDocument) -> Result<()> {
        let fut =
            accessor.with(|mut store| store.get().client.put(collection, from_wit_document(doc)));
        Ok(fut.await?)
    }

    async fn delete(accessor: &Accessor<T, Self>, collection: String, id: String) -> Result<bool> {
        let fut = accessor.with(|mut store| store.get().client.delete(collection, id));
        Ok(fut.await?)
    }

    async fn query(
        accessor: &Accessor<T, Self>, collection: String, options: WitQueryOptions,
    ) -> Result<WitQueryResult> {
        let WitQueryOptions {
            filter,
            order_by,
            limit,
            offset,
            continuation,
        } = options;

        let filter = filter
            .map(|res| accessor.with(|mut store| store.get().table.delete(res)))
            .transpose()?
            .map(|fp| fp.0);

        let opts = QueryOptions {
            filter,
            order_by: order_by
                .into_iter()
                .map(|WitSortField { field, descending }| SortField { field, descending })
                .collect(),
            limit,
            offset,
            continuation,
        };

        let fut = accessor.with(|mut store| store.get().client.query(collection, opts));
        let result = fut.await?;
        Ok(WitQueryResult {
            documents: result.documents.into_iter().map(to_wit_document).collect(),
            continuation: result.continuation,
        })
    }
}

impl StoreHost for WasiDocStoreView<'_> {}

fn from_wit_document(doc: WitDocument) -> Document {
    Document {
        id: doc.id,
        data: doc.data,
    }
}

fn to_wit_document(doc: Document) -> WitDocument {
    WitDocument {
        id: doc.id,
        data: doc.data,
    }
}
