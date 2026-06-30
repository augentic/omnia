//! Host-side validation and message assembly for the `complete` binding.

use serde_json::Value;

use super::Error;
use super::generated::augentic::model::completion as genc;
use super::types::PreparedPrompt;

const TOOL_NAMES: &[&str] = &["resolve", "read", "list", "write", "verify"];

/// Pre-check every prompt before calling a backend.
pub fn check_prompt(prompt: &genc::Prompt) -> Result<(), Error> {
    if let Some(tool) = prompt.tools.iter().find(|t| TOOL_NAMES.contains(&t.name.as_str())) {
        return Err(Error::Backend(format!("reserved tool name: {}", tool.name)));
    }

    if prompt.messages.is_empty()
        && prompt.sections.as_ref().is_none_or(|s| s.task.trim().is_empty())
    {
        return Err(Error::Backend("empty prompt".to_owned()));
    }

    Ok(())
}

// Validate a backend answer against `response-format.kind`.
pub fn check_answer(value: &Value, kind: genc::ResponseFormatKind) -> Result<(), Error> {
    match kind {
        genc::ResponseFormatKind::Text => {
            if !value.is_string() {
                return Err(Error::InvalidAnswer("answer is not a JSON string".to_owned()));
            }
            Ok(())
        }
        genc::ResponseFormatKind::JsonObject => {
            if !value.is_object() {
                return Err(Error::InvalidAnswer("answer is not a JSON object".to_owned()));
            }
            Ok(())
        }
        genc::ResponseFormatKind::JsonSchema => {
            // TODO: validate against `json-schema.schema`.
            Ok(())
        }
    }
}

impl PreparedPrompt {
    /// Validate `prompt` and assemble its provider chat channels (§3.1.1).
    ///
    /// `workspace_lent` records whether a workspace borrow was lent for this
    /// call; the host captures it before taking the borrow out of `grants`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Backend`] when the prompt is empty or declares a tool
    /// whose name collides with a reserved host-injected tool.
    pub fn assemble(prompt: genc::Prompt, workspace_lent: bool) -> Result<Self, Error> {
        // `messages` wins over `sections`. `prompt.system` is always applied.
        if !prompt.messages.is_empty() {
            let system = prompt.system.clone().filter(|v| !v.is_empty());
            let messages = prompt.messages.clone();

            return Ok(Self {
                prompt,
                workspace_lent,
                system,
                messages,
            });
        }

        check_prompt(&prompt)?;

        // assemble from `sections` when `messages` is empty.
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
        let messages = vec![genc::Message {
            role: "user".to_owned(),
            content: join_non_empty(&user_parts).unwrap_or_default(),
        }];

        Ok(Self {
            prompt,
            workspace_lent,
            system,
            messages,
        })
    }
}

