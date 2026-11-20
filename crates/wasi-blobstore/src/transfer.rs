//! The plumbing that carries a guest's `put` into a backend's whole-object
//! write.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, ready};

use bytes::BytesMut;
use futures::FutureExt as _;
use futures::channel::oneshot;
use futures::future::BoxFuture;
use parking_lot::Mutex;
use wasmtime::StoreContextMut;
use wasmtime::component::{FutureConsumer, Source, StreamConsumer, StreamResult};

use crate::host::container_impl::ContainerHandle;
use crate::host::generated::omnia::blobstore::types::Stored;
use crate::host::{Error, Result};

// The bytes one `put` has received. It empties to `None` once `finished`
// resolves, which refuses any later write.
#[derive(Clone)]
pub struct Staging(Arc<Mutex<Option<BytesMut>>>);

impl Default for Staging {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Some(BytesMut::new()))))
    }
}

impl Staging {
    // `false` once `finished` has resolved
    fn extend(&self, bytes: &[u8]) -> bool {
        self.0.lock().as_mut().map(|buffer| buffer.extend_from_slice(bytes)).is_some()
    }

    fn take(&self) -> BytesMut {
        self.0.lock().take().unwrap_or_default()
    }
}

// Takes a guest's `data` stream into its staging.
pub struct Collect(pub Staging);

impl<D> StreamConsumer<D> for Collect {
    type Item = u8;

    fn poll_consume(
        self: Pin<&mut Self>, _cx: &mut Context<'_>, store: StoreContextMut<D>,
        source: Source<'_, Self::Item>, finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        let mut source = source.as_direct(store);
        let bytes = source.remaining();
        let count = bytes.len();
        if count == 0 && finish {
            return Poll::Ready(Ok(StreamResult::Cancelled));
        }
        if !self.0.extend(bytes) {
            return Poll::Ready(Ok(StreamResult::Dropped));
        }
        source.mark_read(count);
        Poll::Ready(Ok(StreamResult::Completed))
    }
}

// Commits one `put` when the guest resolves its `finished` future, whether or
// not the guest awaits the reply.
pub struct Commit {
    write: Option<Write>,
    writing: Option<BoxFuture<'static, Result<Stored>>>,
    reply: Option<oneshot::Sender<Result<Stored>>>,
}

impl Commit {
    pub fn new(
        container: ContainerHandle, name: String, size: Option<u64>, staging: Staging,
        reply: oneshot::Sender<Result<Stored>>,
    ) -> Self {
        Self {
            write: Some(Write {
                container,
                name,
                size,
                staging,
            }),
            writing: None,
            reply: Some(reply),
        }
    }

    fn answer(&mut self, stored: Result<Stored>) {
        if let Some(reply) = self.reply.take() {
            // a guest that dropped the reply no longer waits for it
            drop(reply.send(stored));
        }
    }
}

impl<D> FutureConsumer<D> for Commit {
    type Item = Result<()>;

    fn poll_consume(
        self: Pin<&mut Self>, cx: &mut Context<'_>, mut store: StoreContextMut<D>,
        mut source: Source<'_, Self::Item>, _finish: bool,
    ) -> Poll<wasmtime::Result<()>> {
        let this = self.get_mut();

        // take the guest's verdict
        if let Some(write) = this.write.take() {
            let mut verdict = None;
            source.read(&mut store, &mut verdict)?;
            match verdict {
                // a cancelled write leaves the guest free to resolve `finished` later
                None => this.write = Some(write),
                Some(Ok(())) => this.writing = Some(write.commit()),
                Some(Err(error)) => {
                    write.staging.take();
                    this.answer(Err(error));
                }
            }
        }

        // store the object
        if let Some(writing) = &mut this.writing {
            let stored = ready!(writing.poll_unpin(cx));
            this.writing = None;
            this.answer(stored);
        }
        Poll::Ready(Ok(()))
    }
}

struct Write {
    container: ContainerHandle,
    name: String,
    size: Option<u64>,
    staging: Staging,
}

impl Write {
    fn commit(self) -> BoxFuture<'static, Result<Stored>> {
        let data = self.staging.take().freeze();
        async move {
            if let Some(size) = self.size
                && size != data.len() as u64
            {
                return Err(Error::Invalid(format!(
                    "`{}` was put as {size} bytes, and its stream held {}",
                    self.name,
                    data.len()
                )));
            }
            self.container.write(self.name, data).await
        }
        .boxed()
    }
}
