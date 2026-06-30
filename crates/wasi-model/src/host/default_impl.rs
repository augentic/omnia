//! `ModelDefault` — the crate's default, deterministic (replay) backend (§5.4).
//!
//! It serves a pre-recorded answer for a given prompt.

use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use futures::FutureExt as _;
use omnia::Backend;
use tracing::instrument;

use crate::host::replay::FixtureStore;
use crate::host::types::{BackendAnswer, PreparedPrompt};
use crate::host::{FutureResult, ToolHost, WasiModelCtx};

/// Options used to connect the replay backend.
#[derive(Debug, Clone)]
pub struct ConnectOptions {
    /// Replay fixtures directory.
    pub replay_dir: PathBuf,
}

impl omnia::FromEnv for ConnectOptions {
    fn from_env() -> Result<Self> {
        let replay_dir = std::env::var_os("OMNIA_REPLAY_DIR")
            .map_or_else(|| PathBuf::from("fixtures"), PathBuf::from);
        Ok(Self { replay_dir })
    }
}

/// Default (replay) implementation of `wasi-model`.
#[derive(Clone, Debug)]
pub struct ModelDefault {
    store: Arc<FixtureStore>,
}

impl Backend for ModelDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        let store = FixtureStore::try_from(&options.replay_dir)?;
        tracing::debug!(
            dir = %options.replay_dir.display(),
            fixtures = store.len(),
            "initialized replay backend"
        );
        Ok(Self {
            store: Arc::new(store),
        })
    }
}

impl WasiModelCtx for ModelDefault {
    fn complete(
        &self, request: PreparedPrompt, _tool_host: Arc<dyn ToolHost>,
    ) -> FutureResult<BackendAnswer> {
        let answer = self.store.answer_for(&request);
        async move { answer }.boxed()
    }
}

#[cfg(test)]
mod tests {

    use anyhow::anyhow;
    use omnia::Backend;
    use serde_json::json;

    use super::*;
    use crate::host::generated::augentic::model::completion::{
        Prompt, ResponseFormat, ResponseFormatKind, Sections, ToolGrants,
    };
    use crate::host::replay::{Recording, record_fixture};
    use crate::host::types::{DirEntry, Reference, VerifyReport};
    use crate::host::{FutureResult, ToolHost, WasiModelCtx};

    /// A tool host stub for tests; replay never calls it.
    #[derive(Debug)]
    struct StubToolHost;

    impl ToolHost for StubToolHost {
        fn resolve(&self, _reference: Reference) -> FutureResult<Vec<u8>> {
            async { Err(anyhow!("stub")) }.boxed()
        }

        fn read(&self, _path: String) -> FutureResult<Vec<u8>> {
            async { Err(anyhow!("stub")) }.boxed()
        }

        fn list(&self, _path: String) -> FutureResult<Vec<DirEntry>> {
            async { Err(anyhow!("stub")) }.boxed()
        }

        fn write(&self, _path: String, _bytes: Vec<u8>) -> FutureResult<()> {
            async { Err(anyhow!("stub")) }.boxed()
        }

        fn verify(&self, _check: String) -> FutureResult<VerifyReport> {
            async { Err(anyhow!("stub")) }.boxed()
        }
    }

    /// A minimal prompt used across replay tests.
    fn sample_prompt() -> Prompt {
        Prompt {
            model: None,
            system: None,
            messages: vec![],
            sections: Some(Sections {
                role: Some("a terse judge".to_owned()),
                task: "decide pass or fail".to_owned(),
                context: Some("the candidate looks fine".to_owned()),
                constraints: vec![],
                examples: vec![],
                variables: vec![],
            }),
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
                workspace: None,
                verify: vec![],
            },
        }
    }

    #[tokio::test]
    async fn replay_fixture() {
        let dir = std::env::temp_dir().join(format!("omnia-model-replay-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let prompt = sample_prompt();
        let answer = BackendAnswer {
            value: json!({ "verdict": "pass" }),
            transcript: None,
        };
        let request = PreparedPrompt::assemble(prompt, false).expect("assemble");
        record_fixture(&dir, &request, &answer).expect("write fixture");

        let backend = ModelDefault::connect_with(ConnectOptions {
            replay_dir: dir.clone(),
        })
        .await
        .expect("connect");
        let replayed = backend
            .complete(request, Arc::new(StubToolHost))
            .await
            .expect("replay hits the fixture");
        assert_eq!(replayed.value, json!({ "verdict": "pass" }));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn missing_fixture() {
        let dir = std::env::temp_dir().join(format!("omnia-model-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let backend =
            ModelDefault::connect_with(ConnectOptions { replay_dir: dir }).await.expect("connect");
        let error = backend
            .complete(
                PreparedPrompt::assemble(sample_prompt(), false).expect("assemble"),
                Arc::new(StubToolHost),
            )
            .await
            .expect_err("no fixture should fail");
        assert!(error.to_string().contains("no replay fixture"));
    }

    #[tokio::test]
    async fn record_replay() {
        #[derive(Debug)]
        struct AlwaysOk;

        impl WasiModelCtx for AlwaysOk {
            fn complete(
                &self, _request: PreparedPrompt, _tool_host: Arc<dyn ToolHost>,
            ) -> FutureResult<BackendAnswer> {
                async move {
                    Ok(BackendAnswer {
                        value: json!({ "verdict": "fail" }),
                        transcript: None,
                    })
                }
                .boxed()
            }
        }

        let dir = std::env::temp_dir().join(format!("omnia-model-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        // The generated prompt is not `Clone`, so build a fresh (equivalent)
        // prompt for each leg; record and replay key on the same reduced shape.
        let recording = Recording::new(AlwaysOk, dir.clone());
        let recorded = recording
            .complete(
                PreparedPrompt::assemble(sample_prompt(), false).expect("assemble"),
                Arc::new(StubToolHost),
            )
            .await
            .expect("record");

        let backend = ModelDefault::connect_with(ConnectOptions {
            replay_dir: dir.clone(),
        })
        .await
        .expect("connect");
        let replayed = backend
            .complete(
                PreparedPrompt::assemble(sample_prompt(), false).expect("assemble"),
                Arc::new(StubToolHost),
            )
            .await
            .expect("replay");

        assert_eq!(recorded.value, replayed.value);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
