//! Default implementation for `omnia:config`, sourcing variables from the
//! process environment.

use std::collections::BTreeMap;
use std::env;
use std::fmt::{self, Debug, Formatter};
use std::sync::Arc;

use anyhow::Result;
use futures::{FutureExt as _, future};
use omnia_core::Connect;
use tracing::instrument;

use crate::{ConfigClient, FutureResult};

/// The default `omnia:config` backend, answering from a map fixed when it is built.
///
/// - [`connect`](Connect::connect) takes a snapshot of the process
///   environment, and skips a variable whose name or value is not Unicode.
/// - [`Default`] is empty, and [`FromIterator`] builds one from key-value
///   pairs.
/// - It lists every entry it holds through
///   [`entries`](ConfigClient::entries).
#[derive(Clone, Default)]
pub struct ConfigDefault {
    vars: Arc<BTreeMap<String, String>>,
}

// the environment carries credentials, so `Debug` prints no value
impl Debug for ConfigDefault {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("ConfigDefault")
    }
}

impl<K: Into<String>, V: Into<String>> FromIterator<(K, V)> for ConfigDefault {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(vars: I) -> Self {
        let vars = vars.into_iter().map(|(key, value)| (key.into(), value.into())).collect();
        Self { vars: Arc::new(vars) }
    }
}

impl Connect for ConfigDefault {
    type ConnectOptions = omnia_core::NoOptions;

    #[instrument]
    async fn connect_with(_: Self::ConnectOptions) -> Result<Self> {
        let vars = env::vars_os()
            .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)));
        Ok(vars.collect())
    }
}

impl ConfigClient for ConfigDefault {
    fn get(&self, key: String) -> FutureResult<Option<String>> {
        future::ready(Ok(self.vars.get(&key).cloned())).boxed()
    }

    fn entries(&self) -> FutureResult<Vec<(String, String)>> {
        let entries = self.vars.iter().map(|(key, value)| (key.clone(), value.clone())).collect();
        future::ready(Ok(entries)).boxed()
    }
}
