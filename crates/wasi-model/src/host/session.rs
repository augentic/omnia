//! Per-completion session plumbing between the backend's `ToolHost` and the
//! guest's session streams.
//!
//! The backend stays store-free behind channels: `call-tool` pushes a
//! `tool-call` into an mpsc bridge that [`CallsProducer`] drains toward the
//! guest, and the guest's results stream is piped into [`ResultsConsumer`],
//! which answers the pending oneshot for each call id. Host enforcement —
//! declared-tool check, call budget, result size cap, per-call timeout —
//! lives here, once, and records a typed [`Error`] that the reply pipeline
//! prefers over the backend's own failure.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::anyhow;
use futures::FutureExt as _;
use tokio::sync::{mpsc, oneshot};
use wasmtime::StoreContextMut;
use wasmtime::component::{Destination, Source, StreamConsumer, StreamProducer, StreamResult};

use crate::host::generated::omnia::model::completion::{Reply, ToolCall, ToolResult};
use crate::host::{Error, FutureResult, ToolOutcome};

const CALL_QUEUE: usize = 8;

/// Shared state for one completion session: the calls bridge, the pending
/// calls awaiting guest results, the call budget, and the first typed
/// host-enforcement failure.
pub struct ToolSession {
    limits: Limits,
    // Function-tool names used by request, the only names `call-tool` accepts.
    allowed: Vec<String>,
    inner: Mutex<Inner>,
}

/// Session bounds the host enforces per completion, in `wasi:model`,
/// regardless of backend.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Tool calls one completion may issue before `budget-exhausted`.
    pub max_tool_calls: u32,
    /// Byte cap on a single tool result's output.
    pub max_result_bytes: usize,
    /// How long the host waits for the guest to answer one tool call.
    pub tool_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_tool_calls: 32,
            max_result_bytes: 1 << 20,
            tool_timeout: Duration::from_secs(60),
        }
    }
}

struct Inner {
    // Write end of the calls bridge; `None` once the host closed the stream.
    calls: Option<mpsc::Sender<ToolCall>>,
    pending: HashMap<String, oneshot::Sender<ToolOutcome>>,
    remaining: u32,
    next_id: u64,
    error: Option<Error>,
}

struct PendingCall {
    id: String,
    calls: mpsc::Sender<ToolCall>,
    result: oneshot::Receiver<ToolOutcome>,
}

impl ToolSession {
    pub fn new(limits: Limits, allowed: Vec<String>) -> (Arc<Self>, mpsc::Receiver<ToolCall>) {
        let (calls, calls_rx) = mpsc::channel(CALL_QUEUE);
        let state = Self {
            limits,
            allowed,
            inner: Mutex::new(Inner {
                calls: Some(calls),
                pending: HashMap::new(),
                remaining: limits.max_tool_calls,
                next_id: 0,
                error: None,
            }),
        };
        (Arc::new(state), calls_rx)
    }

    /// Take the typed failure host enforcement recorded, if any. The reply
    /// pipeline prefers it over whatever the backend then returned.
    pub fn take_error(&self) -> Option<Error> {
        self.lock().error.take()
    }

    /// Run one declared function tool through the session; see
    /// `ToolHost::call_tool` for the error contract.
    pub fn call(self: Arc<Self>, name: String, arguments: String) -> FutureResult<ToolOutcome> {
        async move {
            let pending = self.reserve(&name)?;

            let call = ToolCall {
                id: pending.id.clone(),
                name: name.clone(),
                arguments,
            };
            if pending.calls.send(call).await.is_err() {
                self.lock().pending.remove(&pending.id);
                return Err(anyhow!(
                    "guest dropped the calls stream before receiving tool call `{name}`"
                ));
            }

            let output = self.wait_for_result(&pending.id, &name, pending.result).await?;
            self.check_result(&name, output)
        }
        .boxed()
    }

