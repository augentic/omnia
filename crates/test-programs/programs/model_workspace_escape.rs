//! Raw bindings: a workspace grant whose subpath escapes the lent root is
//! refused at `create`.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_model::{completion, wit_stream};
use wasip3::exports::cli::run::Guest;
use wasip3::filesystem::preopens;

struct CliGuest;

wasip3::cli::command::export!(CliGuest);

impl Guest for CliGuest {
    async fn run() -> Result<(), ()> {
        let directories = preopens::get_directories();
        let root = directories
            .iter()
            .find(|(_, name)| name == ".")
            .map(|(dir, _)| dir)
            .expect("host mounts `.`");

        let request = completion::Request {
            model: None,
            system: None,
            messages: vec![completion::Message {
                role: completion::Role::User,
                content: "hi".to_owned(),
            }],
            generation: None,
            format: completion::Format::Text,
            tools: vec![],
            grants: completion::Grants {
                workspace: Some(completion::WorkspaceGrant {
                    root,
                    subpath: "../escape".to_owned(),
                }),
            },
        };

        let (results, results_rx) = wit_stream::new();
        let error =
            completion::create(request, results_rx).await.expect_err("escaping subpath refused");
        drop(results);
        assert!(
            matches!(error, completion::Error::Backend(ref detail) if detail.contains("not a plain relative path")),
            "unexpected: {error:?}"
        );
        Ok(())
    }
}
