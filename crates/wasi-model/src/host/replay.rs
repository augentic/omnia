//! Record and replay support for testing.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use futures::FutureExt as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::generated::augentic::model::completion::{Prompt, ResponseFormatKind, ToolChoice};
use super::types::{BackendAnswer, PreparedPrompt, Transcript};
use super::{FutureResult, ToolHost, WasiModelCtx};

/// In-memory replay index keyed by canonical prompt JSON.
#[derive(Debug, Default)]
pub struct FixtureStore {
    answers: HashMap<String, BackendAnswer>,
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

impl TryFrom<&[&str]> for FixtureStore {
    type Error = anyhow::Error;
    fn try_from(documents: &[&str]) -> Result<Self> {
        let mut store = Self::default();
        for doc in documents {
            let fixture: Fixture = serde_json::from_str(doc).context("parsing embedded fixture")?;
            store.insert(fixture);
        }
        Ok(store)
    }
}

impl FixtureStore {
    /// The replayed answer for an equivalent prompt, if one was recorded.
    #[must_use]
    pub fn get(&self, request: &PreparedPrompt) -> Option<BackendAnswer> {
        let key = ReplayKey::for_request(request);
        self.answers.get(&key.lookup).cloned()
    }

    /// The replayed answer for `request`.
    ///
    /// # Errors
    ///
    /// Returns an error when no equivalent fixture has been recorded.
    pub fn answer_for(&self, request: &PreparedPrompt) -> Result<BackendAnswer> {
        if let Some(answer) = self.get(request) {
            return Ok(answer);
        }

        let key = ReplayKey::for_request(request);
        anyhow::bail!("no replay fixture for key {:016x}", stable_key_id(&key.lookup))
    }

    /// The number of indexed fixtures.
    #[must_use]
    pub fn len(&self) -> usize {
        self.answers.len()
    }

    fn insert(&mut self, fixture: Fixture) {
        let key = ReplayKey::from_fixture_prompt(&fixture.key_prompt);
        self.answers.insert(
            key.lookup,
            BackendAnswer {
                value: fixture.answer,
                transcript: fixture.transcript,
            },
        );
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

#[derive(Debug)]
struct ReplayKey {
    prompt: Value,
    lookup: String,
}

impl ReplayKey {
    fn for_request(request: &PreparedPrompt) -> Self {
        Self::from_prompt(&request.prompt, request.workspace_lent)
    }

    fn from_prompt(prompt: &Prompt, workspace_lent: bool) -> Self {
        let prompt = canonicalize(&reduced_value(prompt, workspace_lent));
        let lookup = canonical_json(&prompt);
        Self { prompt, lookup }
    }

    fn from_fixture_prompt(prompt: &Value) -> Self {
        let prompt = canonicalize(prompt);
        let lookup = canonical_json(&prompt);
        Self { prompt, lookup }
    }

    fn filename(&self) -> String {
        format!("{:016x}.json", stable_key_id(&self.lookup))
    }
}

#[cfg(test)]
pub fn record_fixture(
    dir: &Path, request: &PreparedPrompt, answer: &BackendAnswer,
) -> Result<PathBuf> {
    write_fixture(dir, ReplayKey::for_request(request), answer)
}

fn write_fixture(dir: &Path, key: ReplayKey, answer: &BackendAnswer) -> Result<PathBuf> {
    fs::create_dir_all(dir).with_context(|| format!("creating replay dir {}", dir.display()))?;
    let path = dir.join(key.filename());
    let fixture = Fixture {
        key_prompt: key.prompt,
        answer: answer.value.clone(),
        transcript: answer.transcript.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&fixture).context("serializing fixture")?;
    fs::write(&path, bytes).with_context(|| format!("writing fixture {}", path.display()))?;
    Ok(path)
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

fn canonical_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn stable_key_id(key: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// A recording `WasiModelCtx` that wraps another backend.
#[derive(Debug)]
pub struct Recording<C: WasiModelCtx> {
    inner: C,
    dir: PathBuf,
}

impl<C: WasiModelCtx> Recording<C> {
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
        let key = ReplayKey::for_request(&request);
        let inner = self.inner.complete(request, tool_host);
        let dir = self.dir.clone();

        async move {
            let answer = inner.await?;

            if let Err(error) = write_fixture(&dir, key, &answer) {
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
    use super::{
        FixtureStore, Prompt, ReplayKey, ResponseFormatKind, canonicalize, record_fixture,
    };
    use crate::host::types::{BackendAnswer, PreparedPrompt};

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

    fn request(workspace_lent: bool) -> PreparedPrompt {
        PreparedPrompt::assemble(prompt(), workspace_lent).expect("assemble")
    }

    #[test]
    fn canonicalize_sort() {
        let canonical = serde_json::to_string(&canonicalize(&json!({
            "b": 1,
            "a": { "z": 2, "y": 3 },
        })))
        .unwrap();
        assert_eq!(canonical, r#"{"a":{"y":3,"z":2},"b":1}"#);
    }

    #[test]
    fn ignore_metadata() {
        let base = prompt();
        let mut with_metadata = prompt();
        with_metadata.metadata = vec![MetadataEntry {
            key: "trace".to_owned(),
            value: "abc".to_owned(),
        }];

        assert_eq!(
            ReplayKey::from_prompt(&base, false).lookup,
            ReplayKey::from_prompt(&with_metadata, false).lookup
        );
    }

    #[test]
    fn output_shaping() {
        let base = prompt();
        let mut changed = prompt();
        changed.model = Some("different".to_owned());
        // A field that shapes output must change the key.
        assert_ne!(
            ReplayKey::from_prompt(&base, false).lookup,
            ReplayKey::from_prompt(&changed, false).lookup
        );
    }

    #[test]
    fn workspace_lent() {
        // The lent-workspace marker is part of the key (§5.4).
        assert_ne!(
            ReplayKey::from_prompt(&prompt(), false).lookup,
            ReplayKey::from_prompt(&prompt(), true).lookup
        );
    }

    #[test]
    fn embedded_fixture() {
        let key_prompt = ReplayKey::from_prompt(&prompt(), false).prompt;
        let doc = serde_json::json!({
            "key_prompt": key_prompt,
            "answer": { "verdict": "pass" },
        });
        let doc = doc.to_string();
        let store = FixtureStore::try_from([doc.as_str()].as_slice()).expect("parse embedded");
        assert_eq!(store.len(), 1);
        let answer = store.get(&request(false)).expect("hit embedded fixture");
        assert_eq!(answer.value, json!({ "verdict": "pass" }));
    }

    #[test]
    fn directory_fixture() {
        let dir = std::env::temp_dir().join(format!("omnia-fixture-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let request = request(false);
        record_fixture(
            &dir,
            &request,
            &BackendAnswer {
                value: json!({ "verdict": "pass" }),
                transcript: None,
            },
        )
        .expect("write fixture");

        let store = FixtureStore::try_from(&dir).expect("load fixtures");
        assert_eq!(store.len(), 1);
        let answer = store.get(&request).expect("hit loaded fixture");
        assert_eq!(answer.value, json!({ "verdict": "pass" }));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_dir() {
        let dir =
            std::env::temp_dir().join(format!("omnia-fixture-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let store = FixtureStore::try_from(&dir).expect("load fixtures");
        assert_eq!(store.len(), 0);
    }
}
