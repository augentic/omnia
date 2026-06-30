//! Host-only types used by backends.

use serde::{Deserialize, Serialize};

use super::generated::augentic::model::completion::{Message, Prompt};

/// Host-prepared input for one completion: the generated guest prompt plus the
/// provider chat channels the host assembled from it (§3.1.1).
///
/// The host assembles once at the `complete` gate so every backend consumes the
/// same `system` / `messages`; backends must not re-derive them from `sections`.
#[derive(Debug)]
pub struct PreparedPrompt {
    /// The guest prompt; replay keys on this, never the channels. The
    /// host has already taken the lent `grants.workspace` borrow, so it is
    /// always `None` here — `workspace_lent` carries the keying marker instead.
    pub prompt: Prompt,
    /// Whether a workspace was lent for this call: the stable marker that
    /// replaces the non-serializable `borrow<descriptor>` when keying (§5.4).
    pub workspace_lent: bool,
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

/// The tool-call transcript a backend may capture for replay. Host-only;
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
pub struct Answer {
    /// The parsed JSON answer the backend produced.
    pub value: serde_json::Value,
    /// Optional tool-call transcript for replay.
    pub transcript: Option<Transcript>,
}
