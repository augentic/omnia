//! Wasmtime stream producers/consumers and reply task for [`ToolSession`].

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::sync::mpsc;
use wasmtime::StoreContextMut;
use wasmtime::component::{Destination, Source, StreamConsumer, StreamProducer, StreamResult};

use crate::host::ToolSession;
use crate::host::generated::omnia::model::completion::{
    Error as WitError, Reply as WitReply, ToolCall as WitToolCall, ToolResult as WitToolResult,
};

/// The spawned reply pipeline: the backend future piped through the answer
/// gate, started eagerly at `create` time.
pub struct ReplyTask {
    handle: tokio::task::JoinHandle<Result<WitReply, WitError>>,
}

impl ReplyTask {
    pub fn spawn(
        pipeline: impl Future<Output = Result<WitReply, WitError>> + Send + 'static,
    ) -> Self {
        Self {
            handle: tokio::spawn(pipeline),
        }
    }

    /// Await the pipeline's outcome; dropping this future aborts the task.
    pub async fn join(mut self) -> Result<WitReply, WitError> {
        match (&mut self.handle).await {
            Ok(result) => result,
            Err(error) => Err(WitError::Backend(format!("reply pipeline failed: {error}"))),
        }
    }
}

impl Drop for ReplyTask {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// Drains session tool calls toward the guest.
pub struct CallsProducer {
    rx: mpsc::Receiver<omnia_backend::model::ToolCall>,
}

impl CallsProducer {
    pub const fn new(rx: mpsc::Receiver<omnia_backend::model::ToolCall>) -> Self {
        Self { rx }
    }
}

impl<D> StreamProducer<D> for CallsProducer {
    type Buffer = Option<WitToolCall>;
    type Item = WitToolCall;

    fn poll_produce(
        self: Pin<&mut Self>, cx: &mut Context<'_>, _store: StoreContextMut<D>,
        mut destination: Destination<'_, Self::Item, Self::Buffer>, finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        let this = self.get_mut();
        match this.rx.poll_recv(cx) {
            Poll::Ready(Some(call)) => {
                destination.set_buffer(Some(WitToolCall {
                    id: call.id,
                    name: call.name,
                    arguments: call.arguments,
                }));
                Poll::Ready(Ok(StreamResult::Completed))
            }
            Poll::Ready(None) => Poll::Ready(Ok(StreamResult::Dropped)),
            Poll::Pending if finish => Poll::Ready(Ok(StreamResult::Cancelled)),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Routes guest tool results to their pending calls.
pub struct ResultsConsumer {
    state: Arc<ToolSession>,
}

impl ResultsConsumer {
    pub const fn new(state: Arc<ToolSession>) -> Self {
        Self { state }
    }
}

impl Drop for ResultsConsumer {
    fn drop(&mut self) {
        self.state.shutdown();
    }
}

impl<D> StreamConsumer<D> for ResultsConsumer {
    type Item = WitToolResult;

    fn poll_consume(
        self: Pin<&mut Self>, _cx: &mut Context<'_>, mut store: StoreContextMut<D>,
        mut source: Source<'_, Self::Item>, finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        if self.state.dead() {
            return Poll::Ready(Ok(StreamResult::Dropped));
        }
        let mut buffer: Vec<WitToolResult> = Vec::with_capacity(8);
        source.read(&mut store, &mut buffer)?;
        let took = buffer.len();
        for result in buffer {
            self.state.deliver(&result.id, result.output);
        }
        if took == 0 && finish {
            return Poll::Ready(Ok(StreamResult::Cancelled));
        }
        Poll::Ready(Ok(StreamResult::Completed))
    }
}
