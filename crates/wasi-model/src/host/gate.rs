//! Host-side validation and message assembly for the `create` binding.

use serde_json::Value;

use crate::host::Error;
use crate::host::generated::omnia::model::completion::{Format, Message, Prompt, Role, Tool};
use crate::host::types::{Answer, PreparedPrompt};

const TOOL_NAMES: &[&str] = &["resolve", "read", "list", "write", "verify"];

impl TryFrom<Prompt> for PreparedPrompt {
    type Error = Error;

    fn try_from(prompt: Prompt) -> Result<Self, Self::Error> {
        // Only guest-declared functions carry a name that could shadow a
        // host-injected tool; MCP grants name a server, not a tool.
        if let Some(name) = prompt.tools.iter().find_map(|t| match t {
            Tool::Function(f) if TOOL_NAMES.contains(&f.name.as_str()) => Some(f.name.clone()),
            _ => None,
        }) {
            return Err(Error::Backend(format!("reserved tool name: {name}")));
        }

        // `messages` wins over `sections`. `prompt.system` is always applied.
        if !prompt.messages.is_empty() {
            let system = prompt.system.clone().filter(|v| !v.is_empty());
            let messages = prompt.messages.clone();

            return Ok(Self {
                prompt,
                system,
                messages,
            });
        }

        if prompt.messages.is_empty()
            && prompt.sections.as_ref().is_none_or(|s| s.task.trim().is_empty())
        {
            return Err(Error::Backend("empty prompt".to_owned()));
        }

        // try_from from `sections` when `messages` is empty.
        let Some(sections) = &prompt.sections else {
            return Err(Error::Backend("empty prompt".to_owned()));
        };
        if sections.task.trim().is_empty() {
            return Err(Error::Backend("empty prompt".to_owned()));
        }

        // substitute variables in text
        let substitute = |text: &str| {
            let mut out = text.to_owned();
            for variable in &sections.variables {
                out = out.replace(&format!("{{{}}}", variable.name), &variable.value);
            }
            out
        };

        // system channel: prompt.system, sections.role, formatted constraints.
        let mut system_parts: Vec<String> = Vec::new();
        if let Some(system) = &prompt.system {
            system_parts.push(system.clone());
        }
        if let Some(role) = &sections.role {
            system_parts.push(substitute(role));
        }
        for constraint in &sections.constraints {
            system_parts.push(format!("- {}", substitute(constraint)));
        }

        // user channel: task, context, formatted examples (variables substituted).
        let mut user_parts: Vec<String> = vec![substitute(&sections.task)];
        if let Some(context) = &sections.context {
            user_parts.push(substitute(context));
        }
        for example in &sections.examples {
            user_parts.push(format!(
                "Input: {}\nOutput: {}",
                substitute(&example.input),
                substitute(&example.output)
            ));
        }

        let system = join_non_empty(&system_parts);
        let messages = vec![Message {
            role: Role::User,
            content: join_non_empty(&user_parts).unwrap_or_default(),
        }];

        Ok(Self {
            prompt,
            system,
            messages,
        })
    }
}

fn join_non_empty(parts: &[String]) -> Option<String> {
    let kept: Vec<&str> = parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).collect();
    if kept.is_empty() { None } else { Some(kept.join("\n\n")) }
}

