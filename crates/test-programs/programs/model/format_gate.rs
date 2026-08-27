//! The answer gate's shape checks as the guest observes them: each request
//! carries a marker the scripted backend answers with a mis-shaped value.

#![cfg(target_arch = "wasm32")]

use omnia_guest::model::{Error, Format, Model as _, Request, SchemaFormat, WasiModel};
use test_programs::{REPORT_SCHEMA, user};

test_programs::command!(scenario);

async fn rejected(marker: &str, format: Format) -> String {
    let request = Request::builder().messages(vec![user(marker)]).format(format).build();
    match WasiModel.complete(request).await {
        Err(Error::InvalidAnswer(detail)) => detail,
        other => panic!("expected invalid-answer for `{marker}`, got {other:?}"),
    }
}

fn report() -> Format {
    Format::Schema(SchemaFormat::builder().name("report").schema(REPORT_SCHEMA).build())
}

async fn scenario() {
    assert!(rejected("object-for-text", Format::Text).await.contains("not a JSON string"));
    assert!(rejected("string-for-json", Format::Json).await.contains("not a JSON object"));

    let root = rejected("root-mismatch", report()).await;
    assert!(root.contains("does not conform to schema `report`"), "unexpected: {root}");
    assert!(root.contains("at root"), "unexpected: {root}");

    let nested = rejected("nested-mismatch", report()).await;
    assert!(nested.contains("/ui-surface"), "unexpected: {nested}");
    assert_ne!(root, nested);
}
