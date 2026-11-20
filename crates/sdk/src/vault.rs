//! `omnia:vault`, the lockers of secrets a deployment declares.
//!
//! A guest opens a [`Locker`] with [`open`], or the [`DEFAULT`] locker with
//! [`default`]. Beside the generated methods, a locker has forms that take
//! borrowed names and values:
//!
//! - [`read`](Locker::read), [`write`](Locker::write), and
//!   [`remove`](Locker::remove) a secret, and
//!   [`contains`](Locker::contains) tells whether one is stored.
//! - [`names`](Locker::names) lists every secret under a prefix, where the
//!   generated `list_secrets` lists one page.

use std::collections::BTreeSet;

use crate::Result;
pub use crate::bindings::imports::omnia::vault::{locker, store, types};
pub use crate::vault::locker::Locker;

/// The locker a guest opens when the deployment names no other.
pub const DEFAULT: &str = "default";

/// Open the locker `name`, a binding the deployment declares.
///
/// # Errors
///
/// The host's failure, under its code: `not-found` for a locker the
/// deployment does not declare.
pub async fn open(name: &str) -> Result<Locker> {
    Ok(store::open(name.to_owned()).await?)
}

/// Open the [`DEFAULT`] locker.
///
/// # Errors
///
/// As [`open`].
pub async fn default() -> Result<Locker> {
    open(DEFAULT).await
}

impl Locker {
    /// The value of the secret `name`, or `None` when it is absent.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.get(name.to_owned()).await?)
    }

    /// Whether the secret `name` is stored.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn contains(&self, name: &str) -> Result<bool> {
        Ok(self.exists(name.to_owned()).await?)
    }

    /// Store `value` as the secret `name`, replacing any value there.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn write(&self, name: &str, value: &[u8]) -> Result<()> {
        Ok(self.set(name.to_owned(), value.to_vec()).await?)
    }

    /// Delete the secret `name`. Deleting an absent secret succeeds.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code.
    pub async fn remove(&self, name: &str) -> Result<()> {
        Ok(self.delete(name.to_owned()).await?)
    }

    /// Every secret name that starts with `prefix`, sorted, each once.
    ///
    /// A secret set or deleted during the listing may or may not be in it.
    ///
    /// # Errors
    ///
    /// The host's failure, under its code, on any page.
    pub async fn names(&self, prefix: &str) -> Result<Vec<String>> {
        let mut names = BTreeSet::new();
        let mut after = None;
        loop {
            let page = self.list_secrets(prefix.to_owned(), after, None).await?;
            names.extend(page.secrets);
            match page.next {
                Some(next) => after = Some(next),
                None => return Ok(names.into_iter().collect()),
            }
        }
    }
}