impl Answer {
    /// Validate an answer against the prompt's `format`.
    ///
    /// # Errors
    ///
    /// Returns an error if the answer does not match the requested format.
    pub fn check(value: &Value, format: &Format) -> Result<(), Error> {
        match format {
            Format::Text => {
                if !value.is_string() {
                    return Err(Error::InvalidAnswer("answer is not a JSON string".to_owned()));
                }
                Ok(())
            }
            Format::Json => {
                if !value.is_object() {
                    return Err(Error::InvalidAnswer("answer is not a JSON object".to_owned()));
                }
                Ok(())
            }
            Format::Schema(_) => {
                // TODO: validate against the schema document.
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Answer, Error, PreparedPrompt};
    use crate::host::generated::omnia::model::completion::{
        Example, Format, Function, Grants, Message, Prompt, Role, Schema, Sections, Tool, Variable,
    };

    fn schema() -> Schema {
        Schema {
            name: "verdict".to_owned(),
            schema: "{\"type\":\"object\"}".to_owned(),
            strict: None,
        }
    }

    #[test]
    fn json_string() {
        Answer::check(&json!("hi"), &Format::Text).unwrap();
        let err = Answer::check(&json!({ "a": 1 }), &Format::Text).unwrap_err();
        assert!(matches!(err, Error::InvalidAnswer(_)));
    }

    #[test]
    fn json_object() {
        Answer::check(&json!({ "verdict": "pass" }), &Format::Json).unwrap();
        let err = Answer::check(&json!("nope"), &Format::Json).unwrap_err();
        assert!(matches!(err, Error::InvalidAnswer(_)));
    }

    #[test]
    fn json_schema() {
        Answer::check(&json!({ "x": [1, 2, 3] }), &Format::Schema(schema())).unwrap();
        Answer::check(&json!(42), &Format::Schema(schema())).unwrap();
    }

    #[test]
    fn reserved_tool_name() {
        let mut prompt = prompt_from(vec![message(Role::User, "hi")], None);
        prompt.tools.push(Tool::Function(Function {
            name: "read".to_owned(),
            description: "shadow a host-injected tool".to_owned(),
            parameters: "{}".to_owned(),
        }));
        let err = PreparedPrompt::try_from(prompt).unwrap_err();
        assert!(matches!(err, Error::Backend(m) if m.contains("reserved tool name")));
    }

    #[test]
    fn empty_prompt() {
        let err = PreparedPrompt::try_from(prompt_from(vec![], None)).unwrap_err();
        assert!(matches!(err, Error::Backend(m) if m == "empty prompt"));

        // sections present but task blank is still empty.
        let err = PreparedPrompt::try_from(prompt_from(vec![], Some(sections("   ")))).unwrap_err();
        assert!(matches!(err, Error::Backend(m) if m == "empty prompt"));
    }

    #[test]
    fn non_empty() {
        PreparedPrompt::try_from(prompt_from(vec![message(Role::User, "hi")], None)).unwrap();
        PreparedPrompt::try_from(prompt_from(vec![], Some(sections("do it")))).unwrap();
    }

    #[test]
    fn explicit_messages() {
        // Precedence rule 1: when `messages` is non-empty, `sections` is ignored.
        let prompt = prompt_from(vec![message(Role::User, "explicit")], Some(sections("ignored")));
        let assembled = PreparedPrompt::try_from(prompt).expect("try_from");
        assert_eq!(assembled.messages.len(), 1);
        assert!(matches!(assembled.messages[0].role, Role::User));
        assert_eq!(assembled.messages[0].content, "explicit");
    }

    #[test]
    fn system() {
        // Precedence rule 2: `prompt.system` applies whether turns or sections.
        let mut prompt = prompt_from(vec![message(Role::User, "hi")], None);
        prompt.system = Some("be terse".to_owned());
        let assembled = PreparedPrompt::try_from(prompt).expect("try_from");
        assert_eq!(assembled.system.as_deref(), Some("be terse"));
        assert_eq!(assembled.messages.len(), 1);
        assert_eq!(assembled.messages[0].content, "hi");
    }

    #[test]
    fn assemble_sections() {
        // Precedence rule 3: try_from from sections; variables substitute into parts.
        let prompt = prompt_from(
            vec![],
            Some(Sections {
                role: Some("a {language} reviewer".to_owned()),
                task: "review the {language} code".to_owned(),
                context: None,
                constraints: vec!["be {language}-idiomatic".to_owned()],
                examples: vec![Example {
                    input: "in".to_owned(),
                    output: "out".to_owned(),
                }],
                variables: vec![Variable {
                    name: "language".to_owned(),
                    value: "Rust".to_owned(),
                }],
            }),
        );
        let assembled = PreparedPrompt::try_from(prompt).expect("try_from");
        let system = assembled.system.expect("system channel");
        assert!(system.contains("a Rust reviewer"));
        assert!(system.contains("- be Rust-idiomatic"));
        let user = &assembled.messages[0].content;
        assert!(user.contains("review the Rust code"));
        assert!(user.contains("Input: in\nOutput: out"));
    }

    // Build a prompt with the given turns and sections, defaults elsewhere.
    fn prompt_from(messages: Vec<Message>, sections: Option<Sections>) -> Prompt {
        Prompt {
            model: None,
            system: None,
            messages,
            sections,
            generation: None,
            format: Format::Json,
            tools: vec![],
            tool_choice: None,
            metadata: vec![],
            grants: Grants {
                references: None,
                workspace: None,
                verify: vec![],
            },
        }
    }

    fn sections(task: &str) -> Sections {
        Sections {
            role: None,
            task: task.to_owned(),
            context: None,
            constraints: vec![],
            examples: vec![],
            variables: vec![],
        }
    }

    fn message(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_owned(),
        }
    }
}
