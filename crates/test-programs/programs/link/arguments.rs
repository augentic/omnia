//! Exports `omnia-test:link/ops` and answers each call with the argv it sees,
//! so the suites can show that a store outside the command run gets none.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    world: "echoer",
    path: "wit",
});

struct Arguments;

export!(Arguments);

impl exports::omnia_test::link::ops::Guest for Arguments {
    fn ping(target: String, _message: String) -> String {
        argv(&target)
    }

    async fn ping_async(target: String, _message: String) -> String {
        argv(&target)
    }
}

fn argv(target: &str) -> String {
    format!("{target} argv: {:?}", wasip3::cli::environment::get_arguments())
}
