use std::ops::Range;
use std::sync::Arc;

use bytes::Bytes;
use omnia_core::HostView;
use wasmtime::component::{Access, Accessor, FutureReader, Resource, StreamReader};

use crate::generated::omnia::blobstore::container::{
    self as wit, ByteRange, Content, Host, HostContainer, HostContainerWithStore, Listing,
};
use crate::generated::omnia::blobstore::types::{
    self, Attributes, Conflict, ObjectEntry, Precondition, Stored,
};
use crate::transfer::StagedPut;
use crate::{BlobStoreClient, Container, Error, ObjectMetadata, Result, WasiBlobStore};

const PAGE_LIMIT: u32 = 1_000;

// Sorts after any character a name holds past a folded prefix, so a cursor
// ending in it resumes after every name under that prefix.
const PAST_PREFIX: char = '\u{10FFFF}';

// The host's side of a guest's `container` handle. A backend's `Container`
// reports no name, so the handle keeps the one the guest opened.
#[derive(Clone, Debug)]
pub struct ContainerHandle {
    name: String,
    inner: Arc<dyn Container>,
}

impl ContainerHandle {
    pub fn push<T>(
        accessor: &Accessor<T, WasiBlobStore>, name: String, inner: Arc<dyn Container>,
    ) -> Result<Resource<Self>> {
        let handle = Self { name, inner };
        Ok(accessor.with(|mut store| store.get().table.push(handle))?)
    }

    fn lookup<T>(accessor: &Accessor<T, WasiBlobStore>, resource: &Resource<Self>) -> Result<Self> {
        Ok(omnia_core::get_cloned(accessor, resource)?)
    }

    fn info(&self) -> Result<wit::ContainerMetadata> {
        let info = self.inner.info()?;
        Ok(wit::ContainerMetadata {
            name: self.name.clone(),
            created_at: (info.created_at > 0).then(|| millis(info.created_at)),
        })
    }

    // A backend reports nothing on write, so the object is read back for its etag.
    pub async fn write(&self, name: String, data: Bytes) -> Result<Stored> {
        self.inner.write_data(name.clone(), data).await?;
        let object = self.inner.object_info(name.clone()).await?;
        object
            .map(|object| Stored::from(&object))
            .ok_or_else(|| Error::Other(format!("`{name}` was deleted as it was written")))
    }

    async fn duplicate(&self, name: String, dest: &Self, dest_name: String) -> Result<Stored> {
        let Some(data) = self.inner.get_data(name.clone(), 0..u64::MAX).await? else {
            return Err(self.missing(&name));
        };
        dest.write(dest_name, data).await
    }

    fn missing(&self, name: &str) -> Error {
        Error::NotFound(format!("source object not found: {}/{name}", self.name))
    }

    // A backend lists every name under a prefix, so the host folds and limits.
    async fn list(
        &self, prefix: String, listing: Listing, after: Option<String>, limit: Option<u32>,
    ) -> Result<wit::Page> {
        let limit = match limit {
            Some(0) => return Err(Error::Invalid("a listing limit must be at least 1".to_owned())),
            Some(limit) => limit.min(PAGE_LIMIT) as usize,
            None => PAGE_LIMIT as usize,
        };
        let delimited = matches!(listing, Listing::Delimited);
        let mut objects = Vec::new();
        let mut common_prefixes: Vec<String> = Vec::new();
        let mut next = None;
        let mut cursor = after;

        loop {
            let listed = self.inner.list_objects(prefix.clone(), cursor).await?;
            for object in listed.objects {
                let folded = if delimited { common_prefix(&prefix, &object.name) } else { None };
                if folded.is_some() && folded == common_prefixes.last().map(String::as_str) {
                    continue;
                }
                if objects.len() + common_prefixes.len() == limit {
                    return Ok(wit::Page {
                        objects,
                        common_prefixes,
                        next,
                    });
                }
                if let Some(folded) = folded {
                    next = Some(format!("{folded}{PAST_PREFIX}"));
                    common_prefixes.push(folded.to_owned());
                } else {
                    next = Some(object.name.clone());
                    objects.push(object.into());
                }
            }
            match listed.next {
                Some(more) => cursor = Some(more),
                None => {
                    return Ok(wit::Page {
                        objects,
                        common_prefixes,
                        next: None,
                    });
                }
            }
        }
    }
}

