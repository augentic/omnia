//! The registry seam that fetches a package source.

use futures::future::BoxFuture;

use crate::error::LoadError;

/// Registry acquisition policy — how a [`SourceSpec::Package`] source is
/// fetched.
///
/// [`SourceSpec::Package`]: omnia_core::SourceSpec::Package
pub trait RegistrySource: Send + Sync + 'static {
    /// Produce the raw component bytes for the exact `package` reference,
    /// split by remedy: [`LoadError::Refused`] for an authoritative "no" (no
    /// registry routes it, the registry has no such release), never for a
    /// source failure a retry might clear ([`LoadError::Unavailable`]).
    fn acquire<'a>(&'a self, package: &'a str) -> BoxFuture<'a, Result<Vec<u8>, LoadError>>;
}