    fn reserve(&self, name: &str) -> anyhow::Result<PendingCall> {
        if !self.allowed.iter().any(|allowed| allowed == name) {
            return Err(self.fail(Error::ToolFailed(format!(
                "model called `{name}`, which the request does not declare as a function tool"
            ))));
        }

        let mut inner = self.lock();
        let Some(calls) = inner.calls.clone() else {
            return Err(anyhow!("tool call `{name}` after the session closed its calls stream"));
        };
        if inner.remaining == 0 {
            inner.calls = None;
            return Err(record_error(
                &mut inner,
                Error::BudgetExhausted(format!(
                    "tool-call budget of {} exhausted at `{name}`",
                    self.limits.max_tool_calls
                )),
            ));
        }

        inner.remaining -= 1;
        inner.next_id += 1;
        let id = format!("call-{}", inner.next_id);
        let (sender, result) = oneshot::channel();
        inner.pending.insert(id.clone(), sender);
        drop(inner);
        Ok(PendingCall { id, calls, result })
    }

    async fn wait_for_result(
        &self, id: &str, name: &str, result: oneshot::Receiver<ToolOutcome>,
    ) -> anyhow::Result<ToolOutcome> {
        match tokio::time::timeout(self.limits.tool_timeout, result).await {
            Err(_elapsed) => {
                let mut inner = self.lock();
                inner.pending.remove(id);
                inner.calls = None;
                let error = record_error(
                    &mut inner,
                    Error::BudgetExhausted(format!(
                        "tool call `{name}` got no result within {:?}",
                        self.limits.tool_timeout
                    )),
                );
                drop(inner);
                Err(error)
            }
            Ok(Err(_closed)) => {
                Err(anyhow!("guest closed its results stream before answering tool call `{name}`"))
            }
            Ok(Ok(output)) => Ok(output),
        }
    }

    fn check_result(&self, name: &str, output: ToolOutcome) -> anyhow::Result<ToolOutcome> {
        if let Ok(text) = &output
            && text.len() > self.limits.max_result_bytes
        {
            return Err(self.fail(Error::ToolFailed(format!(
                "tool `{name}` result of {} bytes exceeds the {}-byte cap",
                text.len(),
                self.limits.max_result_bytes
            ))));
        }
        Ok(output)
    }

    // Deliver one guest result to its pending call. Unknown ids (stale after
    // a timeout, or fabricated) are dropped, not fatal.
    fn deliver(&self, result: ToolResult) {
        let sender = self.lock().pending.remove(&result.id);
        if let Some(sender) = sender {
            // A rejected send means the caller stopped waiting; stale either way.
            drop(sender.send(result.output));
        } else {
            tracing::trace!(id = %result.id, "dropping tool result with no pending call");
        }
    }

    // End the session: close the calls stream toward the guest and fail
    // every pending waiter. Runs when the reply pipeline finishes or is
    // cancelled, and when the guest closes its results stream.
    fn shutdown(&self) {
        let mut inner = self.lock();
        inner.calls = None;
        inner.pending.clear();
    }

    // The session can no longer consume results: the calls stream is closed
    // and no call awaits an answer. Rejecting further writes lets a guest
    // still holding the results writer observe the session's end.
    fn dead(&self) -> bool {
        let inner = self.lock();
        inner.calls.is_none() && inner.pending.is_empty()
    }

