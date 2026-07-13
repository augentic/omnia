//! Fixture replay and recording for the `wasi-model` boundary.
//!
//! Owns the canonical request key, the fixture row format, and both faces of
//! replay: the host-side [`ReplayBackend`] (a `WasiModelCtx` for seam tests
//! and example runtimes) and the guest-side [`Replay`] (a `Model` for native
//! tests of model-consuming core logic). [`RecorderBackend`] and [`Recorder`]
//! are the matching decorators that regenerate fixture rows from a live run.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context as _, Result, anyhow};
use futures::FutureExt as _;
use omnia_guest::model::{
    Effort, Error, Format, Message, Model, Reply, Request, Role, Tool, Usage,
};
use omnia_wasi_model::{Answer, FutureResult, ToolHost, Transcript, WasiModelCtx};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// A `request -> answer` row, the unit of replay.
///
/// Serialized one-per-file as `<name>.json` inside a fixture directory;
/// [`ReplayBackend::from_dir`] loads every `.json` in the directory. Recording
/// writes rows through the same [`key_request`] derivation replay matches
/// against.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fixture {
    /// The canonical request key ([`key_request`]) this row answers.
    pub key_request: Value,
    /// The recorded answer value.
    pub answer: Value,
    /// Token accounting to replay alongside the answer.
    #[serde(default)]
    pub usage: Option<omnia_wasi_model::Usage>,
    /// Tool-call transcript to replay alongside the answer.
    #[serde(default)]
    pub transcript: Option<Transcript>,
}

impl Fixture {
    /// A fixture row answering `request` with `answer`.
    #[must_use]
    pub fn new(request: &omnia_wasi_model::Request, answer: Value) -> Self {
        Self {
            key_request: key_request(request),
            answer,
            usage: None,
            transcript: None,
        }
    }
}

/// The canonical JSON a request reduces to for fixture keying — the
/// prompt-affecting fields only (the lent workspace descriptor and any
/// backend-side concerns are excluded).
#[must_use]
pub fn key_request(request: &omnia_wasi_model::Request) -> Value {
    json!({
        "model": request.model,
        "system": request.system,
        "messages": request.messages.iter().map(|message| json!({
            "role": message.role.to_string(),
            "content": message.content,
        })).collect::<Vec<_>>(),
        "generation": request.generation.as_ref().map(|generation| json!({
            "temperature": generation.temperature,
            "top_p": generation.top_p,
            "max_tokens": generation.max_tokens,
            "stop": generation.stop,
            "seed": generation.seed,
            "effort": generation.effort.map(|effort| effort.to_string()),
        })),
        "format": format_value(&request.format),
        "tools": request.tools.iter().map(tool_value).collect::<Vec<_>>(),
        "grants": {
            "references": request.grants.references,
            "verify": request.grants.verify,
        },
    })
}

fn format_value(format: &omnia_wasi_model::Format) -> Value {
    match format {
        omnia_wasi_model::Format::Text => json!({ "kind": "text" }),
        omnia_wasi_model::Format::Json => json!({ "kind": "json" }),
        omnia_wasi_model::Format::Schema(spec) => json!({
            "kind": "schema",
            "schema": {
                "name": spec.name,
                "schema": spec.schema,
            },
        }),
    }
}

fn tool_value(tool: &omnia_wasi_model::Tool) -> Value {
    match tool {
        omnia_wasi_model::Tool::Function(function) => json!({
            "function": {
                "name": function.name,
                "description": function.description,
                "parameters": function.parameters,
            },
        }),
        omnia_wasi_model::Tool::Mcp(mcp) => json!({
            "mcp": {
                "name": mcp.name,
                "tools": mcp.tools,
                "url": mcp.url,
            },
        }),
    }
}

// In-memory replay index keyed by canonical prompt JSON.
#[derive(Debug, Default)]
struct FixtureStore {
    answers: HashMap<String, Answer>,
}

impl FixtureStore {
    fn answer(&self, request: &omnia_wasi_model::Request) -> Result<Answer> {
        let key = serde_json::to_string(&key_request(request))?;

        self.answers.get(&key).cloned().ok_or_else(|| anyhow!("no replay fixture for request"))
    }

    fn insert(&mut self, fixture: Fixture) -> Result<()> {
        let key = serde_json::to_string(&fixture.key_request)?;

        if self
            .answers
            .insert(
                key.clone(),
                Answer {
                    value: fixture.answer,
                    usage: fixture.usage,
                    transcript: fixture.transcript,
                },
            )
            .is_some()
        {
            return Err(anyhow!("duplicate replay fixture for key {key}"));
        }
        Ok(())
    }
}

/// Host-side fixture replay: a `WasiModelCtx` serving pre-recorded answers.
///
/// Replay never runs tools; it short-circuits straight to the recorded
/// answer, and a request with no matching fixture fails loud. Clones share
/// the immutable fixture index.
#[derive(Clone, Debug)]
pub struct ReplayBackend {
    store: Arc<FixtureStore>,
}

impl ReplayBackend {
    /// Build a replay backend over in-memory fixture rows.
    ///
    /// # Errors
    ///
    /// Returns an error when two fixtures share a canonical key.
    pub fn new(fixtures: impl IntoIterator<Item = Fixture>) -> Result<Self> {
        let mut store = FixtureStore::default();
        for fixture in fixtures {
            store.insert(fixture)?;
        }
        Ok(Self {
            store: Arc::new(store),
        })
    }

