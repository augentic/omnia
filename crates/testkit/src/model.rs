//! Lightweight model doubles and fixture replay for testing model consumers.

mod replay;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use omnia_guest::model::{Error, McpGrant, Model, Reply, Request, Tool};

pub use self::replay::{Fixture, Recorder, RecorderBackend, Replay, ReplayBackend, key_request};

/// A model decorator that records requests before delegating them.
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

/// A FIFO model script containing successes and typed failures.
#[derive(Clone, Debug)]
pub struct Scripted {
    responses: Arc<Mutex<VecDeque<Result<Reply, Error>>>>,
}

impl Scripted {
    /// Build a script from ordered completion results.
    #[must_use]
    pub fn new(responses: impl IntoIterator<Item = Result<Reply, Error>>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into_iter().collect())),
        }
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
}

impl Model for Scripted {
    fn create(&self, _request: Request) -> impl Future<Output = Result<Reply, Error>> + Send {
        let response = lock(&self.responses)
            .pop_front()
            .unwrap_or_else(|| Err(Error::Backend("model script exhausted".to_owned())));
        std::future::ready(response)
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
