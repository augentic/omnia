//! Per-completion session state and budget enforcement for `omnia:model`.

use std::collections::HashMap;
use std::fmt::{self, Debug};
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::anyhow;
use futures::{FutureExt as _, future};
use tokio::sync::{mpsc, oneshot};

use super::{Answer, CHECK_TOOL, DirEntry, Error, Limits, Reply, ToolCall, ToolOutcome, Workspace};
use crate::FutureResult;

const CALL_QUEUE: usize = 8;

/// Host-side capabilities for one completion, lent to backends that need them.
pub trait ToolHost: Debug + Send + Sync {
    /// Run one declared function tool through the completion's session.
    fn call_tool(&self, name: String, arguments: String) -> FutureResult<ToolOutcome>;

    /// Ask the guest whether `candidate` is the answer.
    fn check(&self, candidate: String) -> FutureResult<Result<(), String>>;

    /// Bounded workspace read via the lent capability.
    fn read(&self, path: String) -> FutureResult<Vec<u8>>;

    /// Bounded workspace listing via the lent capability.
    fn list(&self, path: String) -> FutureResult<Vec<DirEntry>>;

    /// Accumulate an edit against the session's base tree.
    fn write(&self, path: String, bytes: Vec<u8>) -> FutureResult<()>;

    /// The absolute host path of the lent workspace, when one was lent for
    /// this completion and resolved to an authorized mount.
    fn local_path(&self) -> Option<&std::path::Path> {
        None
    }
}

/// Shared state for one completion session: the calls bridge, the pending
/// calls awaiting results, the call budget, and the first typed
/// host-enforcement failure.
pub struct ToolSession {
    limits: Limits,
    allowed: Vec<String>,
    inner: Mutex<Inner>,
}

impl Debug for ToolSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolSession").field("limits", &self.limits).finish_non_exhaustive()
    }
}

struct Inner {
    calls: Option<mpsc::Sender<ToolCall>>,
    pending: HashMap<String, oneshot::Sender<ToolOutcome>>,
    remaining: u32,
    next_id: u64,
    error: Option<Error>,
}

impl Inner {
    fn record(&mut self, error: Error) -> anyhow::Error {
        let hard = anyhow!(match &error {
            Error::InvalidRequest(detail) => format!("invalid request: {detail}"),
            Error::BudgetExhausted(detail) => format!("budget exhausted: {detail}"),
            Error::ToolFailed(detail) => format!("tool failed: {detail}"),
            Error::Backend(detail) => format!("backend failure: {detail}"),
        });
        self.error.get_or_insert(error);
        hard
    }
}

struct PendingCall {
    id: String,
    calls: mpsc::Sender<ToolCall>,
    result: oneshot::Receiver<ToolOutcome>,
}

impl ToolSession {
    /// Create a fresh session with `limits` and `allowed` tool names.
    #[must_use]
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

    /// Open a completion session bound to `workspace` and return the session,
    /// its tool-call stream, and the [`ToolHost`] handed to the backend.
    #[must_use]
    pub fn open(
        limits: Limits, allowed: Vec<String>, workspace: Option<Workspace>,
    ) -> (Arc<Self>, mpsc::Receiver<ToolCall>, Arc<dyn ToolHost>) {
        let (session, calls_rx) = Self::new(limits, allowed);
        let tool_host: Arc<dyn ToolHost> =
            Arc::new(BoundToolHost::new(Arc::clone(&session), workspace));
        (session, calls_rx, tool_host)
    }

    /// Take the typed failure host enforcement recorded, if any.
    pub fn take_error(&self) -> Option<Error> {
        self.lock().error.take()
    }

    /// Await `answer` under a session-close guard and resolve the final [`Reply`].
    ///
    /// Gives precedence to any typed [`Error`] recorded by session enforcement.
    ///
    /// # Errors
    ///
    /// Returns the session's recorded [`Error`] if enforcement tripped, or the
    /// backend's failure mapped into [`Error`].
    pub async fn finish(self: Arc<Self>, answer: FutureResult<Answer>) -> Result<Reply, Error> {
        let _close = SessionClose::new(Arc::clone(&self));
        match answer.await {
            Ok(answer) => self.take_error().map_or_else(
                || {
                    Ok(Reply {
                        answer: answer.answer,
                        usage: answer.usage,
                    })
                },
                Err,
            ),
            Err(error) => {
                if let Some(session_err) = self.take_error() {
                    return Err(session_err);
                }
                Err(Error::from(error))
            }
        }
    }

    /// Run one declared function tool through the session; see [`ToolHost::call_tool`].
    pub fn call(self: Arc<Self>, name: String, arguments: String) -> FutureResult<ToolOutcome> {
        async move {
            if !self.allowed.contains(&name) {
                return Err(self.fail(Error::ToolFailed(format!(
                    "model called `{name}`, which the request does not declare as a function tool"
                ))));
            }
            let pending = self.reserve(&name, true)?;
            let output = self.exchange(pending, &name, arguments).await?;
            self.check_result(&name, output)
        }
        .boxed()
    }

    /// Ask the guest to accept `candidate` through the reserved `check` call.
    pub fn check(self: Arc<Self>, candidate: String) -> FutureResult<Result<(), String>> {
        async move {
            let pending = self.reserve(CHECK_TOOL, false)?;
            let output = self.exchange(pending, CHECK_TOOL, candidate).await?;
            if let Err(correction) = &output {
                self.cap(CHECK_TOOL, correction)?;
            }
            Ok(output.map(drop))
        }
        .boxed()
    }

