//! Record and replay support for testing.

use std::collections::HashMap;
use std::fs;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use futures::FutureExt as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::generated::augentic::model::completion::{Prompt, ResponseFormatKind, ToolChoice};
use super::types::{BackendAnswer, PreparedPrompt, Transcript};
use super::{FutureResult, ToolHost, WasiModelCtx};

// The canonical `(key_prompt, lookup_key)` pair for record and replay.
pub fn replay_key(prompt: &Prompt, workspace_lent: bool) -> (Value, String) {
    let value = canonicalize(&reduced_value(prompt, workspace_lent));
    let string = serde_json::to_string(&value).unwrap_or_default();
    (value, string)
}

// Write a fixture into `dir`, returning its path.
pub fn write_fixture(dir: &Path, key_prompt: Value, answer: &BackendAnswer) -> Result<PathBuf> {
    fs::create_dir_all(dir).with_context(|| format!("creating replay dir {}", dir.display()))?;
    let mut hasher = DefaultHasher::new();
    serde_json::to_string(&key_prompt).unwrap_or_default().hash(&mut hasher);
    let fixture = Fixture {
        key_prompt,
        answer: answer.value.clone(),
        transcript: answer.transcript.clone(),
    };
    let path = dir.join(format!("{:016x}.json", hasher.finish()));
    let bytes = serde_json::to_vec_pretty(&fixture).context("serializing fixture")?;
    fs::write(&path, bytes).with_context(|| format!("writing fixture {}", path.display()))?;
    Ok(path)
}

// An in-memory index of fixtures loaded from a directory, keyed by canonical
// prompt. Built once at backend `connect`.
#[derive(Debug, Default)]
pub struct FixtureStore {
    answers: HashMap<String, BackendAnswer>,
}

impl FixtureStore {
    /// Load every `*.json` fixture in `dir` (a missing dir is an empty store).
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be read or a fixture is malformed.
    pub fn load(dir: &Path) -> Result<Self> {
        let mut answers = HashMap::new();
        if dir.exists() {
            for entry in fs::read_dir(dir)
                .with_context(|| format!("reading replay dir {}", dir.display()))?
            {
                let path = entry?.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let bytes = fs::read(&path)
                    .with_context(|| format!("reading fixture {}", path.display()))?;
                let fixture: Fixture = serde_json::from_slice(&bytes)
                    .with_context(|| format!("parsing fixture {}", path.display()))?;
                let key =
                    serde_json::to_string(&canonicalize(&fixture.key_prompt)).unwrap_or_default();
                answers.insert(
                    key,
                    BackendAnswer {
                        value: fixture.answer,
                        transcript: fixture.transcript,
                    },
                );
            }
        }
        Ok(Self { answers })
    }

    // The replayed answer for an equivalent prompt, if one was recorded.
    #[must_use]
    pub fn get(&self, prompt: &Prompt, workspace_lent: bool) -> Option<BackendAnswer> {
        let (_, key) = replay_key(prompt, workspace_lent);
        self.answers.get(&key).cloned()
    }

    // The number of loaded fixtures.
    #[must_use]
    pub fn len(&self) -> usize {
        self.answers.len()
    }
}

// A `(prompt + transcript) -> answer` row, the unit of replay (§5.4).
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Fixture {
    key_prompt: Value,
    answer: Value,
    #[serde(default)]
    transcript: Option<Transcript>,
}

// Reduce the generated prompt to its output-determining fields.
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

// Sort object keys so serialization is canonical
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

/// A recording `WasiModelCtx` that wraps another backend.
///
/// On every `complete`, it persists the `(prompt, transcript) -> answer` row the
/// inner backend produces (§3.4); the recorded fixture is what
/// [`ModelDefault`](super::ModelDefault) replays.
#[derive(Debug)]
pub struct Recording<C: WasiModelCtx> {
    inner: C,
    dir: PathBuf,
}

impl<C: WasiModelCtx> Recording<C> {
    /// Wrap `inner`, writing fixtures into `dir`.
    pub fn new(inner: C, dir: impl Into<PathBuf>) -> Self {
        Self {
            inner,
            dir: dir.into(),
        }
    }
}

impl<C: WasiModelCtx> WasiModelCtx for Recording<C> {
    fn complete(
        &self, request: PreparedPrompt, tool_host: Arc<dyn ToolHost>,
    ) -> FutureResult<BackendAnswer> {
        // The generated prompt is not `Clone`, so reduce it to the on-disk key
        // value here, before the request is moved into the inner backend.
        let (key_prompt, _) = replay_key(&request.prompt, request.workspace_lent);
        let inner = self.inner.complete(request, tool_host);
        let dir = self.dir.clone();
        async move {
            let answer = inner.await?;
            // Recording is best-effort: a write failure is logged, not fatal —
            // it must never break a live completion.
            if let Err(error) = write_fixture(&dir, key_prompt, &answer) {
                tracing::warn!(%error, "failed to write replay fixture");
            }
            Ok(answer)
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::generated::augentic::model::completion::{
        MetadataEntry, ResponseFormat, Sections, ToolGrants,
    };
    use super::{Prompt, ResponseFormatKind, canonicalize, replay_key};

    fn prompt() -> Prompt {
        Prompt {
            model: Some("any".to_owned()),
            system: None,
            messages: vec![],
            sections: Some(Sections {
                role: None,
                task: "do it".to_owned(),
                context: None,
                constraints: vec![],
                examples: vec![],
                variables: vec![],
            }),
            generation: None,
            response_format: ResponseFormat {
                kind: ResponseFormatKind::JsonObject,
                json_schema: None,
            },
            tools: vec![],
            tool_choice: None,
            metadata: vec![],
            grants: ToolGrants {
                references: None,
                workspace: None,
                verify: vec![],
            },
        }
    }

    #[test]
    fn canonicalize_sorts_object_keys_recursively() {
        let canonical = serde_json::to_string(&canonicalize(&json!({
            "b": 1,
            "a": { "z": 2, "y": 3 },
        })))
        .unwrap();
        assert_eq!(canonical, r#"{"a":{"y":3,"z":2},"b":1}"#);
    }

    #[test]
    fn key_ignores_metadata_only() {
        let base = prompt();
        let mut with_metadata = prompt();
        with_metadata.metadata = vec![MetadataEntry {
            key: "trace".to_owned(),
            value: "abc".to_owned(),
        }];
        // Tracing metadata must not change the replay key (§5.4).
        assert_eq!(replay_key(&base, false).1, replay_key(&with_metadata, false).1);
    }

    #[test]
    fn key_changes_with_output_shaping_fields() {
        let base = prompt();
        let mut changed = prompt();
        changed.model = Some("different".to_owned());
        // A field that shapes output must change the key.
        assert_ne!(replay_key(&base, false).1, replay_key(&changed, false).1);
    }

    #[test]
    fn key_tracks_workspace_lent() {
        // The lent-workspace marker is part of the key (§5.4).
        assert_ne!(replay_key(&prompt(), false).1, replay_key(&prompt(), true).1);
    }
}
