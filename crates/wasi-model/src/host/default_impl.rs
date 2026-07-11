//! `ModelDefault` — the crate's default, deterministic (replay) backend.
//!
//! It serves a pre-recorded answer for a given prompt.

use std::collections::HashMap;
use std::fmt::Debug;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow};
use futures::FutureExt as _;
use omnia::Backend;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::instrument;

use crate::host::generated::omnia::model::completion::{Request, Tool};
use crate::host::types::{Answer, Transcript, Usage};
use crate::host::{FutureResult, ToolHost, WasiModelCtx};

/// Options used to connect the replay backend.
#[derive(Debug, Clone)]
pub struct ConnectOptions {
    /// Replay fixtures directory.
    pub replay_dir: PathBuf,
}

impl omnia::FromEnv for ConnectOptions {
    fn from_env() -> Result<Self> {
        let replay_dir = std::env::var_os("MODEL_REPLAY_DIR")
            .map_or_else(|| PathBuf::from("fixtures"), PathBuf::from);
        Ok(Self { replay_dir })
    }
}

/// Default (replay) implementation of `wasi-model`.
#[derive(Clone, Debug)]
pub struct ModelDefault {
    store: Arc<FixtureStore>,
}

impl Backend for ModelDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        Self::from_dir(options.replay_dir)
    }
}

impl WasiModelCtx for ModelDefault {
    fn complete(&self, request: Request, _tool_host: Arc<dyn ToolHost>) -> FutureResult<Answer> {
        let answer = self.replay(&request);
        async move { answer }.boxed()
    }
}

impl ModelDefault {
    /// Load replay fixtures from `path`.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory cannot be read or a fixture is malformed.
    pub fn from_dir(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let store = FixtureStore::try_from(path)?;
        tracing::debug!(
            dir = %path.display(),
            fixtures = store.len(),
            "initialized replay backend"
        );
        Ok(Self {
            store: Arc::new(store),
        })
    }

    /// Look up the recorded answer for `request`.
    ///
    /// # Errors
    ///
    /// Returns an error when canonicalization fails or no fixture matches.
    pub fn replay(&self, request: &Request) -> Result<Answer> {
        self.store.answer(request)
    }
}

/// In-memory replay index keyed by canonical prompt JSON.
#[derive(Debug, Default)]
struct FixtureStore {
    answers: HashMap<String, Answer>,
}

impl TryFrom<&Path> for FixtureStore {
    type Error = anyhow::Error;

    fn try_from(path: &Path) -> Result<Self> {
        let mut store = Self::default();

        if !path.exists() {
            return Ok(store);
        }

        for entry in
            fs::read_dir(path).with_context(|| format!("reading replay dir {}", path.display()))?
        {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let bytes =
                fs::read(&path).with_context(|| format!("reading fixture {}", path.display()))?;
            let fixture: Fixture = serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing fixture {}", path.display()))?;
            store.insert(fixture)?;
        }

        Ok(store)
    }
}

impl FixtureStore {
    fn answer(&self, request: &Request) -> Result<Answer> {
        let key_json = &reduced_value(request);
        let key = serde_json::to_string(key_json)?;

        self.answers.get(&key).cloned().ok_or_else(|| anyhow!("no replay fixture for request"))
    }

    #[must_use]
    fn len(&self) -> usize {
        self.answers.len()
    }

    fn insert(&mut self, fixture: Fixture) -> Result<()> {
        let key = serde_json::to_string(&fixture.key_request)?;

        self.answers.insert(
            key,
            Answer {
                value: fixture.answer,
                usage: fixture.usage,
                transcript: fixture.transcript,
            },
        );
        Ok(())
    }
}

// A `request -> answer` row, the unit of replay.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Fixture {
    key_request: Value,
    answer: Value,
    #[serde(default)]
    usage: Option<Usage>,
    #[serde(default)]
    transcript: Option<Transcript>,
}

fn reduced_value(request: &Request) -> Value {
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
        "format": request.format.replay_value(),
        "tools": request.tools.iter().map(Tool::replay_value).collect::<Vec<_>>(),
        "grants": {
            "references": request.grants.references,
            "verify": request.grants.verify,
        },
    })
}
