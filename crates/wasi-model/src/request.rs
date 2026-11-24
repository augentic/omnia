//! The request a backend receives, the checks the host holds it to, and
//! prompt shaping.

use std::collections::HashSet;
use std::fmt;

use omnia_core::host_pattern;

use crate::generated::omnia::model::{completion, types};
use crate::{Effort, Error, Format, Grants, Location, Message, Role, WorkerGrant};

const RESERVED_TOOLS: &[&str] = &["read", "list", "write"];

/// A guest's request, as its backend receives it.
#[derive(Debug)]
pub struct Request {
    /// The name the guest opened the model by.
    pub model: String,
    /// Standing instructions, read before `messages`.
    pub system: Option<String>,
    /// The conversation so far, oldest first.
    pub messages: Vec<Message>,
    /// `None` takes the backend's defaults.
    pub generation: Option<Generation>,
    /// The shape the answer should take.
    pub format: Format,
    /// The tools the guest runs for the model.
    pub tools: Vec<Tool>,
    /// What the guest lends the completion.
    pub grants: Grants,
    /// Whether the guest judges each candidate through
    /// [`ToolHost::check`](crate::ToolHost::check).
    pub check: bool,
}

/// The controls on how the model generates.
///
/// An absent field takes the backend's default.
#[derive(Clone, Debug)]
pub struct Generation {
    /// The sampling temperature.
    pub temperature: Option<f32>,
    /// The nucleus sampling threshold.
    pub top_p: Option<f32>,
    /// The most tokens the model writes in one response.
    pub max_tokens: Option<u32>,
    /// Texts at which the model stops writing.
    pub stop: Vec<String>,
    /// The seed that makes sampling repeatable.
    pub seed: Option<u64>,
    /// How much the model reasons before it answers. A backend fails an
    /// [`Effort::Other`] level it does not know with [`Error::Invalid`].
    pub effort: Option<Effort>,
}

/// A tool offered to the model.
#[derive(Clone, Debug)]
pub enum Tool {
    /// A tool the guest runs.
    Function(Function),
}

/// A tool the guest runs for the model.
#[derive(Clone, Debug)]
pub struct Function {
    /// The name the model calls the tool by.
    pub name: String,
    /// What the tool does, for the model to read, `None` where the guest gave
    /// none.
    pub description: Option<String>,
    /// The JSON Schema of the tool's arguments.
    pub parameters: String,
}

impl Request {
    pub(crate) fn new(name: String, request: completion::Request) -> Self {
        Self {
            model: name,
            system: request.system,
            messages: request.messages,
            generation: request.generation.map(Into::into),
            format: request.format,
            tools: request.tools.into_iter().map(|tool| Tool::Function(tool.into())).collect(),
            grants: request.grants,
            check: request.check,
        }
    }

    /// The names of the request's tools.
    pub fn tool_names(&self) -> Vec<String> {
        self.tools.iter().map(|Tool::Function(function)| function.name.clone()).collect()
    }

    /// Validate the request and its tools.
    ///
    /// # Errors
    ///
    /// Returns an [`Error::Invalid`] error if the request is invalid.
    pub fn validate(&self) -> Result<(), Error> {
        let mut names = HashSet::new();
        for Tool::Function(function) in &self.tools {
            if RESERVED_TOOLS.contains(&function.name.as_str()) {
                return Err(Error::Invalid(format!("reserved tool name: {}", function.name)));
            }
            if !names.insert(function.name.as_str()) {
                return Err(Error::Invalid(format!("duplicate tool name: {}", function.name)));
            }
            if serde_json::from_str::<serde_json::Value>(&function.parameters).is_err() {
                return Err(Error::Invalid(format!(
                    "function tool `{}` parameters is not valid JSON",
                    function.name
                )));
            }
        }

        if self.messages.iter().all(|message| message.content.trim().is_empty()) {
            return Err(Error::Invalid("empty request".to_owned()));
        }

        if let Format::Schema(spec) = &self.format
            && serde_json::from_str::<serde_json::Value>(&spec.schema).is_err()
        {
            return Err(Error::Invalid("format schema is not valid JSON".to_owned()));
        }

        if let Some(worker) = self.grants.worker() {
            worker.validate().map_err(Error::Invalid)?;
        }

        Ok(())
    }
}

