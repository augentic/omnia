//! `omnia:blobstore`, the containers of binary objects a deployment declares.
//!
//! A guest opens a [`Container`] with [`open`]. [`exists`] tells whether a
//! container exists, and [`create`] makes one. Beside the generated methods,
//! a container has unconditional forms that take borrowed names and read or
//! write an object whole:
//!
//! - [`read`](Container::read) and [`read_range`](Container::read_range)
//!   return an object's bytes.
//! - [`write`](Container::write) stores bytes under default
//!   [`Attributes`], and [`write_with`](Container::write_with) under the
//!   caller's.
//! - [`metadata`](Container::metadata) describes an object,
//!   [`contains`](Container::contains) tells whether one exists, and
//!   [`remove`](Container::remove) deletes one.
//!
//! [`Content::read`] collects the bytes of a raw `get`.
//!
//! An absent object is `Ok(None)`. Every backend accepts portable names, and
//! a backend may reject any other with the code `invalid`. A portable
//! container name:
//!
//! - is 3 to 63 characters long;
//! - holds only lowercase ASCII letters, digits, and `-`;
//! - starts and ends with a letter or digit, and never holds `--`.
//!
//! A portable object name is one or more segments joined by `/`. Each segment
//! is not empty, holds only ASCII letters, digits, `.`, `_`, and `-`, and is
//! neither `.` nor `..`. A portable container never holds both an object `a`
//! and an object under `a/`.
//!
//! The unstable [`uploads`] and [`presign`] interfaces are the raw bindings.
//! Every host links them, and answers `unsupported` today.

use std::ops::Range;

use bytes::Bytes;

pub use crate::bindings::imports::omnia::blobstore::{
    admin, container, presign, store, types, uploads,
};
use crate::blobstore::container::ByteRange;
pub use crate::blobstore::container::{Container, Content};
pub use crate::blobstore::types::{Attributes, ObjectMetadata};
use crate::error::unconditional;
use crate::{Result, wit_future, wit_stream};

/// Open the container `name`, a binding the deployment declares.
///
/// # Errors
///
/// The host's failure, under its code: `not-found` for a container the
/// deployment does not declare.
pub async fn open(name: &str) -> Result<Container> {
    Ok(store::open(name.to_owned()).await?)
}

/// Whether the container `name` exists.
///
/// # Errors
///
/// The host's failure, under its code.
pub async fn exists(name: &str) -> Result<bool> {
    Ok(store::container_exists(name.to_owned()).await?)
}

/// Create the container `name` and open it, keeping the objects of one that
/// already exists.
///
/// # Errors
///
/// The host's failure, under its code.
pub async fn create(name: &str) -> Result<Container> {
    Ok(admin::create_container(name.to_owned()).await?)
}

impl Container {
    /// The bytes of the object `name`, or `None` when it is absent.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn read(&self, name: &str) -> Result<Option<Bytes>> {
        let content = self.get(name.to_owned(), None).await?;
        Ok(match content {
            Some(content) => Some(content.read().await?),
            None => None,
        })
    }

    /// The bytes `range` spans in the object `name`, or `None` when it is
    /// absent.
    ///
    /// The range is half-open and clamped to the object's length, so a range
    /// past the end, or one that ends before it starts, reads as empty.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn read_range(&self, name: &str, range: Range<u64>) -> Result<Option<Bytes>> {
        let range = ByteRange::Span((range.start, range.end));
        let content = self.get(name.to_owned(), Some(range)).await?;
        Ok(match content {
            Some(content) => Some(content.read().await?),
            None => None,
        })
    }

    /// Store `data` as the object `name`, replacing any object there.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn write(&self, name: &str, data: &[u8]) -> Result<()> {
        self.write_with(name, data, &Attributes::default()).await
    }

    /// Store `data` as the object `name` under `attributes`, replacing any
    /// object there.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code: `unsupported` for an attribute the
    /// backend cannot store.
    pub async fn write_with(&self, name: &str, data: &[u8], attributes: &Attributes) -> Result<()> {
        let (mut writer, reader) = wit_stream::new();
        let (finish, finished) =
            wit_future::new(|| Err(types::Error::Other("the write was abandoned".to_owned())));
        let size = Some(data.len() as u64);
        let reply = self.put(name, size, reader, finished, attributes, None);

        // a short write or a refused `finished` means the host refused the
        // write, and its reply says why
        writer.write_all(data.to_vec()).await;
        drop(writer);
        let _ = finish.write(Ok(())).await;
        unconditional(reply.await?)?;
        Ok(())
    }

    /// Everything known of the object `name`, or `None` when it is absent.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn metadata(&self, name: &str) -> Result<Option<ObjectMetadata>> {
        Ok(self.stat(name.to_owned()).await?)
    }

    /// Whether the object `name` exists.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn contains(&self, name: &str) -> Result<bool> {
        Ok(self.exists(name.to_owned()).await?)
    }

    /// Delete the object `name`. Deleting an absent object succeeds.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn remove(&self, name: &str) -> Result<()> {
        unconditional(self.delete(name.to_owned(), None).await?)
    }
}

impl Content {
    /// Collect the object's bytes, once the host vouches they are whole.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code, when the read fails part way.
    pub async fn read(self) -> Result<Bytes> {
        let bytes = self.data.collect().await;
        self.finished.await?;
        Ok(Bytes::from(bytes))
    }
}

#[expect(clippy::derivable_impls, reason = "the record is generated, so it cannot derive")]
impl Default for Attributes {
    fn default() -> Self {
        Self {
            content_type: None,
            content_encoding: None,
            content_language: None,
            content_disposition: None,
            cache_control: None,
            metadata: Vec::new(),
        }
    }
}
