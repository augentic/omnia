//! Host-only owned types.
//!
//! Backends receive an *owned* conversion of the generated `prompt` at the
//! `WasiModelCtx` boundary, so they never hold wasmtime guest handles.
//! The backend return type ([`BackendAnswer`]) is host-only — a parsed answer
//! value plus an optional tool-call transcript for record/replay — and never
//! crosses the WIT boundary; the guest sees only the validated `answer` string.

use serde::{Deserialize, Serialize};

use super::generated::augentic::model::completion as genc;

/// One chat turn passed to the provider API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Turn role: `system`, `user`, or `assistant`.
    pub role: String,
    /// Turn body text.
    pub content: String,
}

/// One few-shot pair used when assembling from `sections`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Example {
    /// Example user input.
    pub input: String,
    /// Example model output.
    pub output: String,
}

/// Named substitution slot applied when assembling section text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Variable {
    /// Placeholder name (e.g. `language`).
    pub name: String,
    /// Value substituted for `name`.
    pub value: String,
}

/// Structured prompt template; used when `prompt.messages` is empty.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sections {
    /// Persona / actor instruction.
    pub role: Option<String>,
    /// What the model should do. Required when assembling from sections.
    pub task: String,
    /// Background documents, prior state, or prior turns.
    pub context: Option<String>,
    /// Rules, limits, and things to avoid.
    pub constraints: Vec<String>,
    /// Few-shot input/output pairs.
    pub examples: Vec<Example>,
    /// Template variables applied during assembly.
    pub variables: Vec<Variable>,
}

/// JSON Schema constrained output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonSchemaSpec {
    /// Schema name passed to the provider.
    pub name: String,
    /// JSON Schema document the answer must conform to.
    pub schema: String,
    /// Provider strict-mode hint when supported.
    pub strict: Option<bool>,
}

/// Selects validation depth for the returned `answer`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResponseFormatKind {
    /// Answer must be a JSON string value.
    Text,
    /// Answer must parse as a JSON object.
    JsonObject,
    /// Answer must validate against `response-format.json-schema`.
    JsonSchema,
}

/// Output shape constraint for the completion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseFormat {
    /// Selects validation depth.
    pub kind: ResponseFormatKind,
    /// Required when `kind` is `json-schema`; ignored otherwise.
    pub json_schema: Option<JsonSchemaSpec>,
}

/// Sampling and length controls. Omitted fields defer to backend defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenerationParams {
    /// Sampling temperature.
    pub temperature: Option<f32>,
    /// Nucleus sampling threshold.
    pub top_p: Option<f32>,
    /// Maximum output tokens.
    pub max_tokens: Option<u32>,
    /// Sequences that halt generation.
    pub stop: Vec<String>,
}

/// Guest-declared tool advertised to the model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionTool {
    /// Tool name. Must not collide with reserved host-injected tool names.
    pub name: String,
    /// Natural-language description for the model.
    pub description: String,
    /// JSON Schema for the tool's arguments object.
    pub parameters: String,
}

/// Tool selection policy (`tool_choice` in provider APIs).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolChoice {
    /// Provider selects whether to call tools.
    Auto,
    /// Do not call tools.
    None,
    /// Must call at least one tool.
    Required,
    /// Must call the named tool.
    Named(String),
}

/// Opaque tracing key/value pair (`metadata` / `user` in provider APIs).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataEntry {
    /// Metadata key.
    pub key: String,
    /// Metadata value.
    pub value: String,
}

/// Host capabilities lent for this completion.
///
/// The working-tree `borrow<descriptor>` never survives the boundary as a
/// handle: the host records only whether a tree was lent, and the actual
/// filesystem access is mediated by [`ToolHost`](super::ToolHost).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolGrants {
    /// Guest id whose `references` export `resolve` targets.
    pub references: Option<String>,
    /// Whether a working tree was lent for this call (the stable marker that
    /// replaces the non-serializable `borrow<descriptor>` for keying).
    pub working_tree_lent: bool,
    /// Allowed closed verification profile names for `verify`.
    pub verify: Vec<String>,
}

/// Complete models-API-style request for one completion (owned host mirror).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prompt {
    /// Opaque model id hint; passed through unchanged. Backend may override.
    pub model: Option<String>,
    /// System / instructions channel.
    pub system: Option<String>,
    /// Explicit chat turns. When non-empty, takes precedence over `sections`.
    pub messages: Vec<Message>,
    /// Structured template used when `messages` is empty.
    pub sections: Option<Sections>,
    /// Sampling and length controls.
    pub generation: Option<GenerationParams>,
    /// Required output shape and validation rules.
    pub response_format: ResponseFormat,
    /// Guest-declared tools merged with host-injected tools at the backend.
    pub tools: Vec<FunctionTool>,
    /// Tool selection policy.
    pub tool_choice: Option<ToolChoice>,
    /// Tracing and attribution metadata.
    pub metadata: Vec<MetadataEntry>,
    /// Host capabilities lent for this call.
    pub grants: ToolGrants,
}

/// Host-prepared input for one completion: the owned [`Prompt`] plus the provider
/// chat channels the host assembled from it (§3.1.1).
///
/// The host assembles once at the `complete` gate so every backend consumes the
/// same `system` / `messages`; backends must not re-derive them from `sections`.
#[derive(Clone, Debug, PartialEq)]
pub struct CompletionRequest {
    /// The owned guest prompt; record / replay keys on this, never the channels.
    pub prompt: Prompt,
    /// Assembled system / instructions channel, if any.
    pub system: Option<String>,
    /// Assembled chat turns to send to the provider.
    pub messages: Vec<Message>,
}