    /// Deliver one tool result for call `id` to its pending waiter.
    pub fn deliver(&self, id: &str, output: ToolOutcome) {
        let sender = self.lock().pending.remove(id);
        if let Some(sender) = sender {
            drop(sender.send(output));
        } else {
            tracing::trace!(id = %id, "dropping tool result with no pending call");
        }
    }

    /// Close the calls channel and drop all pending waiters.
    pub fn shutdown(&self) {
        let mut inner = self.lock();
        inner.calls = None;
        inner.pending.clear();
    }

    /// Whether the session has closed its calls stream and has no pending calls.
    #[must_use]
    pub fn dead(&self) -> bool {
        let inner = self.lock();
        inner.calls.is_none() && inner.pending.is_empty()
    }

    async fn exchange(
        &self, pending: PendingCall, name: &str, arguments: String,
    ) -> anyhow::Result<ToolOutcome> {
        let call = ToolCall {
            id: pending.id.clone(),
            name: name.to_owned(),
            arguments,
        };
        if pending.calls.send(call).await.is_err() {
            self.lock().pending.remove(&pending.id);
            return Err(anyhow!(
                "guest dropped the calls stream before receiving tool call `{name}`"
            ));
        }
        self.wait_for_result(&pending.id, name, pending.result).await
    }

    fn reserve(&self, name: &str, budgeted: bool) -> anyhow::Result<PendingCall> {
        let mut inner = self.lock();
        let Some(calls) = inner.calls.clone() else {
            return Err(anyhow!("tool call `{name}` after the session closed its calls stream"));
        };
        if budgeted {
            if inner.remaining == 0 {
                inner.calls = None;
                return Err(inner.record(Error::BudgetExhausted(format!(
                    "tool-call budget of {} exhausted at `{name}`",
                    self.limits.max_tool_calls
                ))));
            }
            inner.remaining -= 1;
        }

        inner.next_id += 1;
        let prefix = if budgeted { "call" } else { name };
        let id = format!("{prefix}-{}", inner.next_id);
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
                Err(inner.record(Error::BudgetExhausted(format!(
                    "tool call `{name}` got no result within {:?}",
                    self.limits.tool_timeout
                ))))
            }
            Ok(Err(_closed)) => {
                Err(anyhow!("guest closed its results stream before answering tool call `{name}`"))
            }
            Ok(Ok(output)) => Ok(output),
        }
    }

    fn check_result(&self, name: &str, output: ToolOutcome) -> anyhow::Result<ToolOutcome> {
        if let Ok(text) = &output {
            self.cap(name, text)?;
        }
        Ok(output)
    }

    fn cap(&self, name: &str, text: &str) -> anyhow::Result<()> {
        if text.len() > self.limits.max_result_bytes {
            return Err(self.fail(Error::ToolFailed(format!(
                "tool `{name}` result of {} bytes exceeds the {}-byte cap",
                text.len(),
                self.limits.max_result_bytes
            ))));
        }
        Ok(())
    }

    fn fail(&self, error: Error) -> anyhow::Error {
        self.lock().record(error)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

struct SessionClose {
    state: Arc<ToolSession>,
}

impl SessionClose {
    const fn new(state: Arc<ToolSession>) -> Self {
        Self { state }
    }
}

impl Drop for SessionClose {
    fn drop(&mut self) {
        self.state.shutdown();
    }
}

struct BoundToolHost {
    session: Arc<ToolSession>,
    workspace: Option<Workspace>,
}

impl BoundToolHost {
    const fn new(session: Arc<ToolSession>, workspace: Option<Workspace>) -> Self {
        Self { session, workspace }
    }

    fn with_workspace<R: Send + 'static>(
        &self, op: &str, path: String, f: impl FnOnce(&Workspace, String) -> FutureResult<R>,
    ) -> FutureResult<R> {
        match &self.workspace {
            Some(workspace) => f(workspace, path),
            None => future::err(anyhow!("{op}(`{path}`) requires grants.workspace")).boxed(),
        }
    }
}

impl Debug for BoundToolHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoundToolHost")
            .field("workspace", &self.workspace.as_ref().map(Workspace::local_path))
            .finish_non_exhaustive()
    }
}

impl ToolHost for BoundToolHost {
    fn call_tool(&self, name: String, arguments: String) -> FutureResult<ToolOutcome> {
        Arc::clone(&self.session).call(name, arguments)
    }

    fn check(&self, candidate: String) -> FutureResult<Result<(), String>> {
        Arc::clone(&self.session).check(candidate)
    }

    fn read(&self, path: String) -> FutureResult<Vec<u8>> {
        self.with_workspace("read", path, Workspace::read)
    }

    fn list(&self, path: String) -> FutureResult<Vec<DirEntry>> {
        self.with_workspace("list", path, Workspace::list)
    }

    fn write(&self, path: String, bytes: Vec<u8>) -> FutureResult<()> {
        self.with_workspace("write", path, move |workspace, path| workspace.write(path, bytes))
    }

    fn local_path(&self) -> Option<&std::path::Path> {
        self.workspace.as_ref().map(Workspace::local_path)
    }
}
