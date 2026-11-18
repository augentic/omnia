mod filter;

use std::collections::HashSet;

use omnia_core::{Digest, HostView};
use serde_json::Value;
use wasmtime::component::{Access, Accessor, Resource};

use crate::host::generated::omnia::docstore::collection::{
    self, Host, HostCollection, HostCollectionWithStore, Page,
};
use crate::host::generated::omnia::docstore::types::{self, Conflict, Precondition, Revision};
use crate::host::{
    DocStoreClient, Document, Error, FutureResult, QueryOpts, Result, SortField, WasiDocStore,
};

// The most documents a page holds, whatever `limit` asks for.
const MAX_PAGE_SIZE: u32 = 1000;

// The host's side of a guest's `collection` handle. It keeps the name the
// guest opened, which every call hands the backend.
#[derive(Clone, Debug)]
pub struct CollectionHandle {
    name: String,
}

impl CollectionHandle {
    pub fn push<T>(accessor: &Accessor<T, WasiDocStore>, name: String) -> Result<Resource<Self>> {
        let handle = Self { name };
        Ok(accessor.with(|mut store| store.get().table.push(handle))?)
    }

    fn lookup<T>(accessor: &Accessor<T, WasiDocStore>, resource: &Resource<Self>) -> Result<Self> {
        Ok(omnia_core::get_cloned(accessor, resource)?)
    }

    // starts `call` on the deployment's backend, naming this collection
    fn call<T, R>(
        &self, accessor: &Accessor<T, WasiDocStore>,
        call: impl FnOnce(&dyn DocStoreClient, String) -> FutureResult<R>,
    ) -> FutureResult<R> {
        accessor.with(|mut store| call(store.get().client, self.name.clone()))
    }

    // a call the `DocStoreClient` trait cannot express
    fn unsupported(&self, call: &str) -> Error {
        Error::Unsupported(format!("collection `{}` cannot {call}", self.name))
    }
}

impl<T> HostCollectionWithStore<T> for WasiDocStore {
    async fn get(
        accessor: &Accessor<T, Self>, self_: Resource<CollectionHandle>, id: String,
    ) -> Result<Option<types::Document>> {
        let collection = CollectionHandle::lookup(accessor, &self_)?;
        let found = collection.call(accessor, |client, name| client.get(name, id)).await?;
        Ok(found.map(types::Document::from))
    }

    async fn get_many(
        accessor: &Accessor<T, Self>, self_: Resource<CollectionHandle>, ids: Vec<String>,
    ) -> Result<Vec<Option<types::Document>>> {
        let collection = CollectionHandle::lookup(accessor, &self_)?;
        let mut documents = Vec::with_capacity(ids.len());
        for id in ids {
            let found = collection.call(accessor, |client, name| client.get(name, id)).await?;
            documents.push(found.map(types::Document::from));
        }
        Ok(documents)
    }

    async fn exists(
        accessor: &Accessor<T, Self>, self_: Resource<CollectionHandle>, id: String,
    ) -> Result<bool> {
        let collection = CollectionHandle::lookup(accessor, &self_)?;
        let found = collection.call(accessor, |client, name| client.get(name, id)).await?;
        Ok(found.is_some())
    }

    async fn put(
        accessor: &Accessor<T, Self>, self_: Resource<CollectionHandle>, id: String, data: Vec<u8>,
        when: Option<Precondition>,
    ) -> Result<std::result::Result<Revision, Conflict>> {
        let collection = CollectionHandle::lookup(accessor, &self_)?;
        let written = revision(&object(&id, &data)?);
        let document = Document { id, data };

        match when {
            None => {
                collection.call(accessor, |client, name| client.put(name, document)).await?;
                Ok(Ok(written))
            }
            Some(Precondition::Absent) => {
                let id = document.id.clone();
                if collection.call(accessor, |client, name| client.insert(name, document)).await? {
                    return Ok(Ok(written));
                }
                let current =
                    collection.call(accessor, |client, name| client.get(name, id)).await?;
                Ok(Err(Conflict {
                    current: current.map(types::Document::from),
                }))
            }
            Some(Precondition::Matches(_)) => {
                Err(collection.unsupported("check a revision before a write"))
            }
        }
    }

