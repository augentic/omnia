use omnia_wasi_model::completion::Role;
use omnia_wasi_model::prompt::Sections;

#[test]
fn channels_user_turn() {
    let sections = Sections {
        task: "do it".to_owned(),
        ..Sections::default()
    };
    let (system, messages) = sections.channels(None);
    assert!(system.is_none());
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, Role::User);
    assert_eq!(messages[0].content, "do it");
}
