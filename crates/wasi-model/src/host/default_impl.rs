//! `ModelDefault` — the crate's default, deterministic (replay) backend (§5.4).
//!
//! It serves a pre-recorded answer for a given prompt.

use std::collections::HashMap;
use std::fmt::Debug;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow};
use futures::FutureExt as _;
use omnia::Backend;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::instrument;

use crate::host::generated::augentic::model::completion::{Prompt, ResponseFormatKind, ToolChoice};
use crate::host::types::{Answer, PreparedPrompt, Transcript};
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
        let store = FixtureStore::try_from(&options.replay_dir)?;
        tracing::debug!(
            dir = %options.replay_dir.display(),
            fixtures = store.len(),
            "initialized replay backend"
        );
        Ok(Self {
            store: Arc::new(store),
        })
    }
}

impl WasiModelCtx for ModelDefault {
    fn complete(
        &self, request: PreparedPrompt, _tool_host: Arc<dyn ToolHost>,
    ) -> FutureResult<Answer> {
        let answer = self.store.answer(&request);
        async move { answer }.boxed()
    }
}

/// In-memory replay index keyed by canonical prompt JSON.
#[derive(Debug, Default)]
struct FixtureStore {
    answers: HashMap<String, Answer>,
}

impl TryFrom<&PathBuf> for FixtureStore {
    type Error = anyhow::Error;

    fn try_from(path: &PathBuf) -> Result<Self> {
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
            store.insert(fixture);
        }

        Ok(store)
    }
}

impl FixtureStore {
    fn answer(&self, request: &PreparedPrompt) -> Result<Answer> {
        let key_json = &reduced_value(&request.prompt);
        let key = serde_json::to_string(key_json)?;

        self.answers.get(&key).cloned().ok_or_else(|| anyhow!("no replay fixture for prompt"))
    }

    #[must_use]
    fn len(&self) -> usize {
        self.answers.len()
    }

    fn insert(&mut self, fixture: Fixture) {
        let key = serde_json::to_string(&fixture.key_prompt).unwrap_or_default();

        self.answers.insert(
            key,
            Answer {
                value: fixture.answer,
                transcript: fixture.transcript,
            },
        );
    }
}

// A `prompt -> answer` row, the unit of replay (§5.4).
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Fixture {
    key_prompt: Value,
    answer: Value,
    #[serde(default)]
    transcript: Option<Transcript>,
}

fn reduced_value(prompt: &Prompt) -> Value {
    json!({
        "model": prompt.model,
        "system": prompt.system,
        "messages": prompt.messages.iter().map(|message| json!({
            "role": message.role,
            "content": message.content,
        })).collect::<Vec<_>>(),
        "sections": prompt.sections.as_ref().map(|sections| json!({
            "role": sections.role,
            "task": sections.task,
            "context": sections.context,
            "constraints": sections.constraints,
            "examples": sections.examples.iter().map(|example| json!({
                "input": example.input,
                "output": example.output,
            })).collect::<Vec<_>>(),
            "variables": sections.variables.iter().map(|variable| json!({
                "name": variable.name,
                "value": variable.value,
            })).collect::<Vec<_>>(),
        })),
        "generation": prompt.generation.as_ref().map(|generation| json!({
            "temperature": generation.temperature,
            "top_p": generation.top_p,
            "max_tokens": generation.max_tokens,
            "stop": generation.stop,
        })),
        "response_format": {
            "kind": match prompt.response_format.kind {
                ResponseFormatKind::Text => "text",
                ResponseFormatKind::JsonObject => "json-object",
                ResponseFormatKind::JsonSchema => "json-schema",
            },
            "json_schema": prompt.response_format.json_schema.as_ref().map(|spec| json!({
                "name": spec.name,
                "schema": spec.schema,
                "strict": spec.strict,
            })),
        },
        "tools": prompt.tools.iter().map(|tool| json!({
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.parameters,
        })).collect::<Vec<_>>(),
        "tool_choice": prompt.tool_choice.as_ref().map(|choice| match choice {
            ToolChoice::Auto => json!("auto"),
            ToolChoice::None => json!("none"),
            ToolChoice::Required => json!("required"),
            ToolChoice::Named(name) => json!({ "named": name }),
        }),
        "grants": {
            "references": prompt.grants.references,
            "verify": prompt.grants.verify,
        },
    })
}