// the folded prefix of `name`, through its first `/` past `prefix`
fn common_prefix<'a>(prefix: &str, name: &'a str) -> Option<&'a str> {
    let at = name.strip_prefix(prefix)?.find('/')?;
    Some(&name[..=prefix.len() + at])
}

impl<T> HostContainerWithStore<T> for WasiBlobStore {
    fn name(
        mut host: Access<'_, T, Self>, self_: Resource<ContainerHandle>,
    ) -> wasmtime::Result<String> {
        Ok(host.get().table.get(&self_)?.name.clone())
    }

    fn info(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerHandle>,
    ) -> impl Future<Output = Result<wit::ContainerMetadata>> + Send {
        std::future::ready(
            ContainerHandle::lookup(accessor, &self_).and_then(|handle| handle.info()),
        )
    }

    async fn get(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerHandle>, name: String,
        range: Option<ByteRange>,
    ) -> Result<Option<Content>> {
        let container = ContainerHandle::lookup(accessor, &self_)?.inner;
        let Some(object) = container.object_info(name.clone()).await? else {
            return Ok(None);
        };
        let Some(bytes) = container.get_data(name, bounds(range, object.size)).await? else {
            return Ok(None);
        };
        let metadata = types::ObjectMetadata::from(object);

        // the bytes are all in hand, so `finished` resolves at once
        accessor.with(|mut store| {
            let resolved = std::future::ready(Ok::<_, wasmtime::Error>(Ok(())));
            let mut finished = FutureReader::new(&mut store, resolved)?;
            let data = match StreamReader::new(&mut store, bytes) {
                Ok(data) => data,
                Err(error) => {
                    finished.close(&mut store)?;
                    return Err(error.into());
                }
            };
            Ok(Some(Content {
                metadata,
                data,
                finished,
            }))
        })
    }

    fn put(
        mut host: Access<'_, T, Self>, self_: Resource<ContainerHandle>, name: String,
        size: Option<u64>, mut data: StreamReader<u8>, mut finished: FutureReader<Result<()>>,
        attributes: Attributes, when: Option<Precondition>,
    ) -> wasmtime::Result<FutureReader<Result<std::result::Result<Stored, Conflict>>>> {
        let container = host.get().table.get(&self_)?.clone();

        // refuse what no backend stores
        let plain = no_attributes(&attributes).and_then(|()| unconditional(when.as_ref()));
        if let Err(refusal) = plain {
            data.close(&mut host)?;
            finished.close(&mut host)?;
            let reply = std::future::ready(Ok::<_, wasmtime::Error>(Err(refusal)));
            return FutureReader::new(&mut host, reply);
        }

        StagedPut::new(container, name, size).pipe(&mut host, data, finished)
    }

    async fn stat(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerHandle>, name: String,
    ) -> Result<Option<types::ObjectMetadata>> {
        let container = ContainerHandle::lookup(accessor, &self_)?.inner;
        Ok(container.object_info(name).await?.map(Into::into))
    }

    async fn exists(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerHandle>, name: String,
    ) -> Result<bool> {
        let container = ContainerHandle::lookup(accessor, &self_)?.inner;
        Ok(container.has_object(name).await?)
    }

    async fn delete(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerHandle>, name: String,
        when: Option<String>,
    ) -> Result<std::result::Result<(), Conflict>> {
        unconditional(when.as_ref())?;
        let container = ContainerHandle::lookup(accessor, &self_)?.inner;
        container.delete_object(name).await?;
        Ok(Ok(()))
    }

    async fn delete_many(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerHandle>, names: Vec<String>,
    ) -> Result<()> {
        let container = ContainerHandle::lookup(accessor, &self_)?.inner;
        for name in names {
            container.delete_object(name).await?;
        }
        Ok(())
    }

    async fn clear(accessor: &Accessor<T, Self>, self_: Resource<ContainerHandle>) -> Result<()> {
        let container = ContainerHandle::lookup(accessor, &self_)?.inner;
        for name in container.names().await? {
            container.delete_object(name).await?;
        }
        Ok(())
    }