    /// Load replay fixtures from every `.json` file in `path`.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory cannot be read, a fixture is
    /// malformed, or two fixtures share a canonical key.
    pub fn from_dir(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let mut store = FixtureStore::default();

        for entry in std::fs::read_dir(path)
            .with_context(|| format!("reading replay dir {}", path.display()))?
        {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let bytes = std::fs::read(&path)
                .with_context(|| format!("reading fixture {}", path.display()))?;
            let fixture: Fixture = serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing fixture {}", path.display()))?;
            store.insert(fixture).with_context(|| format!("loading {}", path.display()))?;
        }

        Ok(Self {
            store: Arc::new(store),
        })
    }

    /// Look up the recorded answer for `request`.
    ///
    /// # Errors
    ///
    /// Returns an error when canonicalization fails or no fixture matches.
    pub fn replay(&self, request: &omnia_wasi_model::Request) -> Result<Answer> {
        self.store.answer(request)
    }
}

impl WasiModelCtx for ReplayBackend {
    fn complete(
        &self, request: omnia_wasi_model::Request, _tool_host: Arc<dyn ToolHost>,
    ) -> FutureResult<Answer> {
        let answer = self.replay(&request);
        async move { answer }.boxed()
    }
}

/// A `WasiModelCtx` decorator that records every completion as a replay
/// fixture row before returning it.
///
/// Rows are keyed on the request actually received across the WIT boundary —
/// what a [`ReplayBackend`] later serves.
///
/// Rows are written as `NNN.json` in call order; the caller owns the
/// directory's lifecycle (clearing stale rows before a regeneration run).
/// Clones share the sequence counter, so a cloned recorder keeps appending
/// rather than overwriting earlier rows.
#[derive(Clone, Debug)]
pub struct RecorderBackend<B> {
    backend: B,
    dir: PathBuf,
    sequence: Arc<AtomicUsize>,
}

impl<B> RecorderBackend<B> {
    /// Wrap `backend`, recording each completion into `dir`.
    pub fn new(backend: B, dir: impl Into<PathBuf>) -> Self {
        Self {
            backend,
            dir: dir.into(),
            sequence: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl<B> WasiModelCtx for RecorderBackend<B>
where
    B: WasiModelCtx,
{
    fn complete(
        &self, request: omnia_wasi_model::Request, tool_host: Arc<dyn ToolHost>,
    ) -> FutureResult<Answer> {
        let key = key_request(&request);
        let inner = self.backend.complete(request, tool_host);
        let dir = self.dir.clone();
        let sequence = Arc::clone(&self.sequence);
        async move {
            let answer = inner.await?;
            let fixture = Fixture {
                key_request: key,
                answer: answer.value.clone(),
                usage: answer.usage,
                transcript: answer.transcript.clone(),
            };
            let index = sequence.fetch_add(1, Ordering::SeqCst);
            write_row(&dir, index, &fixture).context("recording replay fixture")?;
            Ok(answer)
        }
        .boxed()
    }
}

/// Fixture-backed guest-side model, applying the same answer validation and
/// guest-visible projection as the host boundary.
#[derive(Clone, Debug)]
pub struct Replay {
    backend: ReplayBackend,
}

impl Replay {
    /// Build a replay model over in-memory fixture rows.
    ///
    /// # Errors
    ///
    /// Returns an error when two fixtures share a canonical key.
    pub fn new(fixtures: impl IntoIterator<Item = Fixture>) -> Result<Self> {
        Ok(Self {
            backend: ReplayBackend::new(fixtures)?,
        })
    }

    /// Load replay fixtures from `path`.
    ///
    /// # Errors
    ///
    /// Returns an error when the fixture directory cannot be read or a fixture is malformed.
    pub fn from_dir(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            backend: ReplayBackend::from_dir(path)?,
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
    dir: PathBuf,
    sequence: Arc<AtomicUsize>,
}

impl<B> Recorder<B> {
    /// Wrap `backend`, recording each completion into `dir`.
    pub fn new(backend: B, dir: impl Into<PathBuf>) -> Self {
        Self {
            backend,
            dir: dir.into(),
            sequence: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl<B> Model for Recorder<B>
where
    B: Model,
{
    async fn create(&self, request: Request) -> Result<Reply, Error> {
        let reply = self.backend.create(request.clone()).await?;
        let index = self.sequence.fetch_add(1, Ordering::SeqCst);
        record(&self.dir, index, request, &reply)
            .map_err(|error| Error::Backend(format!("recording replay fixture: {error}")))?;
        Ok(reply)
    }
}

// Record one guest `request -> reply` completion as a fixture row.
fn record(dir: &Path, index: usize, request: Request, reply: &Reply) -> Result<()> {
    let wire = wire_request(request);
    let value = wire
        .format
        .parse(&reply.answer)
        .map_err(|reason| anyhow!("answer does not match the request format: {reason}"))?;
    let mut fixture = Fixture::new(&wire, value);
    fixture.usage = reply.usage.map(|usage| omnia_wasi_model::Usage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        reasoning_tokens: usage.reasoning_tokens,
    });
    write_row(dir, index, &fixture)
}

// Write one fixture row as `<dir>/NNN.json`.
fn write_row(dir: &Path, index: usize, fixture: &Fixture) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(format!("{index:03}.json"));
    std::fs::write(&path, serde_json::to_vec_pretty(fixture)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn replay(backend: &ReplayBackend, request: Request) -> Result<Reply, Error> {
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
