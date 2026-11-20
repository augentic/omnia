//! Per-completion session plumbing between the backend's `ToolHost` and the
//! guest's session streams.
//!
//! The backend stays store-free behind channels: `call-tool` and `check`
//! push a `call` into an mpsc bridge that [`CallsProducer`] drains toward
//! the guest, and the guest's results stream is piped into
//! [`ResultsConsumer`], which answers the pending oneshot for each call id.
//! Host enforcement — declared-tool check, call budget, result size cap,
//! per-call timeout, result kind — lives here, once, and records a [`Halt`]
//! that the reply pipeline prefers over the backend's own outcome. A check
//! rides the same bridge outside the declared-tool check and the call
//! budget.

use std::collections::HashMap;
use std::fmt;
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

use crate::generated::omnia::model::types::{self, Call, CallResult, Candidate, ToolCall};
use crate::{Budget, Error, FutureResult, ToolOutcome};

const CALL_QUEUE: usize = 8;

// The guest's `reply`: an answer, an exhaustion, or an error.
pub type Replied = Result<Result<types::Reply, types::Exhausted>, Error>;

// Shared state for one completion session: the calls bridge, the pending
// calls awaiting guest results, the call budget, and the first halt.
pub struct ToolSession {
    limits: Limits,
    // tool names used by request, the only names `call-tool` accepts.
    allowed: Vec<String>,
    inner: Mutex<Inner>,
}

impl ToolSession {
    pub fn new(limits: Limits, allowed: Vec<String>) -> (Arc<Self>, mpsc::Receiver<Call>) {
        let (calls, calls_rx) = mpsc::channel(CALL_QUEUE);
        let state = Self {
            limits,
            allowed,
            inner: Mutex::new(Inner {
                calls: Some(calls),
                pending: HashMap::new(),
                remaining: limits.max_tool_calls,
                next_id: 0,
                halt: None,
            }),
        };
        (Arc::new(state), calls_rx)
    }

    // The reply pipeline prefers a halt over whatever the backend then
    // returned.
    pub fn take_halt(&self) -> Option<Halt> {
        self.lock().halt.take()
    }

    // Run one of the request's tools through the session; see
    // `ToolHost::call_tool` for the error contract.
    pub fn call(self: Arc<Self>, name: String, arguments: String) -> FutureResult<ToolOutcome> {
        async move {
            if !self.allowed.contains(&name) {
                return Err(self.halt(Halt::Other(format!(
                    "model called `{name}`, which the request does not declare as a tool"
                ))));
            }
            let pending = self.reserve(Ask::Tool(&name), Waiter::Tool)?;
            let call = Call::Tool(ToolCall {
                id: pending.id.clone(),
                name: name.clone(),
                arguments,
            });
            let outcome = self.exchange(pending, call, Ask::Tool(&name)).await?;
            let (kind, text) = match &outcome {
                Ok(output) => ("result", output),
                Err(failure) => ("failure", failure),
            };
            self.cap(&format!("tool `{name}` {kind}"), text)?;
            Ok(outcome)
        }
        .boxed()
    }

    // Ask the guest to judge `candidate` through a `check` call; see
    // `ToolHost::check` for the contract.
    pub fn check(self: Arc<Self>, candidate: String) -> FutureResult<Result<(), String>> {
        async move {
            let pending = self.reserve(Ask::Check, Waiter::Check)?;
            let call = Call::Check(Candidate {
                id: pending.id.clone(),
                answer: candidate,
            });
            let outcome = self.exchange(pending, call, Ask::Check).await?;
            // the correction reaches the model, so the result cap bounds it
            if let Err(correction) = &outcome {
                self.cap("check correction", correction)?;
            }
            Ok(outcome)
        }
        .boxed()
    }

    // Send one call toward the guest and await its result.
    async fn exchange<T>(
        &self, pending: PendingCall<T>, call: Call, ask: Ask<'_>,
    ) -> anyhow::Result<T> {
        if pending.calls.send(call).await.is_err() {
            self.lock().pending.remove(&pending.id);
            return Err(self.halt(Halt::Invalid(format!(
                "the guest dropped its calls stream before receiving {ask}"
            ))));
        }
        self.wait_for_result(&pending.id, ask, pending.result).await
    }

