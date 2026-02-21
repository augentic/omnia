//! Default no-op implementation for wasi-otel
//!
//! This is a lightweight implementation for development use only.
//! It logs telemetry data but doesn't export it anywhere.
//! For production use, use the `be-opentelemetry` backend.

#![allow(clippy::used_underscore_binding)]

use std::env;
use std::fmt::{Debug, Formatter};
use std::sync::Arc;

use anyhow::Result;
use omnia::Backend;
use tracing::instrument;
use wasmtime_wasi_config::WasiConfigVariables;

use crate::WasiConfigCtx;

#[derive(Debug, Clone, Default)]
pub struct ConnectOptions;

impl omnia::FromEnv for ConnectOptions {
    fn from_env() -> Result<Self> {
        Ok(Self)
    }
}

/// Default implementation for `wasi:config`.
#[derive(Clone)]
pub struct ConfigDefault {
    /// The configuration variables.
    pub config_vars: Arc<WasiConfigVariables>,
}

impl Debug for ConfigDefault {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "WasiConfigCtxImpl")
    }
}

impl Backend for ConfigDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(_: Self::ConnectOptions) -> Result<Self> {
        let config_vars = env::vars().collect();

        Ok(Self {
            config_vars: Arc::new(config_vars),
        })
    }
}

impl WasiConfigCtx for ConfigDefault {
    fn get_config(&self) -> &WasiConfigVariables {
        &self.config_vars
    }
}
