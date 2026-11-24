//! Stages a guest's `put` stream in memory and writes the object once
//! `finished` resolves.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, ready};

use bytes::{Bytes, BytesMut};
use futures::channel::oneshot;
use parking_lot::Mutex;
use wasmtime::component::{
    FutureConsumer, FutureReader, Source, StreamConsumer, StreamReader, StreamResult,
};
use wasmtime::{AsContextMut, StoreContextMut};

use crate::container_impl::ContainerHandle;
use crate::generated::omnia::blobstore::types::{Conflict, Stored};
use crate::{Error, Result};

type PutOutcome = Result<std::result::Result<Stored, Conflict>>;
type SharedBuffer = Arc<Mutex<Option<BytesMut>>>;
type WriteFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

pub struct StagedPut {
    container: ContainerHandle,
    name: String,
    expected_size: Option<u64>,
}

impl StagedPut {
    pub const fn new(container: ContainerHandle, name: String, expected_size: Option<u64>) -> Self {
        Self {
            container,
            name,
            expected_size,
        }
    }

    pub fn pipe<S: AsContextMut>(
        self, mut store: S, data: StreamReader<u8>, mut finished: FutureReader<Result<()>>,
    ) -> wasmtime::Result<FutureReader<PutOutcome>> {
        let (collector, body) = StagedBody::pair();
        let (reply, outcome) = Reply::pair();

        if let Err(error) = data.pipe(&mut store, collector) {
            finished.close(&mut store)?;
            return Err(error);
        }
        finished.pipe(&mut store, PutCommitter::new(self, body, reply))?;

        FutureReader::new(&mut store, async move {
            let stored = outcome.await.unwrap_or_else(|_| {
                Err(Error::Other("the write ended before `finished` resolved".to_owned()))
            });
            Ok::<_, wasmtime::Error>(stored.map(Ok))
        })
    }

    fn write(self, data: Bytes, reply: Reply) -> Option<WriteFuture> {
        if let Some(expected) = self.expected_size
            && expected != data.len() as u64
        {
            let error = Error::Invalid(format!(
                "`{}` was put as {expected} bytes, and its stream held {}",
                self.name,
                data.len()
            ));
            reply.send(Err(error));
            return None;
        }

        Some(Box::pin(async move {
            let stored = self.container.write(self.name, data).await;
            reply.send(stored);
        }))
    }
}

// Paired write and read halves over one staged buffer. Dropping `StagedBody`
// clears the buffer so any later stream write sees `StreamResult::Dropped`.
struct StagedBody(SharedBuffer);

impl StagedBody {
    fn pair() -> (StreamCollector, Self) {
        let buffer = Arc::new(Mutex::new(Some(BytesMut::new())));
        (StreamCollector(Arc::clone(&buffer)), Self(buffer))
    }

    fn take(self) -> Bytes {
        let buffer = self.0.lock().take();
        buffer.unwrap_or_default().freeze()
    }
}

impl Drop for StagedBody {
    fn drop(&mut self) {
        let buffer = self.0.lock().take();
        drop(buffer);
    }
}

struct StreamCollector(SharedBuffer);

impl StreamCollector {
    fn append(&self, bytes: &[u8]) -> bool {
        self.0.lock().as_mut().is_some_and(|buffer| {
            buffer.extend_from_slice(bytes);
            true
        })
    }
}

impl<D> StreamConsumer<D> for StreamCollector {
    type Item = u8;

    fn poll_consume(
        self: Pin<&mut Self>, _cx: &mut Context<'_>, store: StoreContextMut<D>,
        source: Source<'_, Self::Item>, finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        let mut source = source.as_direct(store);
        let bytes = source.remaining()?;
        if bytes.is_empty() && finish {
            return Poll::Ready(Ok(StreamResult::Cancelled));
        }
        if !self.append(bytes) {
            return Poll::Ready(Ok(StreamResult::Dropped));
        }
        let count = bytes.len();
        source.mark_read(count);
        Poll::Ready(Ok(StreamResult::Completed))
    }
}

// Driven by `finished` rather than the reply future, so dropping the reply
// still commits the write. Once the verdict is taken from `source`,
// cancelling `finished` cannot put it back, so `Writing` still runs to
// completion when `finish` is set.
enum PutCommitter {
    AwaitingVerdict(PendingPut),
    Writing(WriteFuture),
    Done,
}

impl PutCommitter {
    const fn new(put: StagedPut, body: StagedBody, reply: Reply) -> Self {
        Self::AwaitingVerdict(PendingPut { put, body, reply })
    }
}

impl<D> FutureConsumer<D> for PutCommitter {
    type Item = Result<()>;

    fn poll_consume(
        self: Pin<&mut Self>, cx: &mut Context<'_>, mut store: StoreContextMut<D>,
        mut source: Source<'_, Self::Item>, finish: bool,
    ) -> Poll<wasmtime::Result<()>> {
        let this = self.get_mut();

        // resolve the guest's verdict
        if matches!(this, Self::AwaitingVerdict(_)) {
            if finish {
                return Poll::Ready(Ok(()));
            }
            if let Self::AwaitingVerdict(pending) = std::mem::replace(this, Self::Done) {
                let mut verdict = None;
                source.read(&mut store, &mut verdict)?;
                match verdict {
                    None => *this = Self::AwaitingVerdict(pending),
                    Some(Ok(())) => {
                        if let Some(writing) = pending.commit() {
                            *this = Self::Writing(writing);
                        }
                    }
                    Some(Err(error)) => pending.abort(error),
                }
            }
        }

        // drive the backend write
        if let Self::Writing(writing) = this {
            ready!(writing.as_mut().poll(cx));
            *this = Self::Done;
        }

        Poll::Ready(Ok(()))
    }
}

struct PendingPut {
    put: StagedPut,
    body: StagedBody,
    reply: Reply,
}

impl PendingPut {
    fn commit(self) -> Option<WriteFuture> {
        let data = self.body.take();
        self.put.write(data, self.reply)
    }

    fn abort(self, error: Error) {
        drop(self.body);
        self.reply.send(Err(error));
    }
}

struct Reply(oneshot::Sender<Result<Stored>>);

impl Reply {
    fn pair() -> (Self, oneshot::Receiver<Result<Stored>>) {
        let (sender, receiver) = oneshot::channel();
        (Self(sender), receiver)
    }

    // A guest that dropped its reply future no longer waits for the answer.
    fn send(self, outcome: Result<Stored>) {
        drop(self.0.send(outcome));
    }
}
