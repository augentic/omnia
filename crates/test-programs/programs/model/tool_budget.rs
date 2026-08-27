//! The host's tool-call budget fails the completion, not the guest handler.

#![cfg(target_arch = "wasm32")]

use omnia_guest::model::{Error, Model as _, Request, WasiModel};
use test_programs::{lookup, user};

test_programs::run!(scenario);

async fn scenario() {
    let request = Request::builder().messages(vec![user("hi")]).tools(vec![lookup()]).build();

    let mut seen = 0_u32;
    let error = WasiModel
        .complete_with(request, |_call| {
            seen += 1;
            async { Ok::<_, String>("42".to_owned()) }
        })
        .await
        .expect_err("the second call exceeds the budget");

    assert!(
        matches!(error, Error::BudgetExhausted(ref detail) if detail.contains("budget of 1 exhausted")),
        "unexpected: {error:?}"
    );
    assert_eq!(seen, 1, "only the budgeted call reaches the guest");
}
