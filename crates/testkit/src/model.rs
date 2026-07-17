//! Lightweight model doubles for testing model consumers on both faces of
//! the `wasi-model` boundary.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::anyhow;
use futures::FutureExt as _;
use omnia_guest::model::{Error, McpGrant, Model, Reply, Request, Tool, Usage};
use omnia_wasi_model::{Answer, FutureResult, ToolHost, WasiModelCtx};
use serde_json::Value;

/// A FIFO model script of successes and typed failures.
///
/// One queue serves both faces of the boundary: the guest-side [`Model`] for
/// native tests of model-consuming logic, and the host-side [`WasiModelCtx`]
/// for seam tests and example runtimes.
#[derive(Clone, Debug)]
pub struct Scripted {
    responses: Arc<Mutex<VecDeque<Result<Answer, Error>>>>,
}

impl Scripted {
    /// Build a script from ordered completion results.
    #[must_use]
    pub fn new(responses: impl IntoIterator<Item = Result<Reply, Error>>) -> Self {
        Self::results(responses.into_iter().map(|result| result.map(reply_answer)))
    }

    /// Build a script from ordered host answers.
    #[must_use]
    pub fn values(answers: impl IntoIterator<Item = Answer>) -> Self {
        Self::results(answers.into_iter().map(Ok))
    }

    /// Build a one-answer script answering with a JSON value.
    #[must_use]
    pub fn json(value: Value) -> Self {
        Self::values([Answer {
            value,
            usage: None,
            transcript: None,
        }])
    }

    /// Build a one-answer success script.
    #[must_use]
    pub fn reply(answer: impl Into<String>) -> Self {
        Self::answers([answer])
    }

    /// Build a success script from ordered answer strings.
    #[must_use]
    pub fn answers<S>(answers: impl IntoIterator<Item = S>) -> Self
    where
        S: Into<String>,
    {
        Self::new(answers.into_iter().map(|answer| {
            Ok(Reply {
                answer: answer.into(),
                usage: None,
            })
        }))
    }

    /// Build a success script from complete replies.
    #[must_use]
    pub fn replies(replies: impl IntoIterator<Item = Reply>) -> Self {
        Self::new(replies.into_iter().map(Ok))
    }

    /// Assert that every scripted result was consumed.
    ///
    /// # Panics
    ///
    /// Panics when one or more results remain.
    pub fn assert_exhausted(&self) {
        let remaining = lock(&self.responses).len();
        assert_eq!(remaining, 0, "script has {remaining} unconsumed result(s)");
    }

    fn results(results: impl IntoIterator<Item = Result<Answer, Error>>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(results.into_iter().collect())),
        }
    }

    fn pop(&self) -> Option<Result<Answer, Error>> {
        lock(&self.responses).pop_front()
    }
}

impl Model for Scripted {
    fn create(&self, _request: Request) -> impl Future<Output = Result<Reply, Error>> + Send {
        let response = self.pop().map_or_else(
            || Err(Error::Backend("model script exhausted".to_owned())),
            |result| result.map(answer_reply),
        );
        std::future::ready(response)
    }
}

impl WasiModelCtx for Scripted {
    fn complete(
        &self, _request: omnia_wasi_model::Request, _tool_host: Arc<dyn ToolHost>,
    ) -> FutureResult<Answer> {
        let response = self.pop().map_or_else(
            || Err(anyhow!("model script exhausted")),
            |result| result.map_err(anyhow::Error::new),
        );
        async move { response }.boxed()
    }
}

// Lift a guest reply into the host answer the shared queue stores.
fn reply_answer(reply: Reply) -> Answer {
    Answer {
        value: Value::String(reply.answer),
        usage: reply.usage.map(|usage| omnia_wasi_model::Usage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            reasoning_tokens: usage.reasoning_tokens,
        }),
        transcript: None,
    }
}

// Project a host answer to the guest-visible reply: strings pass through,
// any other JSON value is serialized.
fn answer_reply(answer: Answer) -> Reply {
    Reply {
        answer: match answer.value {
            Value::String(text) => text,
            value => value.to_string(),
        },
        usage: answer.usage.map(|usage| Usage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            reasoning_tokens: usage.reasoning_tokens,
        }),
    }
}

/// A model decorator that records requests before delegating them, so
/// tests can assert on prompt assembly, formats, and tool grants.
#[derive(Clone, Debug)]
pub struct Harness<B> {
    backend: B,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl<B> Harness<B> {
    /// Wrap `backend` with request recording.
    #[must_use]
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Return a thread-safe snapshot of every request in call order.
    #[must_use]
    pub fn requests(&self) -> Vec<Request> {
        lock(&self.requests).clone()
    }
}

impl Harness<Scripted> {
    /// Build a recorded harness from ordered completion results.
    #[must_use]
    pub fn scripted(responses: impl IntoIterator<Item = Result<Reply, Error>>) -> Self {
        Self::new(Scripted::new(responses))
    }

    /// Build a recorded harness from ordered answer strings.
    #[must_use]
    pub fn answering<S>(answers: impl IntoIterator<Item = S>) -> Self
    where
        S: Into<String>,
    {
        Self::new(Scripted::answers(answers))
    }

    /// Assert that every scripted result was consumed.
    ///
    /// # Panics
    ///
    /// Panics when one or more results remain.
    pub fn assert_exhausted(&self) {
        self.backend.assert_exhausted();
    }
}

impl<B> Model for Harness<B>
where
    B: Model,
{
    fn create(&self, request: Request) -> impl Future<Output = Result<Reply, Error>> + Send {
        lock(&self.requests).push(request.clone());
        self.backend.create(request)
    }
}

/// Return the MCP grants carried by a request.
#[must_use]
pub fn mcp_grants(request: &Request) -> Vec<&McpGrant> {
    request
        .tools
        .iter()
        .filter_map(|tool| match tool {
            Tool::Mcp(grant) => Some(grant),
            Tool::Function(_) => None,
        })
        .collect()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
