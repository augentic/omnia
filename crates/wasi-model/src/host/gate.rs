//! Host-side validation and message assembly for the `complete` binding.

use serde_json::Value;

use super::Error;
use super::types::{Message, PreparedPrompt, Prompt, ResponseFormatKind};

/// Host-injected tool names guests must not redeclare in `prompt.tools`.
pub const RESERVED_TOOL_NAMES: &[&str] = &["resolve", "read", "list", "write", "verify"];

/// Provider chat request assembled from a [`Prompt`]; `system` is separate from turns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assembled {
    /// System/instructions channel (already joined), if any.
    pub system: Option<String>,
    /// Chat turns to send to the provider.
    pub messages: Vec<Message>,
}

/// Pre-check every prompt before calling a backend.
pub fn check_prompt(prompt: &Prompt) -> Result<(), Error> {
    if let Some(tool) = prompt.tools.iter().find(|t| RESERVED_TOOL_NAMES.contains(&t.name.as_str()))
    {
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
pub fn check_answer(value: &Value, kind: ResponseFormatKind) -> Result<(), Error> {
    match kind {
        ResponseFormatKind::Text => {
            if value.is_string() {
                Ok(())
            } else {
                Err(Error::InvalidAnswer("answer is not a JSON string".to_owned()))
            }
        }
        ResponseFormatKind::JsonObject => {
            if value.is_object() {
                Ok(())
            } else {
                Err(Error::InvalidAnswer("answer is not a JSON object".to_owned()))
            }
        }
        ResponseFormatKind::JsonSchema => {
            // TODO: validate against `json-schema.schema`.
            Ok(())
        }
    }
}

// Assemble a [`Prompt`] into a provider chat request.
fn assemble(prompt: &Prompt) -> Result<Assembled, Error> {
    // 1. `messages` wins over `sections`. 2. `prompt.system` is always applied.
    if !prompt.messages.is_empty() {
        return Ok(Assembled {
            system: non_empty(prompt.system.clone()),
            messages: prompt.messages.clone(),
        });
    }

    // 3. Assemble from `sections` only when `messages` is empty.
    let Some(sections) = &prompt.sections else {
        return Err(Error::Backend("empty prompt".to_owned()));
    };
    if sections.task.trim().is_empty() {
        return Err(Error::Backend("empty prompt".to_owned()));
    }

    let subst = |text: &str| substitute(text, sections);

    // System channel: prompt.system, sections.role, formatted constraints.
    let mut system_parts: Vec<String> = Vec::new();
    if let Some(system) = &prompt.system {
        system_parts.push(system.clone());
    }
    if let Some(role) = &sections.role {
        system_parts.push(subst(role));
    }
    for constraint in &sections.constraints {
        system_parts.push(format!("- {}", subst(constraint)));
    }

    // User channel: task, context, formatted examples (variables substituted).
    let mut user_parts: Vec<String> = vec![subst(&sections.task)];
    if let Some(context) = &sections.context {
        user_parts.push(subst(context));
    }
    for example in &sections.examples {
        user_parts.push(format!(
            "Input: {}\nOutput: {}",
            subst(&example.input),
            subst(&example.output)
        ));
    }

    let system = join_non_empty(&system_parts);
    let messages = vec![Message {
        role: "user".to_owned(),
        content: join_non_empty(&user_parts).unwrap_or_default(),
    }];

    Ok(Assembled { system, messages })
}

impl TryFrom<Prompt> for PreparedPrompt {
    type Error = Error;

    /// Run the pre-call checks and assemble the provider chat channels for
    /// `value`, the single assembly authority the `complete` gate and tests share.
    fn try_from(value: Prompt) -> Result<Self, Error> {
        check_prompt(&value)?;
        let Assembled { system, messages } = assemble(&value)?;
        Ok(Self {
            prompt: value,
            system,
            messages,
        })
    }
}

// Substitute `{name}` placeholders with each variable's value.
fn substitute(text: &str, sections: &super::types::Sections) -> String {
    let mut out = text.to_owned();
    for variable in &sections.variables {
        out = out.replace(&format!("{{{}}}", variable.name), &variable.value);
    }
    out
}

// Join non-empty parts with blank lines; None when all are empty.
fn join_non_empty(parts: &[String]) -> Option<String> {
    let kept: Vec<&str> = parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).collect();
    if kept.is_empty() { None } else { Some(kept.join("\n\n")) }
}

// Normalize an optional string to None when empty or whitespace.
fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Error, assemble, check_answer, check_prompt};
    use crate::host::types::{
        Example, FunctionTool, Message, Prompt, ResponseFormat, ResponseFormatKind, Sections,
        ToolGrants, Variable,
    };

    // Build a prompt with the given turns and sections, defaults elsewhere.
    fn prompt_with(messages: Vec<Message>, sections: Option<Sections>) -> Prompt {
        Prompt {
            model: None,
            system: None,
            messages,
            sections,
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
                workspace_lent: false,
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

    fn message(role: &str, content: &str) -> Message {
        Message {
            role: role.to_owned(),
            content: content.to_owned(),
        }
    }

    #[test]
    fn text_gate_requires_a_json_string() {
        check_answer(&json!("hi"), ResponseFormatKind::Text).unwrap();
        let err = check_answer(&json!({ "a": 1 }), ResponseFormatKind::Text).unwrap_err();
        assert!(matches!(err, Error::InvalidAnswer(_)));
    }

    #[test]
    fn json_object_gate_requires_an_object() {
        check_answer(&json!({ "verdict": "pass" }), ResponseFormatKind::JsonObject).unwrap();
        let err = check_answer(&json!("nope"), ResponseFormatKind::JsonObject).unwrap_err();
        assert!(matches!(err, Error::InvalidAnswer(_)));
    }

    #[test]
    fn json_schema_gate_is_parse_only_in_phase_1() {
        // Any well-formed JSON value passes the Phase 1 (parse-only) schema gate.
        check_answer(&json!({ "x": [1, 2, 3] }), ResponseFormatKind::JsonSchema).unwrap();
        check_answer(&json!(42), ResponseFormatKind::JsonSchema).unwrap();
    }

    #[test]
    fn reserved_tool_name_is_rejected() {
        let mut prompt = prompt_with(vec![message("user", "hi")], None);
        prompt.tools.push(FunctionTool {
            name: "read".to_owned(),
            description: "shadow a host-injected tool".to_owned(),
            parameters: "{}".to_owned(),
        });
        let err = check_prompt(&prompt).unwrap_err();
        assert!(matches!(err, Error::Backend(m) if m.contains("reserved tool name")));
    }

    #[test]
    fn empty_prompt_is_rejected() {
        let err = check_prompt(&prompt_with(vec![], None)).unwrap_err();
        assert!(matches!(err, Error::Backend(m) if m == "empty prompt"));

        // sections present but task blank is still empty.
        let err = check_prompt(&prompt_with(vec![], Some(sections("   ")))).unwrap_err();
        assert!(matches!(err, Error::Backend(m) if m == "empty prompt"));
    }

    #[test]
    fn non_empty_prompt_passes_checks() {
        check_prompt(&prompt_with(vec![message("user", "hi")], None)).unwrap();
        check_prompt(&prompt_with(vec![], Some(sections("do it")))).unwrap();
    }

    #[test]
    fn explicit_messages_win_over_sections() {
        // Precedence rule 1: when `messages` is non-empty, `sections` is ignored.
        let prompt = prompt_with(vec![message("user", "explicit")], Some(sections("ignored")));
        let assembled = assemble(&prompt).expect("assemble");
        assert_eq!(assembled.messages, vec![message("user", "explicit")]);
    }

    #[test]
    fn system_is_always_a_separate_channel() {
        // Precedence rule 2: `prompt.system` applies whether turns or sections.
        let mut prompt = prompt_with(vec![message("user", "hi")], None);
        prompt.system = Some("be terse".to_owned());
        let assembled = assemble(&prompt).expect("assemble");
        assert_eq!(assembled.system.as_deref(), Some("be terse"));
        assert_eq!(assembled.messages, vec![message("user", "hi")]);
    }

    #[test]
    fn sections_assemble_with_variable_substitution() {
        // Precedence rule 3: assemble from sections; variables substitute into parts.
        let prompt = prompt_with(
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
        let assembled = assemble(&prompt).expect("assemble");
        let system = assembled.system.expect("system channel");
        assert!(system.contains("a Rust reviewer"));
        assert!(system.contains("- be Rust-idiomatic"));
        let user = &assembled.messages[0].content;
        assert!(user.contains("review the Rust code"));
        assert!(user.contains("Input: in\nOutput: out"));
    }
}