    async fn list(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerHandle>, prefix: String,
        listing: Listing, after: Option<String>, limit: Option<u32>,
    ) -> Result<wit::Page> {
        ContainerHandle::lookup(accessor, &self_)?.list(prefix, listing, after, limit).await
    }

    async fn copy(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerHandle>, name: String,
        dest: Resource<ContainerHandle>, dest_name: String, when: Option<Precondition>,
    ) -> Result<std::result::Result<Stored, Conflict>> {
        unconditional(when.as_ref())?;
        let source = ContainerHandle::lookup(accessor, &self_)?;
        let dest = ContainerHandle::lookup(accessor, &dest)?;
        Ok(Ok(source.duplicate(name, &dest, dest_name).await?))
    }

    async fn rename(
        accessor: &Accessor<T, Self>, self_: Resource<ContainerHandle>, name: String,
        dest: Resource<ContainerHandle>, dest_name: String, when: Option<Precondition>,
    ) -> Result<std::result::Result<Stored, Conflict>> {
        unconditional(when.as_ref())?;
        let source = ContainerHandle::lookup(accessor, &self_)?;
        let dest = ContainerHandle::lookup(accessor, &dest)?;

        // deleting the source would destroy the destination
        if source.name == dest.name && name == dest_name {
            let Some(object) = source.inner.object_info(name.clone()).await? else {
                return Err(source.missing(&name));
            };
            return Ok(Ok(Stored::from(&object)));
        }

        let stored = source.duplicate(name.clone(), &dest, dest_name).await?;
        source.inner.delete_object(name).await?;
        Ok(Ok(stored))
    }

    fn drop(mut host: Access<'_, T, Self>, rep: Resource<ContainerHandle>) -> wasmtime::Result<()> {
        host.get().table.delete(rep)?;
        Ok(())
    }
}

// the half-open range `range` selects in an object of `size` bytes
const fn bounds(range: Option<ByteRange>, size: u64) -> Range<u64> {
    match range {
        None => 0..u64::MAX,
        Some(ByteRange::Span((start, end))) => start..end,
        Some(ByteRange::Offset(start)) => start..u64::MAX,
        Some(ByteRange::Suffix(len)) => size.saturating_sub(len)..u64::MAX,
    }
}

fn no_attributes(attributes: &Attributes) -> Result<()> {
    let Attributes {
        content_type,
        content_encoding,
        content_language,
        content_disposition,
        cache_control,
        metadata,
    } = attributes;
    let headers =
        [content_type, content_encoding, content_language, content_disposition, cache_control];
    if headers.iter().all(|header| header.is_none()) && metadata.is_empty() {
        return Ok(());
    }
    Err(Error::Unsupported("this backend stores no object attributes".to_owned()))
}

fn unconditional<W>(when: Option<&W>) -> Result<()> {
    match when {
        None => Ok(()),
        Some(_) => Err(Error::Unsupported("this backend checks no write precondition".to_owned())),
    }
}

// A weak stand-in until a backend reports one: two writes of one size within a
// second share it.
fn etag(object: &ObjectMetadata) -> String {
    format!("{:x}-{:x}", object.created_at, object.size)
}

// a `Container` reports seconds, and the WIT carries milliseconds
const fn millis(seconds: u64) -> u64 {
    seconds.saturating_mul(1000)
}

impl From<ObjectMetadata> for types::ObjectMetadata {
    fn from(object: ObjectMetadata) -> Self {
        Self {
            etag: etag(&object),
            name: object.name,
            size: object.size,
            modified_at: millis(object.created_at),
            version: None,
            attributes: Attributes {
                content_type: None,
                content_encoding: None,
                content_language: None,
                content_disposition: None,
                cache_control: None,
                metadata: Vec::new(),
            },
        }
    }
}

impl From<ObjectMetadata> for ObjectEntry {
    fn from(object: ObjectMetadata) -> Self {
        Self {
            etag: etag(&object),
            name: object.name,
            size: object.size,
            modified_at: millis(object.created_at),
        }
    }
}

impl From<&ObjectMetadata> for Stored {
    fn from(object: &ObjectMetadata) -> Self {
        Self {
            etag: etag(object),
            version: None,
        }
    }
}

impl Host for HostView<'_, dyn BlobStoreClient> {}

impl HostContainer for HostView<'_, dyn BlobStoreClient> {}
