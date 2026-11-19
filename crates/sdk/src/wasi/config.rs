//! [`Config`] over `omnia:config`.

use omnia_wasi_config::omnia::store;

use crate::{Config, Result, WasiProvider};

impl Config for WasiProvider {
    async fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(store::get(key.to_owned()).await?)
    }
}
