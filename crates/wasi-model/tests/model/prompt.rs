use omnia_wasi_model::prompt::{Example, Sections};

#[test]
fn assemble_sections() {
    let sections = Sections {
        role: Some("a {language} reviewer".to_owned()),
        task: "review the {language} code".to_owned(),
        context: Some("the {language} crate".to_owned()),
        constraints: vec!["be {language}-idiomatic".to_owned()],
        examples: vec![Example {
            input: "in".to_owned(),
            output: "out".to_owned(),
        }],
        variables: vec![("language".to_owned(), "Rust".to_owned())],
    };
    // Preamble is not substituted; it leads the system channel.
    assert_eq!(
        sections.assemble(Some("prefer {language}")),
        (
            Some("prefer {language}\n\na Rust reviewer\n\n- be Rust-idiomatic".to_owned()),
            "review the Rust code\n\nthe Rust crate\n\nInput: in\nOutput: out".to_owned(),
        )
    );
}

#[test]
fn blank_parts_dropped() {
    let sections = Sections {
        role: Some("   ".to_owned()),
        task: "do it".to_owned(),
        context: Some(String::new()),
        ..Sections::default()
    };
    let (system, user) = sections.assemble(None);
    assert!(system.is_none());
    assert_eq!(user, "do it");
}
