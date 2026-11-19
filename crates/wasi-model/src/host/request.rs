//! The request a backend receives, the checks the host holds it to, and
//! prompt shaping.

use std::fmt;
use std::net::IpAddr;

use crate::host::generated::omnia::model::{completion, mcp, types};
use crate::host::{Error, Format, Grants, Message, Role, WorkerGrant};

const RESERVED_TOOLS: &[&str] = &["read", "list", "write"];

// The model a request opens when it names none, which a backend receives as
// `Request::model: None`.
const DEFAULT_MODEL: &str = "default";

/// A guest's request, as its backend receives it.
#[derive(Debug)]
pub struct Request {
    /// The model the guest opened, `None` for `default`.
    pub model: Option<String>,
    /// Standing instructions, read before `messages`.
    pub system: Option<String>,
    /// The conversation so far, oldest first.
    pub messages: Vec<Message>,
    /// `None` takes the backend's defaults.
    pub generation: Option<Generation>,
    /// The shape the answer should take.
    pub format: Format,
    /// The tools the guest runs, then the MCP servers the guest offers.
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
    /// How much the model reasons before it answers.
    pub effort: Option<Effort>,
}

/// How much the model reasons before it answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effort {
    /// The least reasoning the model does.
    Minimal,
    /// Less reasoning.
    Low,
    /// Balanced reasoning.
    Medium,
    /// The most reasoning.
    High,
}

/// A tool offered to the model.
#[derive(Clone, Debug)]
pub enum Tool {
    /// A tool the guest runs.
    Function(Function),
    /// A Model Context Protocol server, whose tools the backend calls itself.
    Mcp(Mcp),
}

/// A tool the guest runs for the model.
#[derive(Clone, Debug)]
pub struct Function {
    /// The name the model calls the tool by.
    pub name: String,
    /// What the tool does, for the model to read. Empty where the guest gave
    /// none.
    pub description: String,
    /// The JSON Schema of the tool's arguments.
    pub parameters: String,
}

/// A Model Context Protocol server whose tools the model may call.
#[derive(Clone, Debug)]
pub struct Mcp {
    /// Names the server to the model.
    pub name: String,
    /// The tools the model may call, empty for every tool the server lists.
    pub tools: Vec<String>,
    /// The server's HTTP endpoint.
    pub url: String,
}

impl Request {
    // The request the guest sent to the model `name`, with the tools of
    // `servers` after its own.
    pub(crate) fn new(
        name: String, request: completion::Request, servers: Vec<mcp::Server>,
    ) -> Result<Self, Error> {
        let mut tools: Vec<Tool> =
            request.tools.into_iter().map(|tool| Tool::Function(tool.into())).collect();
        for server in servers {
            tools.push(Tool::Mcp(server.try_into()?));
        }

        Ok(Self {
            model: (name != DEFAULT_MODEL).then_some(name),
            system: request.system,
            messages: request.messages,
            generation: request.generation.map(Generation::try_from).transpose()?,
            format: request.format,
            tools,
            grants: request.grants,
            check: request.check,
        })
    }

    /// The request's MCP server grants, each carrying its own endpoint URL.
    #[must_use]
    pub fn mcp_servers(&self) -> Vec<&Mcp> {
        self.tools
            .iter()
            .filter_map(|tool| match tool {
                Tool::Mcp(grant) => Some(grant),
                Tool::Function(_) => None,
            })
            .collect()
    }

    /// The names of the request's function tools.
    pub fn tool_names(&self) -> Vec<String> {
        self.tools
            .iter()
            .filter_map(|tool| match tool {
                Tool::Function(function) => Some(function.name.clone()),
                Tool::Mcp(_) => None,
            })
            .collect()
    }

    /// Validate the request and its tools.
    ///
    /// # Errors
    ///
    /// Returns an [`Error::Invalid`] error if the request is invalid.
    pub fn validate(&self) -> Result<(), Error> {
        for tool in &self.tools {
            let Tool::Function(function) = tool else {
                continue;
            };
            if RESERVED_TOOLS.contains(&function.name.as_str()) {
                return Err(Error::Invalid(format!("reserved tool name: {}", function.name)));
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

        if let Some(worker) = &self.grants.worker {
            if self.grants.workspace.is_none() {
                return Err(Error::Invalid("a worker grant requires a workspace lend".to_owned()));
            }
            worker.validate().map_err(Error::Invalid)?;
        }

        Ok(())
    }
}

impl From<types::Tool> for Function {
    fn from(tool: types::Tool) -> Self {
        Self {
            name: tool.name,
            description: tool.description.unwrap_or_default(),
            parameters: tool.input_schema,
        }
    }
}

impl TryFrom<mcp::Server> for Mcp {
    type Error = Error;

    fn try_from(server: mcp::Server) -> Result<Self, Error> {
        if server.authorization.is_some() {
            return Err(Error::Unsupported(format!(
                "MCP server `{}` sets an authorization, which this host does not send",
                server.name
            )));
        }
        if server.allowed_tools.as_ref().is_some_and(Vec::is_empty) {
            return Err(Error::Invalid(format!(
                "MCP server `{}` allows an empty list of tools",
                server.name
            )));
        }
        Ok(Self {
            name: server.name,
            tools: server.allowed_tools.unwrap_or_default(),
            url: server.url,
        })
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

impl TryFrom<types::Generation> for Generation {
    type Error = Error;

    fn try_from(generation: types::Generation) -> Result<Self, Error> {
        Ok(Self {
            temperature: generation.temperature,
            top_p: generation.top_p,
            max_tokens: generation.max_tokens,
            stop: generation.stop_sequences,
            seed: generation.seed,
            effort: generation.effort.as_deref().map(Effort::try_from).transpose()?,
        })
    }
}

impl TryFrom<&str> for Effort {
    type Error = Error;

    fn try_from(level: &str) -> Result<Self, Error> {
        match level {
            "minimal" => Ok(Self::Minimal),
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            level => Err(Error::Invalid(format!("unknown effort `{level}`"))),
        }
    }
}

impl WorkerGrant {
    /// Hold each entry to its shape: an egress entry a host name or
    /// `*.suffix`, a read root `~/` then a plain relative path, an
    /// environment variable a name.
    ///
    /// # Errors
    ///
    /// Returns the first entry that breaks its shape, as [`Error::Invalid`]
    /// detail.
    pub fn validate(&self) -> Result<(), String> {
        for entry in &self.egress {
            egress_host(entry).map_err(|what| format!("worker egress `{entry}` {what}"))?;
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

// a name whole or `*.suffix`: labels of `[A-Za-z0-9-]` joined by `.`, never
// an address, a port, a scheme, or a path
fn egress_host(entry: &str) -> Result<(), &'static str> {
    if entry.contains("://") {
        return Err("carries a scheme");
    }
    if entry.contains('/') {
        return Err("carries a path");
    }
    let name = entry.trim_prefix("*.");
    if name.starts_with('[') || name.parse::<IpAddr>().is_ok() {
        return Err("is an IP literal, not a host name");
    }
    if name.contains(':') {
        return Err("carries a port");
    }
    let labelled = !name.is_empty()
        && name.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        });
    if !labelled {
        return Err("is not a host name or `*.suffix`");
    }
    Ok(())
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
