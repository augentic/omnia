//! Process-environment and map default implementation of [`ConfigClient`].

use std::collections::HashMap;
use std::env;
use std::fmt::{Debug, Formatter};
use std::sync::Arc;

use anyhow::Result;
use futures::FutureExt as _;
use tracing::instrument;

use super::ConfigClient;
use crate::{Connect, FutureResult, NoOptions};

/// Default implementation for `wasi:config`, sourcing variables from the
/// process environment on [`Connect`] or from a seeded map.
#[derive(Clone, Default)]
pub struct ConfigDefault {
    vars: Arc<HashMap<String, String>>,
}

impl<K, V> FromIterator<(K, V)> for ConfigDefault
where
    K: Into<String>,
    V: Into<String>,
{
    fn from_iter<I: IntoIterator<Item = (K, V)>>(vars: I) -> Self {
        Self {
            vars: Arc::new(vars.into_iter().map(|(k, v)| (k.into(), v.into())).collect()),
        }
    }
}

impl Debug for ConfigDefault {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigDefault").finish_non_exhaustive()
    }
}

impl Connect for ConfigDefault {
    type ConnectOptions = NoOptions;

    #[instrument]
    async fn connect_with(_: Self::ConnectOptions) -> Result<Self> {
        Ok(Self {
            vars: Arc::new(env::vars().collect()),
        })
    }
}

impl ConfigClient for ConfigDefault {
    fn get(&self, key: String) -> FutureResult<Option<String>> {
        let value = self.vars.get(&key).cloned();
        async move { Ok(value) }.boxed()
    }

    fn get_all(&self) -> FutureResult<Vec<(String, String)>> {
        let mut pairs: Vec<(String, String)> =
            self.vars.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        async move { Ok(pairs) }.boxed()
    }
}