    async fn put_many(
        accessor: &Accessor<T, Self>, self_: Resource<CollectionHandle>,
        documents: Vec<(String, Vec<u8>)>,
    ) -> Result<()> {
        if let Some(id) = repeated(&documents) {
            return Err(Error::Invalid(format!("`{id}` is given more than once")));
        }
        for (id, data) in &documents {
            object(id, data)?;
        }

        let collection = CollectionHandle::lookup(accessor, &self_)?;
        for (id, data) in documents {
            let document = Document { id, data };
            collection.call(accessor, |client, name| client.put(name, document)).await?;
        }
        Ok(())
    }

    async fn delete(
        accessor: &Accessor<T, Self>, self_: Resource<CollectionHandle>, id: String,
        when: Option<Revision>,
    ) -> Result<std::result::Result<(), Conflict>> {
        let collection = CollectionHandle::lookup(accessor, &self_)?;
        if when.is_some() {
            return Err(collection.unsupported("check a revision before a delete"));
        }
        collection.call(accessor, |client, name| client.delete(name, id)).await?;
        Ok(Ok(()))
    }

    async fn delete_many(
        accessor: &Accessor<T, Self>, self_: Resource<CollectionHandle>, ids: Vec<String>,
    ) -> Result<()> {
        let collection = CollectionHandle::lookup(accessor, &self_)?;
        for id in ids {
            collection.call(accessor, |client, name| client.delete(name, id)).await?;
        }
        Ok(())
    }

    async fn query(
        accessor: &Accessor<T, Self>, self_: Resource<CollectionHandle>,
        filter: collection::Filter, order_by: Vec<collection::SortField>, after: Option<String>,
        limit: Option<u32>,
    ) -> Result<Page> {
        let collection = CollectionHandle::lookup(accessor, &self_)?;
        let limit = match limit {
            Some(0) => return Err(Error::Invalid("a query `limit` must be at least 1".to_owned())),
            limit => limit.map_or(MAX_PAGE_SIZE, |limit| limit.min(MAX_PAGE_SIZE)),
        };
        let tree = filter::tree(&collection, filter)?;
        let options = QueryOpts {
            order_by: order_by
                .into_iter()
                .map(|sort| SortField {
                    field: sort.field,
                    descending: sort.descending,
                })
                .collect(),
            limit: Some(limit),
            offset: None,
            continuation: after,
        };

        let result =
            collection.call(accessor, |client, name| client.query(name, tree, options)).await?;
        Ok(Page {
            documents: result.documents.into_iter().map(types::Document::from).collect(),
            next: result.continuation,
        })
    }

    fn drop(
        mut host: Access<'_, T, Self>, rep: Resource<CollectionHandle>,
    ) -> wasmtime::Result<()> {
        host.get().table.delete(rep)?;
        Ok(())
    }
}

// a body the guest writes is a JSON object, refused before the backend sees it
fn object(id: &str, data: &[u8]) -> Result<Value> {
    match serde_json::from_slice(data) {
        Ok(body @ Value::Object(_)) => Ok(body),
        Ok(_) => Err(Error::Invalid(format!("the body of `{id}` is not a JSON object"))),
        Err(err) => Err(Error::Invalid(format!("the body of `{id}` is not JSON: {err}"))),
    }
}

// The digest of a body as `serde_json` writes it stands in for a revision
// until backends carry one. Writes and reads digest that one form, so a
// write's revision is the one a read of the same JSON value reports.
fn revision(body: &Value) -> Revision {
    Digest::of(body.to_string().as_bytes()).to_string()
}

impl From<Document> for types::Document {
    fn from(document: Document) -> Self {
        let revision = serde_json::from_slice(&document.data)
            .map_or_else(|_| Digest::of(&document.data).to_string(), |body| revision(&body));
        Self {
            id: document.id,
            data: document.data,
            revision,
        }
    }
}

fn repeated(documents: &[(String, Vec<u8>)]) -> Option<&str> {
    let mut seen = HashSet::new();
    documents.iter().map(|(id, _)| id.as_str()).find(|id| !seen.insert(*id))
}

impl Host for HostView<'_, dyn DocStoreClient> {}

impl HostCollection for HostView<'_, dyn DocStoreClient> {}
