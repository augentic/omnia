use std::fmt::Debug;
use std::ops::Range;

use bytes::Bytes;
pub use omnia_core::FutureResult;

/// Providers implement the [`Container`] trait to allow the host to
/// interact with different backend containers.
pub trait Container: Debug + Send + Sync + 'static {
    /// Returns the metadata for the container.
    ///
    /// # Errors
    ///
    /// Returns an error if the container metadata cannot be retrieved.
    fn info(&self) -> anyhow::Result<ContainerMetadata>;

    /// The bytes of the object `name` in `range`, or `None` when the object is absent.
    ///
    /// The range is half-open and clamped to the object's length, so a range
    /// past the end, or one that ends before it starts, reads as empty.
    fn get_data(&self, name: String, range: Range<u64>) -> FutureResult<Option<Bytes>>;

    /// Set the value associated with the key.
    fn write_data(&self, name: String, data: Bytes) -> FutureResult<()>;

    /// One page of the objects whose names start with `prefix`, in name order.
    ///
    /// `after` is the [`next`](Page::next) cursor of the previous page, or
    /// `None` for the first page. A backend may answer every object in one
    /// page.
    fn list_objects(&self, prefix: String, after: Option<String>) -> FutureResult<Page>;

    /// Delete the value associated with the key, succeeding when the object is already absent.
    fn delete_object(&self, name: String) -> FutureResult<()>;

    /// Check if the object exists.
    fn has_object(&self, name: String) -> FutureResult<bool>;

    /// The metadata of the object `name`, or `None` when the object is absent.
    fn object_info(&self, name: String) -> FutureResult<Option<ObjectMetadata>>;
}

impl dyn Container {
    // every object's name, across every page of the listing
    pub(crate) async fn names(&self) -> anyhow::Result<Vec<String>> {
        let mut names = Vec::new();
        let mut after = None;
        loop {
            let page = self.list_objects(String::new(), after).await?;
            names.extend(page.objects.into_iter().map(|object| object.name));
            match page.next {
                Some(next) => after = Some(next),
                None => return Ok(names),
            }
        }
    }
}

/// A container's metadata, as [`Container::info`] reports it.
#[derive(Clone, Debug)]
pub struct ContainerMetadata {
    /// When the container was created, in seconds since the Unix epoch, or
    /// zero where the store records no time.
    pub created_at: u64,
}

/// An object's metadata, as [`Container::object_info`] reports it.
#[derive(Clone, Debug)]
pub struct ObjectMetadata {
    /// The object's name.
    pub name: String,
    /// When the object was written, in seconds since the Unix epoch.
    pub created_at: u64,
    /// The object's size in bytes.
    pub size: u64,
}

/// One page of the objects [`Container::list_objects`] lists.
#[derive(Clone, Debug)]
pub struct Page {
    /// The objects on this page, in name order.
    pub objects: Vec<ObjectMetadata>,
    /// The cursor that resumes the listing after this page, `None` on the
    /// last page.
    pub next: Option<String>,
}
