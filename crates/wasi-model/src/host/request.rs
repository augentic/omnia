//! Request validation and prompt shaping.

use std::fmt;
use std::net::IpAddr;

use crate::host::Error;
use crate::host::generated::omnia::model::completion::{
    Format, Mcp, Request, Role, Tool, WorkerGrant,
};

const RESERVED_TOOLS: &[&str] = &["read", "list", "write", "check"];

impl Request {
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

    /// The request's tool names, each carrying its own parameters schema.
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
    /// Returns an `InvalidRequest` error if the request is invalid.
    pub fn validate(&self) -> Result<(), Error> {
        for tool in &self.tools {
            let Tool::Function(function) = tool else {
                continue;
            };
            if RESERVED_TOOLS.contains(&function.name.as_str()) {
                return Err(Error::InvalidRequest(format!(
                    "reserved tool name: {}",
                    function.name
                )));
            }
            if serde_json::from_str::<serde_json::Value>(&function.parameters).is_err() {
                return Err(Error::InvalidRequest(format!(
                    "function tool `{}` parameters is not valid JSON",
                    function.name
                )));
            }
        }

        if self.messages.iter().all(|message| message.content.trim().is_empty()) {
            return Err(Error::InvalidRequest("empty request".to_owned()));
        }

        if let Format::Schema(spec) = &self.format
            && serde_json::from_str::<serde_json::Value>(&spec.schema).is_err()
        {
            return Err(Error::InvalidRequest("format schema is not valid JSON".to_owned()));
        }

        if let Some(worker) = &self.grants.worker {
            if self.grants.workspace.is_none() {
                return Err(Error::InvalidRequest(
                    "a worker grant requires a workspace lend".to_owned(),
                ));
            }
            worker.validate().map_err(Error::InvalidRequest)?;
        }

        Ok(())
    }
}

impl WorkerGrant {
    /// Hold each entry to its shape: an egress entry a host name or
    /// `*.suffix`, a read root `~/<path>` holding no `..`, an environment
    /// variable a name.
    ///
    /// # Errors
    ///
    /// Returns the first entry that breaks its shape, as `InvalidRequest` detail.
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
    let name = entry.strip_prefix("*.").unwrap_or(entry);
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

fn read_root(entry: &str) -> Result<(), &'static str> {
    let Some(rest) = entry.strip_prefix("~/") else {
        return Err("must be spelled `~/<path>`");
    };
    if rest.is_empty() {
        return Err("names the home itself");
    }
    if rest.split('/').any(|segment| segment == "..") {
        return Err("holds `..`");
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

#[cfg(test)]
mod tests {
    use super::{Format, Mcp, Request, Role, Tool};
    use crate::host::{Function, Grants, Message};

    fn request(system: Option<&str>, messages: Vec<(Role, &str)>, tools: Vec<Tool>) -> Request {
        Request {
            model: None,
            system: system.map(str::to_owned),
            messages: messages
                .into_iter()
                .map(|(role, content)| Message {
                    role,
                    content: content.to_owned(),
                })
                .collect(),
            generation: None,
            format: Format::Text,
            tools,
            grants: Grants {
                workspace: None,
                worker: None,
            },
            check: false,
        }
    }

    #[test]
    fn mcp_servers_skip_functions() {
        let request = request(
            None,
            vec![(Role::User, "hi")],
            vec![
                Tool::Function(Function {
                    name: "lookup".to_owned(),
                    description: "lookup".to_owned(),
                    parameters: "{}".to_owned(),
                }),
                Tool::Mcp(Mcp {
                    name: "docs".to_owned(),
                    tools: vec!["search".to_owned()],
                    url: "https://mcp.example".to_owned(),
                }),
            ],
        );
        let names: Vec<&str> =
            request.mcp_servers().iter().map(|grant| grant.name.as_str()).collect();
        assert_eq!(names, ["docs"]);
        assert_eq!(request.tool_names(), ["lookup"]);
    }

    #[test]
    fn prompt_joins_channels() {
        let prompt = request(
            Some("be terse"),
            vec![(Role::User, "hi"), (Role::Assistant, "yo"), (Role::System, "note")],
            vec![],
        )
        .to_string();
        assert!(
            prompt.starts_with("be terse\n\nhi\n\n[assistant]\nyo\n\n[system]\nnote\n\n"),
            "{prompt}"
        );
        assert!(prompt.contains("plain text"), "{prompt}");
    }

    #[test]
    fn role_display() {
        assert_eq!(Role::System.to_string(), "system");
        assert_eq!(Role::User.to_string(), "user");
        assert_eq!(Role::Assistant.to_string(), "assistant");
    }
}
