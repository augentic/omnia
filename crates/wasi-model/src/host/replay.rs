//! Replay fixture index for `ModelDefault` (§5.4).

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context as _, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::generated::augentic::model::completion::{Prompt, ResponseFormatKind, ToolChoice};
use super::types::{Answer, PreparedPrompt, Transcript};

/// In-memory replay index keyed by canonical prompt JSON.
#[derive(Debug, Default)]
pub struct FixtureStore {
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
    /// The replayed answer for `request`.
    ///
    /// # Errors
    ///
    /// Returns an error when no equivalent fixture is indexed.
    pub fn answer_for(&self, request: &PreparedPrompt) -> Result<Answer> {
        let prompt = &request.prompt;
        let workspace_lent = request.workspace_lent;
        let key_json = &canonicalize(&reduced_value(prompt, workspace_lent));
        let key = serde_json::to_string(key_json)?;

        self.answers.get(&key).cloned().ok_or_else(|| anyhow!("no replay fixture for prompt"))
    }

    /// The number of indexed fixtures.
    #[must_use]
    pub fn len(&self) -> usize {
        self.answers.len()
    }

    fn insert(&mut self, fixture: Fixture) {
        let key_json = &canonicalize(&fixture.key_prompt);
        let key = serde_json::to_string(key_json).unwrap_or_default();

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

fn reduced_value(prompt: &Prompt, workspace_lent: bool) -> Value {
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
            "workspace_lent": workspace_lent,
            "verify": prompt.grants.verify,
        },
    })
}

// Sort object keys so serialization is canonical.
fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            let mut sorted = serde_json::Map::with_capacity(entries.len());
            for (key, val) in entries {
                sorted.insert(key.clone(), canonicalize(val));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize).collect()),
        scalar => scalar.clone(),
    }
}
