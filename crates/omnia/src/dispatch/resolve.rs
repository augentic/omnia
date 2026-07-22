//! Resolve-on-miss: the deployment-supplied [`GuestResolver`] seam.
//!
//! A registry miss on a dispatch path may consult a resolver to *fault the
//! component in*: resolve → register (through the ordinary registration
//! internals, so serve-before-publish, allow-list bounds, and static-wins all
//! hold) → retry the lookup. Resolution is single-flight per identity — every
//! concurrent waiter shares one resolve outcome, negatives included — and no
//! negative outcome is cached across flights: a component installed between
//! two calls becomes dispatchable on the next.

use std::fmt;
use std::sync::Arc;

use crate::deployment::GuestArtifact;
use crate::host::FutureResult;
use crate::registry::GuestId;

/// Supplies components for identities the registry does not hold, consulted
/// on a dispatch-path registry miss (resolve-on-miss).
///
/// A resolver is deployment *code*, not configuration — supply it through
/// [`DeploymentBuilder::resolver`](crate::DeploymentBuilder::resolver) (or
/// [`Runtime::with_resolver`](crate::Runtime::with_resolver) when assembling
/// from parts). Verification (digest, signature, provenance) is deployment
/// policy and happens inside `resolve`, before the runtime sees the bytes.
pub trait GuestResolver: Send + Sync + 'static {
    /// Look up a component for `guest`.
    ///
    /// `expected_export` is the dispatch site's required export interface
    /// (e.g. `wasi:http/handler`, version-tolerant); the returned component
    /// is validated against it after load — the resolver's answer is not
    /// trusted to be well-shaped.
    ///
    /// Outcomes: `Ok(Some(artifact))` supplies the component; `Ok(None)` is
    /// the definitive miss — the resolver has no component for this identity,
    /// a valid answer rather than a fault; `Err` means resolution *failed*
    /// (fetch error, failed verification) and the answer is unknown. Neither
    /// negative outcome is cached.
    fn resolve(
        &self, guest: GuestId, expected_export: String,
    ) -> FutureResult<Option<GuestArtifact>>;
}

/// Maps an unrouted HTTP request path to a guest identity, consulted when no
/// static `[[route.http]]` prefix matches (the same shape axum gives
/// `Router::fallback`).
///
/// The returned identity goes through the ordinary registry lookup — and
/// hence resolve-on-miss when a [`GuestResolver`] is installed. Like the
/// resolver, a fallback is deployment code, supplied through
/// [`DeploymentBuilder::http_fallback`](crate::DeploymentBuilder::http_fallback).
pub type HttpFallback = Arc<dyn Fn(&str) -> Option<GuestId> + Send + Sync>;

/// Why [`Runtime::ensure_guest`](crate::Runtime::ensure_guest) could not
/// produce a registered guest.
///
/// Typed (rather than folded into `anyhow`) so trigger servers can map the
/// outcomes faithfully — e.g. HTTP answers [`Unresolved`](Self::Unresolved)
/// with 404 (unknown tenant) and the failure variants with 500.
#[derive(Clone, Debug)]
pub enum EnsureError {
    /// The guest is not registered and nothing supplied it: no resolver is
    /// installed, or the resolver answered `Ok(None)`.
    Unresolved(GuestId),
    /// The resolver — or the registration of its artifact — failed.
    ResolveFailed(Arc<anyhow::Error>),
    /// A registered component does not export the interface the dispatch
    /// site requires.
    ExportMismatch {
        /// The guest whose component was checked.
        guest: GuestId,
        /// The export interface the dispatch site expected.
        export: String,
    },
}

impl fmt::Display for EnsureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unresolved(guest) => write!(f, "guest `{guest}` is not registered"),
            Self::ResolveFailed(_) => write!(f, "guest resolution failed"),
            Self::ExportMismatch { guest, export } => {
                write!(f, "guest `{guest}` does not export interface `{export}`")
            }
        }
    }
}

impl std::error::Error for EnsureError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ResolveFailed(source) => {
                let source: &(dyn std::error::Error + 'static) = (**source).as_ref();
                Some(source)
            }
            Self::Unresolved(_) | Self::ExportMismatch { .. } => None,
        }
    }
}

/// Type-erased resolve-on-miss hook installed on the dispatch handle, so the
/// link path (which never touches `Registry::get`) can fault a target in.
///
/// Implemented over the runtime (holding a weak back-reference to avoid a
/// reference cycle through the registry's dispatch handle).
pub trait ResolveHook: Send + Sync + 'static {
    /// Ensure `guest` is registered, resolving it on a miss; `expected_export`
    /// is the link interface the dispatch requires.
    fn ensure(&self, guest: &GuestId, expected_export: &str) -> FutureResult<()>;
}
