//! Default implementation for `omnia:config`, serving the host variables
//! `CONFIG_KEYS` names.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt::{self, Debug, Formatter};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use futures::{FutureExt as _, future};
use omnia_core::Connect;
use parking_lot::Mutex;
use tracing::instrument;

use crate::{ConfigClient, FutureResult};

#[expect(missing_docs, reason = "`FromEnv` has no docs")]
mod config {
    use fromenv::{FromEnv, ParseResult};

    /// Options used to connect the default `omnia:config` backend.
    ///
    /// This struct is used to load connection options from environment variables.
    #[derive(Debug, Clone, Default, FromEnv)]
    pub struct ConnectOptions {
        /// The host variables a guest may read (`CONFIG_KEYS`, comma-separated, default: none).
        #[env(from = "CONFIG_KEYS", default = "", with = comma_separated)]
        pub keys: Vec<String>,
    }

    #[expect(clippy::unnecessary_wraps, reason = "the signature `fromenv` parsers take")]
    fn comma_separated(value: &str) -> ParseResult<Vec<String>> {
        Ok(value
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect())
    }
}
pub use config::ConnectOptions;

impl omnia_core::FromEnv for ConnectOptions {
    fn load_env() -> Result<Self> {
        // `Self::from_env()` is the builder the `FromEnv` derive emits
        Self::from_env().finalize().context("issue loading connection options")
    }
}

/// The default `omnia:config` backend, answering from a map fixed when it is built.
///
/// - [`connect`](Connect::connect) serves the host variables
///   [`ConnectOptions::keys`] names, as they stand when it connects. It
///   serves none when `CONFIG_KEYS` is unset, and skips a variable whose name
///   or value is not Unicode.
/// - A [`get`](ConfigClient::get) for a host variable it does not serve
///   answers `None`, and the first one for each variable logs a warning
///   naming `CONFIG_KEYS`.
/// - [`Default`] is empty, and [`FromIterator`] builds one from key-value
///   pairs.
/// - It lists every entry it serves through
///   [`entries`](ConfigClient::entries).
#[derive(Clone, Default)]
pub struct ConfigDefault {
    vars: Arc<BTreeMap<String, String>>,
    // the names of the host variables it does not serve, each until a guest
    // has asked for it once
    withheld: Arc<Mutex<BTreeSet<String>>>,
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
        let keys: BTreeSet<String> = options.keys.into_iter().collect();
        let mut vars = BTreeMap::new();
        let mut withheld = BTreeSet::new();
        for (key, value) in env::vars_os() {
            let Ok(key) = key.into_string() else { continue };
            if !keys.contains(&key) {
                withheld.insert(key);
            } else if let Ok(value) = value.into_string() {
                vars.insert(key, value);
            }
        }

        Ok(Self {
            vars: Arc::new(vars),
            withheld: Arc::new(Mutex::new(withheld)),
        })
    }
}

impl ConfigClient for ConfigDefault {
    fn get(&self, key: String) -> FutureResult<Option<String>> {
        let value = self.vars.get(&key).cloned();
        if value.is_none() && self.withheld.lock().remove(&key) {
            tracing::warn!(
                key,
                "a guest read a host variable `CONFIG_KEYS` does not name, and it reads as unset"
            );
        }
        future::ready(Ok(value)).boxed()
    }

    fn entries(&self) -> FutureResult<Vec<(String, String)>> {
        let entries = self.vars.iter().map(|(key, value)| (key.clone(), value.clone())).collect();
        future::ready(Ok(entries)).boxed()
    }
}
