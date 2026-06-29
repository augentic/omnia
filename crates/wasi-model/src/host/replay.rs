//! Boundary-level record / replay (§3.4, §5.4).
//!
//! Record and replay are composable `WasiModelCtx` wrappers, not a decorator
//! framework: [`Recording`] is a `WasiModelCtx` that wraps another and logs
//! `(prompt, transcript) -> answer`, and [`ModelDefault`](super::ModelDefault)
//! is a `WasiModelCtx` that serves a recorded answer for an equivalent prompt.
//! Both sit at the typed `complete` boundary, so a fixture captured against any
//! backend replays identically.
//!
//! ## Keying (§5.4)
//!
//! The key is the *canonical JSON* of the prompt reduced to the fields that
//! determine the model's output: `metadata` is dropped (tracing only) and the
//! working-tree handle is already a stable `working_tree_lent` boolean marker on
//! the owned [`Prompt`] (never a run-specific `borrow`). Canonical JSON sorts
//! object keys recursively and emits no insignificant whitespace, so a fixture
//! recorded against one backend matches under another.
//!
//! Phase 1 keys by canonical-JSON *equality* (the canonical string is the map
//! key) rather than `sha256(canonical_json)`, honouring the Phase 1 exit
//! criterion of *no new dependency beyond `serde_json`*. The on-disk filename
//! uses a non-cryptographic std hash purely for uniqueness; matching never
//! depends on it. Turning the key into a content-addressed `sha256` is a tracked
//! follow-up alongside the fixture-management expansion (§5.4, Phase 3).

use std::collections::HashMap;
use std::fs;
use std::hash::{Hash as _, Hasher as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use futures::FutureExt as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::types::{BackendAnswer, CompletionRequest, Prompt, Transcript};
use super::{FutureResult, ToolHost, WasiModelCtx};

/// A `(prompt + transcript) -> answer` row, the unit of replay (§5.4).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fixture {
    /// The reduced prompt that determines the answer (the key, §5.4).
    pub key_prompt: Value,
    /// The validated JSON answer to replay.
    pub answer: Value,
    /// The tool-call transcript captured when the fixture was recorded.
    #[serde(default)]
    pub transcript: Option<Transcript>,
}

/// The canonical replay key for `prompt`: canonical JSON of the prompt reduced
/// per §5.4 (drop `metadata`; the working-tree marker is already a boolean).
#[must_use]
pub fn canonical_key(prompt: &Prompt) -> String {
    key_from_value(&reduced_value(prompt))
}

/// Reduce a prompt to its output-determining fields (drops `metadata`).
fn reduced_value(prompt: &Prompt) -> Value {
    let mut value = serde_json::to_value(prompt).unwrap_or(Value::Null);
    if let Value::Object(map) = &mut value {
        map.remove("metadata");
    }
    value
}

/// The canonical string for an already-reduced value (sorted keys, compact).
fn key_from_value(value: &Value) -> String {
    serde_json::to_string(&canonicalize(value)).unwrap_or_default()
}

/// Recursively sort object keys so serialization is canonical regardless of the
/// `serde_json` map backing (`preserve_order` on or off).
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

/// A non-cryptographic, stable-within-a-build filename for a fixture. Matching
/// is by canonical-string equality, so this only needs to avoid collisions.
fn fixture_filename(canonical: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    canonical.hash(&mut hasher);
    format!("{:016x}.json", hasher.finish())
}

/// Write a fixture for `(prompt, answer)` into `dir`, returning its path.
///
/// # Errors
///
/// Returns an error if the directory cannot be created or the file written.
pub fn write_fixture(dir: &Path, prompt: &Prompt, answer: &BackendAnswer) -> Result<PathBuf> {
    fs::create_dir_all(dir).with_context(|| format!("creating replay dir {}", dir.display()))?;
    let key_prompt = canonicalize(&reduced_value(prompt));
    let canonical = serde_json::to_string(&key_prompt).unwrap_or_default();
    let fixture = Fixture {
        key_prompt,
        answer: answer.value.clone(),
        transcript: answer.transcript.clone(),
    };
    let path = dir.join(fixture_filename(&canonical));
    let bytes = serde_json::to_vec_pretty(&fixture).context("serializing fixture")?;
    fs::write(&path, bytes).with_context(|| format!("writing fixture {}", path.display()))?;
    Ok(path)
}

/// An in-memory index of fixtures loaded from a directory, keyed by canonical
/// prompt. Built once at backend `connect`.
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
                answers.insert(
                    key_from_value(&fixture.key_prompt),
                    BackendAnswer {
                        value: fixture.answer,
                        transcript: fixture.transcript,
                    },
                );
            }
        }
        Ok(Self { answers })
    }

    /// The replayed answer for an equivalent prompt, if one was recorded.
    #[must_use]
    pub fn get(&self, prompt: &Prompt) -> Option<BackendAnswer> {
        self.answers.get(&canonical_key(prompt)).cloned()
    }

    /// The number of loaded fixtures.
    #[must_use]
    pub fn len(&self) -> usize {
        self.answers.len()
    }

    /// Whether the store has no fixtures.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.answers.is_empty()
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
        &self, request: CompletionRequest, tool_host: Arc<dyn ToolHost>,
    ) -> FutureResult<BackendAnswer> {
        // Keep the prompt for the fixture key before the request is consumed.
        let prompt = request.prompt.clone();
        let inner = self.inner.complete(request, tool_host);
        let dir = self.dir.clone();
        async move {
            let answer = inner.await?;
            // Recording is best-effort: a write failure is logged, not fatal —
            // it must never break a live completion.
            if let Err(error) = write_fixture(&dir, &prompt, &answer) {
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

    use super::{canonical_key, canonicalize};
    use crate::host::types::{
        MetadataEntry, Prompt, ResponseFormat, ResponseFormatKind, Sections, ToolGrants,
    };

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
                working_tree_lent: false,
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
        assert_eq!(canonical_key(&base), canonical_key(&with_metadata));
    }

    #[test]
    fn key_changes_with_output_shaping_fields() {
        let base = prompt();
        let mut changed = prompt();
        changed.model = Some("different".to_owned());
        // A field that shapes output must change the key.
        assert_ne!(canonical_key(&base), canonical_key(&changed));
    }

    #[test]
    fn key_is_deterministic() {
        assert_eq!(canonical_key(&prompt()), canonical_key(&prompt()));
    }
}
