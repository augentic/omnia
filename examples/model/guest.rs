//! # Model example — `create` guest
//!
//! A `wasi:cli/command` reactor that **imports** `omnia:model/completion` and
//! calls `create` once when the host drives `wasi:cli/run`. It builds a
//! `json-schema` prompt, assembling the `system` / `messages` channels with the
//! guest-side `Sections` builder. It declares no HTTP/messaging
//! trigger, so it is driven by the seam suite
//! (`crates/seam-suite/tests/seam/model.rs`) rather than a live request — the
//! replay acceptance vehicle.
//!
//! It also reads `wasi:filesystem/preopens` and, when the host has mounted a
//! workspace named `.` (the `[[mount]]` in `omnia.toml`), lends it
//! through `grants.workspace`. With no mount configured the preopen table is
//! empty and the guest lends nothing.

#![cfg(target_arch = "wasm32")]

use omnia_wasi_model::completion;
use omnia_wasi_model::prompt::Sections;
use wasip3::exports::cli::run::Guest;
use wasip3::filesystem::preopens;

struct CliGuest;

wasip3::cli::command::export!(CliGuest);

impl Guest for CliGuest {
    async fn run() -> Result<(), ()> {
        // Read the preopen table the host populated from `[[mount]]` and
        // pick the tree named `.` to lend. `directories` must outlive the
        // `create` call below — the lent `workspace` borrows one of its
        // descriptors.
        let directories = preopens::get_directories();
        let workspace = directories.iter().find_map(|(dir, name)| (name == ".").then_some(dir));

        let (system, messages) = Sections {
            role: Some("a terse code reviewer".to_string()),
            task: "decide whether the change is acceptable".to_string(),
            context: Some("the diff adds a bounds check".to_string()),
            ..Sections::default()
        }
        .channels(None);

        let request = completion::Request {
            model: None,
            system,
            messages,
            generation: None,
            format: completion::Format::Schema(completion::Schema {
                name: "verdict".to_string(),
                schema: "{\"type\":\"object\"}".to_string(),
            }),
            tools: vec![],
            grants: completion::Grants {
                references: Some("shelf".to_string()),
                workspace,
                verify: vec![],
            },
        };

        let answer = match completion::create(request).await {
            Ok(reply) => reply.answer,
            Err(error) => format!("error: {error:?}"),
        };

        println!("{answer}");
        Ok(())
    }
}