    // Mint a pending call. A tool call spends the tool-call budget.
    fn reserve<T>(
        &self, ask: Ask<'_>, waiter: fn(oneshot::Sender<T>) -> Waiter,
    ) -> anyhow::Result<PendingCall<T>> {
        let mut inner = self.lock();
        let Some(calls) = inner.calls.clone() else {
            return Err(inner.record(Halt::Invalid(format!(
                "the guest closed its results stream before {ask}"
            ))));
        };
        if let Ask::Tool(name) = ask {
            if inner.remaining == 0 {
                inner.calls = None;
                return Err(inner.record(Halt::Exhausted(
                    Budget::ToolCalls,
                    format!(
                        "tool-call budget of {} exhausted at `{name}`",
                        self.limits.max_tool_calls
                    ),
                )));
            }
            inner.remaining -= 1;
        }

        inner.next_id += 1;
        let id = format!("{}-{}", ask.prefix(), inner.next_id);
        let (sender, result) = oneshot::channel();
        inner.pending.insert(id.clone(), waiter(sender));
        drop(inner);
        Ok(PendingCall { id, calls, result })
    }

    async fn wait_for_result<T>(
        &self, id: &str, ask: Ask<'_>, result: oneshot::Receiver<T>,
    ) -> anyhow::Result<T> {
        match tokio::time::timeout(self.limits.tool_timeout, result).await {
            Err(_elapsed) => {
                let mut inner = self.lock();
                inner.pending.remove(id);
                inner.calls = None;
                Err(inner.record(Halt::Exhausted(
                    Budget::CallTime,
                    format!("{ask} got no result within {:?}", self.limits.tool_timeout),
                )))
            }
            Ok(Err(_closed)) => Err(self.halt(Halt::Invalid(format!(
                "the guest closed its results stream before answering {ask}"
            )))),
            Ok(Ok(output)) => Ok(output),
        }
    }

    // Fail hard when `text`, bound for the model, exceeds the result cap.
    fn cap(&self, what: &str, text: &str) -> anyhow::Result<()> {
        if text.len() > self.limits.max_result_bytes {
            return Err(self.halt(Halt::Invalid(format!(
                "{what} of {} bytes exceeds the {}-byte cap",
                text.len(),
                self.limits.max_result_bytes
            ))));
        }
        Ok(())
    }

    // Deliver one guest result to its pending call. Unknown ids (stale after
    // a timeout, or fabricated) are dropped, not fatal. A result of the
    // wrong kind ends the session.
    fn deliver(&self, result: CallResult) {
        let id = match &result {
            CallResult::Tool(result) => result.id.clone(),
            CallResult::Check(verdict) => verdict.id.clone(),
        };
        let mut inner = self.lock();
        let Some(waiter) = inner.pending.remove(&id) else {
            tracing::trace!(%id, "dropping a result with no pending call");
            return;
        };

        // a rejected send means the caller stopped waiting
        let (call, answer) = match (waiter, result) {
            (Waiter::Tool(sender), CallResult::Tool(result)) => {
                drop(sender.send(result.output));
                return;
            }
            (Waiter::Check(sender), CallResult::Check(verdict)) => {
                drop(sender.send(verdict.outcome));
                return;
            }
            (Waiter::Tool(_), CallResult::Check(_)) => ("tool call", "verdict"),
            (Waiter::Check(_), CallResult::Tool(_)) => ("check", "tool result"),
        };
        inner.halt.get_or_insert_with(|| {
            Halt::Invalid(format!("the guest answered the {call} `{id}` with a {answer}"))
        });
        inner.end();
    }

    // Runs when the reply pipeline ends, and when the guest closes its results stream.
    fn shutdown(&self) {
        self.lock().end();
    }

    // Runs when the guest drops its calls stream, which leaves any call it
    // has yet to answer unanswerable.
    fn calls_dropped(&self) {
        let mut inner = self.lock();
        if !inner.pending.is_empty() {
            inner.halt.get_or_insert_with(|| {
                Halt::Invalid(
                    "the guest dropped its calls stream with a call outstanding".to_owned(),
                )
            });
            inner.end();
        }
    }

    // Rejecting further writes lets a guest still holding the results writer
    // observe the session's end.
    fn dead(&self) -> bool {
        let inner = self.lock();
        inner.calls.is_none() && inner.pending.is_empty()
    }

    fn halt(&self, halt: Halt) -> anyhow::Error {
        self.lock().record(halt)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

struct Inner {
    // Write end of the calls bridge; `None` once the host closed the stream.
    calls: Option<mpsc::Sender<Call>>,
    pending: HashMap<String, Waiter>,
    remaining: u32,
    next_id: u64,
    halt: Option<Halt>,
}

impl Inner {
    // The first halt wins the reply, while the error the backend sees names
    // this one.
    fn record(&mut self, halt: Halt) -> anyhow::Error {
        let error = anyhow!("{halt}");
        self.halt.get_or_insert(halt);
        error
    }

    fn end(&mut self) {
        self.calls = None;
        self.pending.clear();
    }
}

// Why host enforcement ended a completion, whatever its backend returns.
pub enum Halt {
    Invalid(String),
    Exhausted(Budget, String),
    Other(String),
}

impl Halt {
    pub fn into_reply(self) -> Replied {
        match self {
            Self::Invalid(detail) => Err(Error::Invalid(detail)),
            Self::Exhausted(budget, detail) => Ok(Err(types::Exhausted {
                budget,
                detail,
                usage: None,
            })),
            Self::Other(detail) => Err(Error::Other(detail)),
        }
    }
}

impl fmt::Display for Halt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(detail) => write!(f, "invalid: {detail}"),
            Self::Exhausted(_, detail) => write!(f, "exhausted: {detail}"),
            Self::Other(detail) => write!(f, "other: {detail}"),
        }
    }
}

