//! The guest's `wasi:cli/run` export.

use std::io::Write as _;

use crate::api::command::IntoExit;

/// Run a guest command entry at the `wasi:cli/run` boundary.
///
/// Awaits `entry` inside the guest telemetry scope, flushes stdout, and exits
/// the process with any non-zero status the entry reports.
/// [`command::export!`](crate::api::command::export!) runs its entry through
/// it, and a hand-written `wasi:cli/run` export calls it directly.
pub async fn execute<E: IntoExit>(entry: impl Future<Output = E>) {
    // `scope` exports telemetry before an `exit-with-code`, which never returns.
    let result = omnia_wasi_otel::scope(|| entry).await.into_exit();
    // `exit-with-code` exits immediately, meaning stdout never flushes
    let _ = std::io::stdout().flush();
    if let Err(code) = result
        && code != 0
    {
        wasip3::cli::exit::exit_with_code(code);
    }
}

#[doc(hidden)]
pub struct Package {
    pub version: &'static str,
    pub description: &'static str,
}

// What `command::export!` accepts as its entry. `M` tells the two shapes
// apart: `()` for a zero-argument `async fn`, the `Router` for a router fn.
#[doc(hidden)]
pub trait Export<P, M> {
    fn run(self, provider: P, package: Package) -> impl Future<Output = ()>;
}

impl<F, Fut, P, E> Export<P, ()> for F
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = E>,
    E: IntoExit,
{
    async fn run(self, _provider: P, _package: Package) {
        execute(self()).await;
    }
}

#[cfg(feature = "command")]
impl<F, P> Export<P, crate::api::Router<P>> for F
where
    F: FnOnce(P) -> crate::api::Router<P>,
    P: Send + Sync + 'static,
{
    async fn run(self, provider: P, package: Package) {
        execute(async move {
            let router = self(provider).package(package.version, package.description);
            router.run(wasip3::cli::environment::get_arguments()).await
        })
        .await;
    }
}