fn join_non_empty(parts: &[String]) -> Option<String> {
    let kept: Vec<&str> = parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).collect();
    if kept.is_empty() { None } else { Some(kept.join("\n\n")) }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Error, PreparedPrompt, check_answer, check_prompt, genc};

    #[test]
    fn json_string() {
        check_answer(&json!("hi"), genc::ResponseFormatKind::Text).unwrap();
        let err = check_answer(&json!({ "a": 1 }), genc::ResponseFormatKind::Text).unwrap_err();
        assert!(matches!(err, Error::InvalidAnswer(_)));
    }

    #[test]
    fn json_object() {
        check_answer(&json!({ "verdict": "pass" }), genc::ResponseFormatKind::JsonObject).unwrap();
        let err = check_answer(&json!("nope"), genc::ResponseFormatKind::JsonObject).unwrap_err();
        assert!(matches!(err, Error::InvalidAnswer(_)));
    }

    #[test]
    fn json_schema() {
        // Any well-formed JSON value passes the Phase 1 (parse-only) schema gate.
        check_answer(&json!({ "x": [1, 2, 3] }), genc::ResponseFormatKind::JsonSchema).unwrap();
        check_answer(&json!(42), genc::ResponseFormatKind::JsonSchema).unwrap();
    }

    #[test]
    fn reserved_tool_name() {
        let mut prompt = prompt_from(vec![message("user", "hi")], None);
        prompt.tools.push(genc::FunctionTool {
            name: "read".to_owned(),
            description: "shadow a host-injected tool".to_owned(),
            parameters: "{}".to_owned(),
        });
        let err = check_prompt(&prompt).unwrap_err();
        assert!(matches!(err, Error::Backend(m) if m.contains("reserved tool name")));
    }

    #[test]
    fn empty_prompt() {
        let err = check_prompt(&prompt_from(vec![], None)).unwrap_err();
        assert!(matches!(err, Error::Backend(m) if m == "empty prompt"));

        // sections present but task blank is still empty.
        let err = check_prompt(&prompt_from(vec![], Some(sections("   ")))).unwrap_err();
        assert!(matches!(err, Error::Backend(m) if m == "empty prompt"));
    }

    #[test]
    fn non_empty() {
        check_prompt(&prompt_from(vec![message("user", "hi")], None)).unwrap();
        check_prompt(&prompt_from(vec![], Some(sections("do it")))).unwrap();
    }

    #[test]
    fn explicit_messages() {
        // Precedence rule 1: when `messages` is non-empty, `sections` is ignored.
        let prompt = prompt_from(vec![message("user", "explicit")], Some(sections("ignored")));
        let assembled = PreparedPrompt::assemble(prompt, false).expect("assemble");
        assert_eq!(assembled.messages.len(), 1);
        assert_eq!(assembled.messages[0].role, "user");
        assert_eq!(assembled.messages[0].content, "explicit");
    }

    #[test]
    fn system() {
        // Precedence rule 2: `prompt.system` applies whether turns or sections.
        let mut prompt = prompt_from(vec![message("user", "hi")], None);
        prompt.system = Some("be terse".to_owned());
        let assembled = PreparedPrompt::assemble(prompt, false).expect("assemble");
        assert_eq!(assembled.system.as_deref(), Some("be terse"));
        assert_eq!(assembled.messages.len(), 1);
        assert_eq!(assembled.messages[0].content, "hi");
    }

    #[test]
    fn assemble_sections() {
        // Precedence rule 3: assemble from sections; variables substitute into parts.
        let prompt = prompt_from(
            vec![],
            Some(genc::Sections {
                role: Some("a {language} reviewer".to_owned()),
                task: "review the {language} code".to_owned(),
                context: None,
                constraints: vec!["be {language}-idiomatic".to_owned()],
                examples: vec![genc::Example {
                    input: "in".to_owned(),
                    output: "out".to_owned(),
                }],
                variables: vec![genc::Variable {
                    name: "language".to_owned(),
                    value: "Rust".to_owned(),
                }],
            }),
        );
        let assembled = PreparedPrompt::assemble(prompt, false).expect("assemble");
        let system = assembled.system.expect("system channel");
        assert!(system.contains("a Rust reviewer"));
        assert!(system.contains("- be Rust-idiomatic"));
        let user = &assembled.messages[0].content;
        assert!(user.contains("review the Rust code"));
        assert!(user.contains("Input: in\nOutput: out"));
    }

    // Build a prompt with the given turns and sections, defaults elsewhere.
    fn prompt_from(messages: Vec<genc::Message>, sections: Option<genc::Sections>) -> genc::Prompt {
        genc::Prompt {
            model: None,
            system: None,
            messages,
            sections,
            generation: None,
            response_format: genc::ResponseFormat {
                kind: genc::ResponseFormatKind::JsonObject,
                json_schema: None,
            },
            tools: vec![],
            tool_choice: None,
            metadata: vec![],
            grants: genc::ToolGrants {
                references: None,
                workspace: None,
                verify: vec![],
            },
        }
    }

    fn sections(task: &str) -> genc::Sections {
        genc::Sections {
            role: None,
            task: task.to_owned(),
            context: None,
            constraints: vec![],
            examples: vec![],
            variables: vec![],
        }
    }

    fn message(role: &str, content: &str) -> genc::Message {
        genc::Message {
            role: role.to_owned(),
            content: content.to_owned(),
        }
    }
}
