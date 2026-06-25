//! Floor-side validation and message assembly.
//!
//! These are *host* concerns the `complete` binding applies at the boundary
//! (§3.4), never behaviour each backend re-implements:
//!
//! - reserved floor-tool-name collisions and empty prompts are rejected before a
//!   backend is called (§3.1.2, §3.1.1);
//! - the returned answer is structurally validated before the guest sees it
//!   (§3.1.3);
//! - [`assemble`] is the §3.1.1 precedence a backend uses to map the typed
//!   surface onto a provider chat request (replay does not need it; it is
//!   exercised by genai in Phase 2a and unit-tested here now).

use serde_json::Value;

use super::Error;
use super::types::{Message, Prompt, ResponseFormatKind};

/// Floor tool names that a guest must not redeclare in `prompt.tools` (§3.1.2).
pub const RESERVED_TOOL_NAMES: &[&str] = &["resolve", "read", "list", "write", "verify"];

/// The provider chat request assembled from a [`Prompt`] (§3.1.1). `system` is
/// always a separate channel from the turns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assembled {
    /// The system / instructions channel (already joined), if any.
    pub system: Option<String>,
    /// The chat turns to send to the provider.
    pub messages: Vec<Message>,
}

/// Floor pre-checks applied to every prompt before a backend is called.
///
/// # Errors
///
/// Returns `error::backend("reserved tool name")` if a guest-declared tool
/// collides with a reserved floor name, or `error::backend("empty prompt")` if
/// there is nothing to send (§3.1.1 rule 4).
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

/// Structurally validate a backend answer against `response-format.kind`
/// (§3.1.3). This is the floor's final gate, re-applied in the `complete`
/// binding even for backends that self-check.
///
/// # Errors
///
/// Returns `error::invalid-answer` when the value does not satisfy the gate for
/// its kind.
pub fn validate_answer(value: &Value, kind: ResponseFormatKind) -> Result<(), Error> {
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
            // Phase 1 gate is parse-only: the value is already valid JSON. Full
            // JSON-Schema enforcement is a tracked Phase 3 follow-up (§3.1.3) —
            // it takes a validator-crate dependency this floor crate does not yet
            // carry. TODO(phase-3): validate `value` against `json-schema.schema`.
            Ok(())
        }
    }
}

/// Assemble a [`Prompt`] into a provider chat request (§3.1.1).
///
/// The precedence: explicit turns beat templates, and `system` is always a
/// separate channel. Callers should run [`check_prompt`] first; this still
/// rejects an empty input defensively.
///
/// # Errors
///
/// Returns `error::backend("empty prompt")` if there is nothing to assemble.
pub fn assemble(prompt: &Prompt) -> Result<Assembled, Error> {
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
        user_parts.push(format!("Input: {}\nOutput: {}", subst(&example.input), subst(&example.output)));
    }

    let system = join_non_empty(&system_parts);
    let messages = vec![Message {
        role: "user".to_owned(),
        content: join_non_empty(&user_parts).unwrap_or_default(),
    }];

    Ok(Assembled { system, messages })
}

/// Substitute `{name}` placeholders with each variable's value.
fn substitute(text: &str, sections: &super::types::Sections) -> String {
    let mut out = text.to_owned();
    for variable in &sections.variables {
        out = out.replace(&format!("{{{}}}", variable.name), &variable.value);
    }
    out
}

/// Join non-empty parts with blank lines, yielding `None` when all are empty.
fn join_non_empty(parts: &[String]) -> Option<String> {
    let kept: Vec<&str> =
        parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).collect();
    if kept.is_empty() { None } else { Some(kept.join("\n\n")) }
}

/// Normalize an optional string to `None` when it is empty or whitespace.
fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Error, assemble, check_prompt, validate_answer};
    use crate::host::types::{
        Example, FunctionTool, Message, Prompt, ResponseFormat, ResponseFormatKind, Sections,
        ToolGrants, Variable,
    };

    /// Build a prompt with the given turns and sections, defaults elsewhere.
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
            grants: ToolGrants { references: None, working_tree_lent: false, verify: vec![] },
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
        Message { role: role.to_owned(), content: content.to_owned() }
    }

    #[test]
    fn text_gate_requires_a_json_string() {
        assert!(validate_answer(&json!("hi"), ResponseFormatKind::Text).is_ok());
        let err = validate_answer(&json!({ "a": 1 }), ResponseFormatKind::Text).unwrap_err();
        assert!(matches!(err, Error::InvalidAnswer(_)));
    }

    #[test]
    fn json_object_gate_requires_an_object() {
        assert!(validate_answer(&json!({ "verdict": "pass" }), ResponseFormatKind::JsonObject).is_ok());
        let err = validate_answer(&json!("nope"), ResponseFormatKind::JsonObject).unwrap_err();
        assert!(matches!(err, Error::InvalidAnswer(_)));
    }

    #[test]
    fn json_schema_gate_is_parse_only_in_phase_1() {
        // Any well-formed JSON value passes the Phase 1 (parse-only) schema gate.
        assert!(validate_answer(&json!({ "x": [1, 2, 3] }), ResponseFormatKind::JsonSchema).is_ok());
        assert!(validate_answer(&json!(42), ResponseFormatKind::JsonSchema).is_ok());
    }

    #[test]
    fn reserved_tool_name_is_rejected() {
        let mut prompt = prompt_with(vec![message("user", "hi")], None);
        prompt.tools.push(FunctionTool {
            name: "read".to_owned(),
            description: "shadow a floor tool".to_owned(),
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
        assert!(check_prompt(&prompt_with(vec![message("user", "hi")], None)).is_ok());
        assert!(check_prompt(&prompt_with(vec![], Some(sections("do it")))).is_ok());
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
