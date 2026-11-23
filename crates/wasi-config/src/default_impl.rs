//! Default implementation for `omnia:config`, serving the host variables
//! `CONFIG_KEYS` names.

use std::collections::BTreeMap;
use std::fmt::{self, Debug, Formatter};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use futures::{FutureExt as _, future};
use omnia_core::env::{self, PatternError};
use omnia_core::{Connect, FromEnv};
use tracing::instrument;

use crate::{ConfigClient, FutureResult};

/// The host variables the default `omnia:config` backend serves.
#[derive(Debug, Clone, FromEnv)]
pub struct ConnectOptions {
    /// The host variables a guest may read (`CONFIG_KEYS`, comma-separated, default: none).
    ///
    /// Each entry is a whole name, or a prefix that ends in `*`, as
    /// [`env::pattern`] checks. `*` alone serves every variable.
    #[env(from = "CONFIG_KEYS", default = "", with = keys)]
    pub keys: Vec<String>,
}

fn keys(value: &str) -> Result<Vec<String>, PatternError> {
    let Ok(entries) = env::list(value);
    entries.iter().map(|entry| env::pattern(entry)).collect()
}

/// The default `omnia:config` backend, answering from a map fixed when it is built.
///
/// - It serves the host variables [`ConnectOptions::keys`] matches, as they
///   stand when it connects, and skips a variable whose name or value is not
///   Unicode.
/// - [`connect`](Connect::connect) loads the keys from `CONFIG_KEYS`, and
///   serves none when it is unset. [`connect_with`](Connect::connect_with)
///   takes them in code.
/// - A [`get`](ConfigClient::get) for a host variable it does not serve
///   answers `None`, and the first one for each variable logs a warning.
/// - [`Default`] is empty, and [`FromIterator`] builds one from key-value
///   pairs.
/// - It lists every entry it serves through
///   [`entries`](ConfigClient::entries).
#[derive(Clone, Default)]
pub struct ConfigDefault {
    vars: Arc<BTreeMap<String, String>>,
    // the host variables it does not serve, each flagged once a read of it
    // has warned
    withheld: Arc<BTreeMap<String, AtomicBool>>,
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
        Self {
            vars: Arc::new(vars),
            withheld: Arc::default(),
        }
    }
}

impl Connect for ConfigDefault {
    type ConnectOptions = ConnectOptions;

    #[instrument]
    async fn connect_with(options: Self::ConnectOptions) -> Result<Self> {
        let mut vars = BTreeMap::new();
        let mut withheld = BTreeMap::new();
        for (key, value) in std::env::vars_os() {
            let Ok(key) = key.into_string() else { continue };
            if !options.keys.iter().any(|pattern| env::matches(pattern, &key)) {
                withheld.insert(key, AtomicBool::new(false));
            } else if let Ok(value) = value.into_string() {
                vars.insert(key, value);
            }
        }

        Ok(Self {
            vars: Arc::new(vars),
            withheld: Arc::new(withheld),
        })
    }
}

impl ConfigClient for ConfigDefault {
    fn get(&self, key: String) -> FutureResult<Option<String>> {
        let value = self.vars.get(&key).cloned();
        if let Some(warned) = self.withheld.get(&key)
            && !warned.swap(true, Ordering::Relaxed)
        {
            tracing::warn!(
                key,
                "a guest read a host variable outside `ConfigDefault`'s keys, so it reads as \
                 unset; the keys come from `CONFIG_KEYS` unless the runtime sets them in code"
            );
        }
        future::ready(Ok(value)).boxed()
    }

    fn entries(&self) -> FutureResult<Vec<(String, String)>> {
        let entries = self.vars.iter().map(|(key, value)| (key.clone(), value.clone())).collect();
        future::ready(Ok(entries)).boxed()
    }
}
