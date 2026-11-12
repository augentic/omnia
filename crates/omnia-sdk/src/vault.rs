//! Secret store wire types.

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
use bytes::Bytes;
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
use omnia_wasi_vault::vault;

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
use crate::SecretStore;
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
use crate::capability::{Error, ErrorKind, Page, Result};

/// The locker a provider's own [`SecretStore`](crate::SecretStore) handle uses.
pub const DEFAULT: &str = "default";

/// A [`SecretStore`](crate::SecretStore) handle on one locker of the WASI host.
///
/// [`SecretStore::named`](crate::SecretStore::named) on
/// [`WasiProvider`](crate::WasiProvider) returns one.
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
#[derive(Clone, Debug)]
pub struct Handle {
    name: String,
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl Handle {
    pub(crate) fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
        }
    }

    async fn open(&self) -> Result<vault::Locker> {
        Ok(vault::open(self.name.clone()).await?)
    }
}

// `omnia:vault` lists a locker in one call, so `keys` answers one page in key
// order and resumes after the key it is given
#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl SecretStore for Handle {
    type Named = Self;

    async fn get(&self, key: &str) -> Result<Option<Bytes>> {
        Ok(self.open().await?.get(key.to_owned()).await?.map(Bytes::from))
    }

    async fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        Ok(self.open().await?.set(key.to_owned(), value.to_vec()).await?)
    }

    async fn delete(&self, key: &str) -> Result<()> {
        Ok(self.open().await?.delete(key.to_owned()).await?)
    }

    async fn keys(&self, after: Option<&str>) -> Result<Page<String>> {
        let mut keys = self.open().await?.list_ids().await?;
        keys.retain(|key| after.is_none_or(|after| key.as_str() > after));
        keys.sort_unstable();
        Ok(Page::new(keys, None))
    }

    fn named(&self, name: &str) -> Self {
        Self::new(name)
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        Ok(self.open().await?.exists(key.to_owned()).await?)
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
impl From<vault::Error> for Error {
    fn from(err: vault::Error) -> Self {
        match err {
            vault::Error::NoSuchStore => Self::new(ErrorKind::NotFound, "no such locker"),
            vault::Error::AccessDenied => Self::new(ErrorKind::Denied, "access denied"),
            vault::Error::Other(message) => Self::new(ErrorKind::Other, message),
        }
    }
}
