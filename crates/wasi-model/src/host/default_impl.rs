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
use crate::host::types::{Answer, PreparedPrompt};
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
    ) -> FutureResult<Answer> {
        let answer = self.store.answer_for(&request);
        async move { answer }.boxed()
    }
}