// A call the host makes of the guest, as the failures that report it name it.
#[derive(Clone, Copy)]
enum Ask<'a> {
    Tool(&'a str),
    Check,
}

impl Ask<'_> {
    const fn prefix(self) -> &'static str {
        match self {
            Self::Tool(_) => "call",
            Self::Check => "check",
        }
    }
}

impl fmt::Display for Ask<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tool(name) => write!(f, "tool call `{name}`"),
            Self::Check => f.write_str("the check"),
        }
    }
}

// The caller awaiting a call's result, which fixes the kind of result that
// answers it.
enum Waiter {
    Tool(oneshot::Sender<ToolOutcome>),
    Check(oneshot::Sender<Result<(), String>>),
}

struct PendingCall<T> {
    id: String,
    calls: mpsc::Sender<Call>,
    result: oneshot::Receiver<T>,
}

const MAX_RESULT_BYTES: usize = 1024 * 1024;

/// Session bounds the host enforces per completion, in `omnia:model`,
/// regardless of backend.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Tool calls one completion may issue before it is `exhausted`.
    pub max_tool_calls: u32,
    /// Byte cap on a tool's output or failure text, and on a check's
    /// correction. A larger one fails the completion as `invalid`.
    pub max_result_bytes: usize,
    /// How long the host waits for the guest to answer one call before the
    /// completion is `exhausted`.
    pub tool_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_tool_calls: 32,
            max_result_bytes: MAX_RESULT_BYTES,
            tool_timeout: Duration::from_secs(60),
        }
    }
}

// The spawned reply pipeline: the backend future piped through the halt
// check, started eagerly when a completion begins.
//
// Eager because wasmtime polls a `FutureReader` producer only once the
// guest awaits the reply — but a session guest reads `calls` first, so a
// lazy backend would deadlock the session before its first call.
// Cancellation stays structural: dropping this task (the guest dropped its
// reply future, so wasmtime dropped the producer awaiting it) aborts the
// pipeline, whose `SessionClose` guard then ends the session.
//
// Requires a tokio runtime, which every `omnia` deployment provides.
pub struct ReplyTask {
    handle: tokio::task::JoinHandle<Replied>,
}

impl ReplyTask {
    pub fn spawn(pipeline: impl Future<Output = Replied> + Send + 'static) -> Self {
        Self {
            handle: tokio::spawn(pipeline),
        }
    }

    // Dropping this future aborts the task.
    pub async fn join(mut self) -> Replied {
        match (&mut self.handle).await {
            Ok(replied) => replied,
            // aborting requires dropping `self`, so this is a pipeline panic
            Err(error) => Err(Error::Other(format!("reply pipeline failed: {error}"))),
        }
    }
}

impl Drop for ReplyTask {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

// Ends the session when dropped: the calls stream closes toward the guest
// and pending waiters fail — whether the reply pipeline completed normally
// or was cancelled by the guest dropping its reply future. Without it a
// guest joining the calls loop with the reply would wait on an open calls
// stream forever.
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

// Drains session calls toward the guest; a closed bridge (budget or
// deadline enforcement, or the session ending) closes the stream. Its drop
// (the guest dropped the calls stream, or store teardown) fails any call
// still outstanding.
pub struct CallsProducer {
    rx: mpsc::Receiver<Call>,
    state: Arc<ToolSession>,
}

impl CallsProducer {
    pub const fn new(rx: mpsc::Receiver<Call>, state: Arc<ToolSession>) -> Self {
        Self { rx, state }
    }
}

impl Drop for CallsProducer {
    fn drop(&mut self) {
        self.state.calls_dropped();
    }
}

impl<D> StreamProducer<D> for CallsProducer {
    type Buffer = Option<Call>;
    type Item = Call;

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

// Routes guest results to their pending calls; its drop (the guest closed
// the results stream, or store teardown) fails every waiter.
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
    type Item = CallResult;

    fn poll_consume(
        self: Pin<&mut Self>, _cx: &mut Context<'_>, mut store: StoreContextMut<D>,
        mut source: Source<'_, Self::Item>, finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        if self.state.dead() {
            return Poll::Ready(Ok(StreamResult::Dropped));
        }
        let mut buffer: Vec<CallResult> = Vec::with_capacity(8);
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