/// A reference an adapter asked the model to resolve (`ToolHost::resolve`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    /// The opaque reference body the adapter's `references` shelf interprets.
    pub name: String,
}

/// One bounded directory entry returned by `ToolHost::list`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    /// Entry name (never an OS path).
    pub name: String,
    /// Whether the entry is a directory.
    pub is_directory: bool,
}

/// The outcome of a `verify` profile run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyReport {
    /// Whether the check passed.
    pub ok: bool,
    /// Human-readable detail.
    pub detail: String,
}

/// One recorded tool interaction within a completion's transcript.
// `args`/`result` are `serde_json::Value`, which is not `Eq` (it carries f64),
// so this type can only be `PartialEq`.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolTurn {
    /// The tool the model called.
    pub tool: String,
    /// The arguments the model supplied.
    pub args: serde_json::Value,
    /// The result the host returned.
    pub result: serde_json::Value,
}

/// The tool-call transcript a backend may capture for record/replay. Host-only;
/// it never crosses the WIT boundary. Empty for backends with no tool loop
/// (replay, cursor) in Phase 1.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    /// Ordered tool turns the backend drove to reach the answer.
    pub turns: Vec<ToolTurn>,
}

/// A backend's result: the parsed answer value plus an optional transcript.
/// Host-only — the guest sees only the validated `answer` string the `complete`
/// binding derives from `value`.
#[derive(Clone, Debug, PartialEq)]
pub struct BackendAnswer {
    /// The parsed JSON answer the backend produced.
    pub value: serde_json::Value,
    /// Optional tool-call transcript for record/replay.
    pub transcript: Option<Transcript>,
}

impl From<genc::Message> for Message {
    fn from(m: genc::Message) -> Self {
        Self {
            role: m.role,
            content: m.content,
        }
    }
}

impl From<genc::Example> for Example {
    fn from(e: genc::Example) -> Self {
        Self {
            input: e.input,
            output: e.output,
        }
    }
}

impl From<genc::Variable> for Variable {
    fn from(v: genc::Variable) -> Self {
        Self {
            name: v.name,
            value: v.value,
        }
    }
}

impl From<genc::Sections> for Sections {
    fn from(s: genc::Sections) -> Self {
        Self {
            role: s.role,
            task: s.task,
            context: s.context,
            constraints: s.constraints,
            examples: s.examples.into_iter().map(Example::from).collect(),
            variables: s.variables.into_iter().map(Variable::from).collect(),
        }
    }
}

impl From<genc::JsonSchemaSpec> for JsonSchemaSpec {
    fn from(j: genc::JsonSchemaSpec) -> Self {
        Self {
            name: j.name,
            schema: j.schema,
            strict: j.strict,
        }
    }
}

impl From<genc::ResponseFormatKind> for ResponseFormatKind {
    fn from(k: genc::ResponseFormatKind) -> Self {
        match k {
            genc::ResponseFormatKind::Text => Self::Text,
            genc::ResponseFormatKind::JsonObject => Self::JsonObject,
            genc::ResponseFormatKind::JsonSchema => Self::JsonSchema,
        }
    }
}

impl From<genc::ResponseFormat> for ResponseFormat {
    fn from(r: genc::ResponseFormat) -> Self {
        Self {
            kind: r.kind.into(),
            json_schema: r.json_schema.map(JsonSchemaSpec::from),
        }
    }
}

impl From<genc::GenerationParams> for GenerationParams {
    fn from(g: genc::GenerationParams) -> Self {
        Self {
            temperature: g.temperature,
            top_p: g.top_p,
            max_tokens: g.max_tokens,
            stop: g.stop,
        }
    }
}

impl From<genc::FunctionTool> for FunctionTool {
    fn from(t: genc::FunctionTool) -> Self {
        Self {
            name: t.name,
            description: t.description,
            parameters: t.parameters,
        }
    }
}

impl From<genc::ToolChoice> for ToolChoice {
    fn from(c: genc::ToolChoice) -> Self {
        match c {
            genc::ToolChoice::Auto => Self::Auto,
            genc::ToolChoice::None => Self::None,
            genc::ToolChoice::Required => Self::Required,
            genc::ToolChoice::Named(name) => Self::Named(name),
        }
    }
}

impl From<genc::MetadataEntry> for MetadataEntry {
    fn from(m: genc::MetadataEntry) -> Self {
        Self {
            key: m.key,
            value: m.value,
        }
    }
}

impl From<genc::ToolGrants> for ToolGrants {
    fn from(g: genc::ToolGrants) -> Self {
        Self {
            references: g.references,
            // The `borrow<descriptor>` is reduced to a stable marker here; the
            // descriptor itself is resolved against the table by the host when
            // it builds the `ToolHost` (Phase 2b wires it to a real tree).
            working_tree_lent: g.working_tree.is_some(),
            verify: g.verify,
        }
    }
}

impl From<genc::Prompt> for Prompt {
    fn from(p: genc::Prompt) -> Self {
        Self {
            model: p.model,
            system: p.system,
            messages: p.messages.into_iter().map(Message::from).collect(),
            sections: p.sections.map(Sections::from),
            generation: p.generation.map(GenerationParams::from),
            response_format: p.response_format.into(),
            tools: p.tools.into_iter().map(FunctionTool::from).collect(),
            tool_choice: p.tool_choice.map(ToolChoice::from),
            metadata: p.metadata.into_iter().map(MetadataEntry::from).collect(),
            grants: p.grants.into(),
        }
    }
}