    fn fail(&self, error: Error) -> anyhow::Error {
        record_error(&mut self.lock(), error)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

// Record `error` as the session's typed failure (the first one wins) and
// return the matching hard error for the backend.
fn record_error(inner: &mut Inner, error: Error) -> anyhow::Error {
    let hard = anyhow!(describe(&error));
    inner.error.get_or_insert(error);
    hard
}

fn describe(error: &Error) -> String {
    match error {
        Error::InvalidRequest(detail) => format!("invalid request: {detail}"),
        Error::InvalidAnswer(detail) => format!("invalid answer: {detail}"),
        Error::BudgetExhausted(detail) => format!("budget exhausted: {detail}"),
        Error::ToolFailed(detail) => format!("tool failed: {detail}"),
        Error::Backend(detail) => format!("backend failure: {detail}"),
    }
}

/// The spawned reply pipeline: the backend future piped through the answer
/// gate, started eagerly at `create` time.
///
/// Eager because wasmtime polls a `FutureReader` producer only once the
/// guest awaits the reply — but a session guest reads `calls` first, so a
/// lazy backend would deadlock the session before its first tool call.
/// Cancellation stays structural: dropping this task (the guest dropped its
/// reply future, so wasmtime dropped the producer awaiting it) aborts the
/// pipeline, whose [`SessionClose`] guard then ends the session.
///
/// Requires a tokio runtime, which every `omnia` deployment provides.
pub struct ReplyTask {
    handle: tokio::task::JoinHandle<Result<Reply, Error>>,
}

impl ReplyTask {
    pub fn spawn(pipeline: impl Future<Output = Result<Reply, Error>> + Send + 'static) -> Self {
        Self {
            handle: tokio::spawn(pipeline),
        }
    }

    /// Await the pipeline's outcome; dropping this future aborts the task.
    pub async fn join(mut self) -> Result<Reply, Error> {
        match (&mut self.handle).await {
            Ok(result) => result,
            // Cancelled cannot be observed here (aborting requires dropping
            // `self`), so this is a pipeline panic.
            Err(error) => Err(Error::Backend(format!("reply pipeline failed: {error}"))),
        }
    }
}

impl Drop for ReplyTask {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// Ends the session when dropped: the calls stream closes toward the guest
/// and pending waiters fail — whether the reply pipeline completed normally
/// or was cancelled by the guest dropping its reply future. Without it a
/// guest joining the calls loop with the reply would wait on an open calls
/// stream forever.
pub struct SessionClose {
    state: Arc<ToolSession>,
}

impl SessionClose {
    pub const fn new(state: Arc<ToolSession>) -> Self {
        Self { state }
    }
}

impl Drop for SessionClose {
    fn drop(&mut self) {
        self.state.shutdown();
    }
}

/// Drains session tool calls toward the guest; a closed bridge (budget or
/// deadline enforcement, or the session dropping) closes the stream.
pub struct CallsProducer {
    rx: mpsc::Receiver<ToolCall>,
}

impl CallsProducer {
    pub const fn new(rx: mpsc::Receiver<ToolCall>) -> Self {
        Self { rx }
    }
}

impl<D> StreamProducer<D> for CallsProducer {
    type Buffer = Option<ToolCall>;
    type Item = ToolCall;

    fn poll_produce(
        self: Pin<&mut Self>, cx: &mut Context<'_>, _store: StoreContextMut<D>,
        mut destination: Destination<'_, Self::Item, Self::Buffer>, finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        let this = self.get_mut();
        match this.rx.poll_recv(cx) {
            Poll::Ready(Some(call)) => {
                destination.set_buffer(Some(call));
                Poll::Ready(Ok(StreamResult::Completed))
            }
            Poll::Ready(None) => Poll::Ready(Ok(StreamResult::Dropped)),
            Poll::Pending if finish => Poll::Ready(Ok(StreamResult::Cancelled)),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Routes guest tool results to their pending calls; its drop (the guest
/// closed the results stream, or store teardown) fails every waiter.
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
    type Item = ToolResult;

    fn poll_consume(
        self: Pin<&mut Self>, _cx: &mut Context<'_>, mut store: StoreContextMut<D>,
        mut source: Source<'_, Self::Item>, finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        if self.state.dead() {
            return Poll::Ready(Ok(StreamResult::Dropped));
        }
        let mut buffer: Vec<ToolResult> = Vec::with_capacity(8);
        source.read(&mut store, &mut buffer)?;
        let took = buffer.len();
        for result in buffer {
            self.state.deliver(result);
        }
        if took == 0 && finish {
            return Poll::Ready(Ok(StreamResult::Cancelled));
        }
        Poll::Ready(Ok(StreamResult::Completed))
    }
}
