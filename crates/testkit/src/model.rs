//! Lightweight model doubles for testing guest core logic.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use omnia_guest::model::{Effort, Format, Message, Role, Usage};
use omnia_guest::model::{Error, McpGrant, Model, Reply, Request, Tool};

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

/// Fixture-backed model using `omnia-wasi-model` replay semantics.
#[derive(Clone, Debug)]
pub struct Replay {
    backend: omnia_wasi_model::ModelDefault,
}

impl Replay {
    /// Load replay fixtures from `path`.
    ///
    /// # Errors
    ///
    /// Returns an error when the fixture directory cannot be read or a fixture is malformed.
    pub fn from_dir(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        Ok(Self {
            backend: omnia_wasi_model::ModelDefault::from_dir(path)?,
        })
    }
}

impl Model for Replay {
    fn create(&self, request: Request) -> impl Future<Output = Result<Reply, Error>> + Send {
        let result = replay(&self.backend, request);
        std::future::ready(result)
    }
}

/// A model decorator that records every completion as a replay fixture
/// row before returning it, so a scripted or live run regenerates the
/// fixture directory a [`Replay`] later serves.
///
/// Rows are written as `NNN.json` in call order; the caller owns the
/// directory's lifecycle (clearing stale rows before a regeneration run).
/// Clones share the sequence counter, so a cloned recorder keeps
/// appending rather than overwriting earlier rows.
#[derive(Clone, Debug)]
pub struct Recorder<B> {
    backend: B,
    dir: std::path::PathBuf,
    sequence: Arc<std::sync::atomic::AtomicUsize>,
}

impl<B> Recorder<B> {
    /// Wrap `backend`, recording each completion into `dir`.
    pub fn new(backend: B, dir: impl Into<std::path::PathBuf>) -> Self {
        Self {
            backend,
            dir: dir.into(),
            sequence: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }
}

impl<B> Model for Recorder<B>
where
    B: Model,
{
    async fn create(&self, request: Request) -> Result<Reply, Error> {
        let reply = self.backend.create(request.clone()).await?;
        let index = self.sequence.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        record(&self.dir, index, request, &reply)
            .map_err(|error| Error::Backend(format!("recording replay fixture: {error}")))?;
        Ok(reply)
    }
}

/// Write one `request -> reply` fixture row as `<dir>/NNN.json`.
fn record(
    dir: &std::path::Path, index: usize, request: Request, reply: &Reply,
) -> anyhow::Result<()> {
    use anyhow::Context as _;

    let wire = wire_request(request);
    let value = wire
        .format
        .parse(&reply.answer)
        .map_err(|reason| anyhow::anyhow!("answer does not match the request format: {reason}"))?;
    let mut fixture = omnia_wasi_model::Fixture::new(&wire, value);
    fixture.usage = reply.usage.map(|usage| omnia_wasi_model::Usage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        reasoning_tokens: usage.reasoning_tokens,
    });
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(format!("{index:03}.json"));
    std::fs::write(&path, serde_json::to_vec_pretty(&fixture)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn replay(backend: &omnia_wasi_model::ModelDefault, request: Request) -> Result<Reply, Error> {
    let wire = wire_request(request);
    omnia_wasi_model::validate_request(&wire).map_err(wire_error)?;
    let answer = backend.replay(&wire).map_err(|error| Error::Backend(error.to_string()))?;
    let reply = answer.project(&wire.format).map_err(wire_error)?;
    Ok(Reply {
        answer: reply.answer,
        usage: reply.usage.map(|usage| Usage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            reasoning_tokens: usage.reasoning_tokens,
        }),
    })
}

fn wire_request(request: Request) -> omnia_wasi_model::Request {
    omnia_wasi_model::Request {
        model: request.model,
        system: request.system,
        messages: request.messages.into_iter().map(wire_message).collect(),
        generation: request.generation.map(|generation| omnia_wasi_model::Generation {
            temperature: generation.temperature,
            top_p: generation.top_p,
            max_tokens: generation.max_tokens,
            stop: generation.stop,
            seed: generation.seed,
            effort: generation.effort.map(wire_effort),
        }),
        format: match request.format {
            Format::Text => omnia_wasi_model::Format::Text,
            Format::Json => omnia_wasi_model::Format::Json,
            Format::Schema(schema) => omnia_wasi_model::Format::Schema(omnia_wasi_model::Schema {
                name: schema.name,
                schema: schema.schema,
            }),
        },
        tools: request
            .tools
            .into_iter()
            .map(|tool| match tool {
                Tool::Function(function) => {
                    omnia_wasi_model::Tool::Function(omnia_wasi_model::Function {
                        name: function.name,
                        description: function.description,
                        parameters: function.parameters,
                    })
                }
                Tool::Mcp(mcp) => omnia_wasi_model::Tool::Mcp(omnia_wasi_model::Mcp {
                    name: mcp.name,
                    tools: mcp.tools,
                    url: mcp.url,
                }),
            })
            .collect(),
        grants: omnia_wasi_model::Grants {
            references: request.references,
            workspace: None,
            verify: request.verify,
        },
    }
}

fn wire_message(message: Message) -> omnia_wasi_model::Message {
    omnia_wasi_model::Message {
        role: match message.role {
            Role::System => omnia_wasi_model::Role::System,
            Role::User => omnia_wasi_model::Role::User,
            Role::Assistant => omnia_wasi_model::Role::Assistant,
        },
        content: message.content,
    }
}

const fn wire_effort(effort: Effort) -> omnia_wasi_model::Effort {
    match effort {
        Effort::Minimal => omnia_wasi_model::Effort::Minimal,
        Effort::Low => omnia_wasi_model::Effort::Low,
        Effort::Medium => omnia_wasi_model::Effort::Medium,
        Effort::High => omnia_wasi_model::Effort::High,
    }
}

fn wire_error(error: omnia_wasi_model::Error) -> Error {
    match error {
        omnia_wasi_model::Error::InvalidRequest(detail) => Error::InvalidRequest(detail),
        omnia_wasi_model::Error::InvalidAnswer(detail) => Error::InvalidAnswer(detail),
        omnia_wasi_model::Error::BudgetExhausted(detail) => Error::BudgetExhausted(detail),
        omnia_wasi_model::Error::ToolFailed(detail) => Error::ToolFailed(detail),
        omnia_wasi_model::Error::Backend(detail) => Error::Backend(detail),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