impl Grants {
    /// The directory lent to the completion, `None` where the guest lends none.
    #[must_use]
    pub fn location(&self) -> Option<&Location> {
        self.workspace.as_ref().map(|workspace| &workspace.location)
    }

    /// What the backend's worker may reach beyond the lent directory, `None`
    /// where the guest grants nothing more.
    #[must_use]
    pub fn worker(&self) -> Option<&WorkerGrant> {
        self.workspace.as_ref().and_then(|workspace| workspace.worker.as_ref())
    }
}

impl Effort {
    /// The level's name: `low`, `medium`, `high`, or the backend's own.
    #[must_use]
    pub const fn as_str(&self) -> &str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Other(level) => level.as_str(),
        }
    }
}

impl From<types::Tool> for Function {
    fn from(tool: types::Tool) -> Self {
        Self {
            name: tool.name,
            description: tool.description,
            parameters: tool.input_schema,
        }
    }
}

impl Generation {
    pub(crate) const fn is_set(&self) -> bool {
        self.temperature.is_some()
            || self.top_p.is_some()
            || self.max_tokens.is_some()
            || !self.stop.is_empty()
            || self.seed.is_some()
            || self.effort.is_some()
    }
}

impl From<types::Generation> for Generation {
    fn from(generation: types::Generation) -> Self {
        Self {
            temperature: generation.temperature,
            top_p: generation.top_p,
            max_tokens: generation.max_tokens,
            stop: generation.stop_sequences,
            seed: generation.seed,
            effort: generation.effort,
        }
    }
}

impl WorkerGrant {
    /// Hold each entry to its shape: an egress entry a [host pattern](host_pattern),
    /// a read root `~/` then a plain relative path, an environment variable
    /// a name.
    ///
    /// # Errors
    ///
    /// Returns the first entry that breaks its shape, as [`Error::Invalid`]
    /// detail.
    pub fn validate(&self) -> Result<(), String> {
        for entry in &self.egress {
            host_pattern(entry).map_err(|what| format!("worker egress `{entry}` {what}"))?;
        }
        for entry in &self.read {
            read_root(entry).map_err(|what| format!("worker read root `{entry}` {what}"))?;
        }
        for (name, _) in &self.env {
            if !is_env_name(name) {
                return Err(format!("worker env `{name}` is not an environment variable name"));
            }
        }
        Ok(())
    }
}

// `~/` then a plain relative path, the shape a workspace lend's subpath is
// held to: `/`-separated, no segment empty, `.` or `..`, no `\`. Anything
// looser names the home (`~/.`) or, once a backend joins it, leaves it (`~//etc`)
fn read_root(entry: &str) -> Result<(), &'static str> {
    let Some(rest) = entry.strip_prefix("~/") else {
        return Err("must be spelled `~/<path>`");
    };
    if rest.split('/').all(|segment| segment.is_empty() || segment == ".") {
        return Err("names the home itself");
    }
    if rest.split('/').any(|segment| segment == "..") {
        return Err("holds `..`");
    }
    if rest.contains('\\') || rest.split('/').any(|segment| segment.is_empty() || segment == ".") {
        return Err("is not a plain `/`-separated path");
    }
    Ok(())
}

fn is_env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|first| first == b'_' || first.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

// the prompt: system text, each message (non-user ones under a `[role]`
// header) and the format's final-answer instruction, joined by blank lines
impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut blocks: Vec<String> = self.system.iter().cloned().collect();
        blocks.extend(self.messages.iter().map(|message| match message.role {
            Role::User => message.content.clone(),
            Role::System | Role::Assistant => {
                format!("[{}]\n{}", message.role, message.content)
            }
        }));
        blocks.push(self.format.instruction());
        f.write_str(&blocks.join("\n\n"))
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        })
    }
}
