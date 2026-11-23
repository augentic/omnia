//! Per-store context. [`StoreBase`] holds the state identical for every
//! deployment (WASI table/context, memory limiter, host→guest dispatcher);
//! [`StoreCtx`] pairs it with the deployment's backend bundle `B`, implements
//! `WasiView`, and assembles every host's linker-facing view
//! ([`StoreCtx::view`]) from the bundle's [`Provides`] impls.

use std::fmt;
use std::sync::Arc;

use wasmtime::{StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::cli::StdoutStream;
use wasmtime_wasi::{FsPerms, ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::{
    ChainCtx, Dispatcher, Extensions, Grants, GuestId, HostCtx, MountRegistry, Provides,
    RuntimeOptions,
};

/// Where every guest's stdout and stderr go when a deployment redirects
/// them: one shared stream pair, each store opening a fresh writer onto it.
///
/// Unset on a [`StoreConfig`], a guest inherits the process streams. A test
/// host sets a pair of memory pipes to read a command guest's envelope.
#[derive(Clone)]
pub struct Stdio {
    /// The guest's stdout.
    pub stdout: Arc<dyn StdoutStream + Sync>,
    /// The guest's stderr.
    pub stderr: Arc<dyn StdoutStream + Sync>,
}

impl Stdio {
    /// A pair over `stdout` and `stderr`.
    pub fn new(
        stdout: impl StdoutStream + Sync + 'static, stderr: impl StdoutStream + Sync + 'static,
    ) -> Self {
        Self {
            stdout: Arc::new(stdout),
            stderr: Arc::new(stderr),
        }
    }
}

// Manual: the streams carry no `Debug`.
impl fmt::Debug for Stdio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Stdio")
    }
}

/// The per-store construction inputs for [`StoreBase::new`].
///
/// `guest`, `options`, `dispatcher`, and `chain` say what the store is for.
/// Each of the rest has a `None` or empty value a hand-built store context
/// can pass: empty argv, no mounts, an empty environment.
pub struct StoreConfig<'a> {
    /// The guest the store is built for.
    pub guest: GuestId,
    /// Runtime options; caps linear-memory growth at
    /// [`RuntimeOptions::max_memory_bytes`].
    pub options: &'a RuntimeOptions,
    /// Type-erased host->guest dispatcher: a fresh handle to the owning
    /// [`Runtime`](crate::Runtime) so any host->guest call lands a new
    /// instance.
    pub dispatcher: Arc<dyn Dispatcher>,
    /// The dispatch-chain context the guest runs at: a root for a trigger or
    /// the command driver, the context [`ChainPolicy::enter`] derived for a
    /// dispatched callee.
    ///
    /// [`ChainPolicy::enter`]: crate::ChainPolicy::enter
    pub chain: ChainCtx,
    /// Guest argv (`args[0]` is the program name); `None` for reactor
    /// deployments that do not model a CLI invocation.
    pub args: Option<Arc<Vec<String>>>,
    /// Mount registry preopened into the guest sandbox; `None` for
    /// deployments without `[[mount]]`s.
    pub mounts: Option<Arc<MountRegistry>>,
    /// The guest's complete environment; `None` gives the guest an empty one.
    pub env: Option<Arc<Vec<(String, String)>>>,
    /// Where the guest's stdout and stderr go; `None` inherits the process
    /// streams.
    pub stdio: Option<Stdio>,
    /// The runtime's installed extensions; [`Extensions::new`] for hand-built
    /// store contexts, where a capability that reads its extension refuses.
    pub extensions: Extensions,
}

/// The fixed per-store state shared by every guest store context.
///
/// Construction policy (WASI inheritance, argv, and the memory limit) lives in
/// [`StoreBase::new`] so it is documented and unit-testable instead of being
/// inlined in [`Runtime::enter`](crate::Runtime::enter).
pub struct StoreBase {
    /// The guest this store runs.
    pub guest: GuestId,
    /// The store's WASI resource table.
    pub table: ResourceTable,
    /// The store's WASI context (the guest environment, inherited stdin,
    /// host stdout/stderr).
    pub wasi: WasiCtx,
    /// The per-guest memory limiter the runtime installs on every [`Store`].
    ///
    /// [`Store`]: wasmtime::Store
    pub limits: StoreLimits,
    /// Type-erased host->guest dispatcher; a fresh handle to the owning
    /// runtime. Inert unless a host binding reaches for it.
    pub dispatcher: Arc<dyn Dispatcher>,
    /// The dispatch-chain context this store's guest runs at. A link relay
    /// reads it to derive the callee's.
    pub chain: ChainCtx,
    /// Mount registry: the startup-validated mounts the guest's grants reach,
    /// also preopened into [`wasi`](Self::wasi). A consuming host crate reads
    /// it to match a lent `descriptor` back to its mount by directory
    /// identity. Empty unless the deployment configures `[[mount]]`s.
    pub mounts: Arc<MountRegistry>,
    /// Handle to the runtime's installed extensions, the state slot a
    /// capability crate's host binding reaches for. Empty in hand-built
    /// store contexts.
    pub extensions: Extensions,
}

impl StoreBase {
    /// Build the fixed per-store state for a single guest invocation, applying
    /// the WASI construction policy shared by every deployment.
    ///
    /// Applies the guest environment (the [`env`](StoreConfig::env) pairs,
    /// none when unset), inherits stdin, wires stdout/stderr to the
    /// [`stdio`](StoreConfig::stdio) pair when set and the host streams
    /// otherwise, applies the configured argv, and caps linear-memory growth.
    #[must_use]
    pub fn new(config: StoreConfig<'_>) -> Self {
        let mounts = config.mounts.unwrap_or_default();

        let mut wasi_builder = WasiCtxBuilder::new();
        if let Some(env) = &config.env {
            wasi_builder.envs(env);
        }
        wasi_builder.inherit_stdin();
        match config.stdio {
            Some(stdio) => {
                wasi_builder.stdout(stdio.stdout).stderr(stdio.stderr);
            }
            None => {
                wasi_builder.stdout(tokio::io::stdout()).stderr(tokio::io::stderr());
            }
        }
        if let Some(args) = &config.args {
            wasi_builder.args(args.as_slice());
        }

        // preopen each mount; one removed mid-run is skipped, with no ambient fallback
        for entry in mounts.entries() {
            let perms = if entry.writable { FsPerms::ReadWrite } else { FsPerms::ReadOnly };
            if let Err(error) = wasi_builder.preopened_dir(&entry.host_path, &entry.name, perms) {
                tracing::warn!(
                    %error,
                    name = %entry.name,
                    path = %entry.host_path.display(),
                    "failed to preopen mount; guest will not see it",
                );
            }
        }

        Self {
            guest: config.guest,
            table: ResourceTable::new(),
            wasi: wasi_builder.build(),
            limits: StoreLimitsBuilder::new().memory_size(config.options.max_memory_bytes).build(),
            dispatcher: config.dispatcher,
            chain: config.chain,
            mounts,
            extensions: config.extensions,
        }
    }
}

/// The per-guest store context every deployment shares.
///
/// `StoreCtx<B>` pairs the fixed [`StoreBase`] with the deployment's connected
/// backend bundle `B` — the `runtime!`-generated bundle, or [`()`](unit) for
/// a backend-less deployment (such as a `mode: command` `wasi:cli` runtime).
/// `WasiView` is implemented against [`base`](Self::base); [`view`](Self::view)
/// assembles any host's linker-facing view, so a deployment only supplies the
/// bundle and its [`Provides`] impls (generated by the `runtime!` macro).
pub struct StoreCtx<B> {
    /// The fixed per-store state shared by every deployment.
    pub base: StoreBase,
    /// The deployment's connected backend bundle (cloned per store).
    pub backends: B,
}

impl<B> StoreCtx<B> {
    /// Assemble host `H`'s linker-facing view: the host's borrow of the
    /// bundle plus the store's resource table.
    ///
    /// A host passes `StoreCtx::view::<Self>` as the accessor of its
    /// bindgen `add_to_linker`; a trigger server calls it on the store it
    /// builds for a guest.
    pub fn view<H: HostCtx>(&mut self) -> H::Data<'_>
    where
        B: Provides<H>,
    {
        H::view(self.backends.borrow(), &mut self.base.table)
    }

    /// The guest this store runs.
    #[must_use]
    pub const fn guest(&self) -> &GuestId {
        &self.base.guest
    }

    /// A handle to the store's startup-validated mount registry, for
    /// matching a lent `wasi:filesystem` descriptor against the authorized
    /// mounts.
    #[must_use]
    pub fn mounts(&self) -> Arc<MountRegistry> {
        Arc::clone(&self.base.mounts)
    }

    /// A handle to the store's host->guest dispatcher, for host-mediated
    /// dynamic linking.
    #[must_use]
    pub fn dispatcher(&self) -> Arc<dyn Dispatcher> {
        Arc::clone(&self.base.dispatcher)
    }

    /// A handle to the runtime's installed extensions, the state a capability
    /// crate's host binding reaches for.
    #[must_use]
    pub fn extensions(&self) -> Extensions {
        self.base.extensions.clone()
    }

    /// The dispatch-chain context this store's guest runs at; the link relay
    /// derives the callee's from it.
    #[must_use]
    pub const fn chain(&self) -> ChainCtx {
        self.base.chain
    }
}

impl<B: Send + 'static> WasiView for StoreCtx<B> {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.base.wasi,
            table: &mut self.base.table,
        }
    }
}

const RUST_LOG: &str = "RUST_LOG";
const NO_COLOR: &str = "NO_COLOR";

/// The complete guest environment: the `host` pairs `inherit` names, with
/// `RUST_LOG` set to the run's tracing directives and `NO_COLOR` set when
/// the console does not colour its lines.
///
/// Each `inherit` entry is a whole name, or a prefix that ends in `*`, so
/// `*` alone keeps every pair.
///
/// `rust_log` is the composition of the run's verbosity flag with the
/// process `RUST_LOG` ([`telemetry::directives`](crate::telemetry::directives)),
/// so it replaces an inherited variable in place — the process value is
/// already folded in — or is appended when none is inherited.
///
/// `ansi` is whether the host console colours its lines. When it does not,
/// `NO_COLOR=1` is appended so a guest's console — which cannot see the
/// descriptor it shares — decides the same way; an inherited `NO_COLOR` set
/// to a non-empty value stands as it is.
pub fn guest_env(
    host: impl IntoIterator<Item = (String, String)>, inherit: &[String], rust_log: &str,
    ansi: bool,
) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = host
        .into_iter()
        .filter(|(name, _)| inherit.iter().any(|pattern| crate::names::matches(pattern, name)))
        .collect();
    match env.iter_mut().find(|(name, _)| name == RUST_LOG) {
        Some((_, value)) => rust_log.clone_into(value),
        None => env.push((RUST_LOG.to_owned(), rust_log.to_owned())),
    }

    if !ansi {
        match env.iter_mut().find(|(name, _)| name == NO_COLOR) {
            Some((_, value)) if value.is_empty() => "1".clone_into(value),
            Some(_) => {}
            None => env.push((NO_COLOR.to_owned(), "1".to_owned())),
        }
    }

    env
}

// the pairs of `env` that `grants` reach; the runtime's own `RUST_LOG` and
// `NO_COLOR` reach every guest
pub fn granted_env(env: &[(String, String)], grants: &Grants) -> Vec<(String, String)> {
    env.iter()
        .filter(|(name, _)| name == RUST_LOG || name == NO_COLOR || grants.allows_env(name))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(entries: &[(&str, &str)]) -> Vec<(String, String)> {
        entries.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())).collect()
    }

    fn names(patterns: &[&str]) -> Vec<String> {
        patterns.iter().map(|pattern| (*pattern).to_owned()).collect()
    }

    // A console off a terminal tells the guest so; an inherited `NO_COLOR`
    // already in force stands, and an empty one — no colour suppression, by
    // the convention — is filled the way an absent one is.
    #[test]
    fn piped_stderr() {
        let all = names(&["*"]);
        let host = pairs(&[("HOME", "/home/op")]);
        assert_eq!(
            guest_env(host, &all, "info", false),
            pairs(&[("HOME", "/home/op"), ("RUST_LOG", "info"), ("NO_COLOR", "1")])
        );

        let host = pairs(&[("NO_COLOR", "true"), ("RUST_LOG", "off")]);
        assert_eq!(
            guest_env(host, &all, "info", false),
            pairs(&[("NO_COLOR", "true"), ("RUST_LOG", "info")])
        );

        let host = pairs(&[("NO_COLOR", ""), ("RUST_LOG", "off")]);
        assert_eq!(
            guest_env(host, &all, "info", false),
            pairs(&[("NO_COLOR", "1"), ("RUST_LOG", "info")])
        );
    }

    // The runtime's own variables reach the guest whatever it inherits; a
    // host `NO_COLOR` it does not inherit leaves the console's decision.
    #[test]
    fn named_variables() {
        let host = pairs(&[
            ("HOME", "/home/op"),
            ("ORDERS_URL", "https://orders"),
            ("ORDERS_KEY", "secret"),
            ("PATH", "/bin"),
            ("NO_COLOR", "true"),
        ]);
        assert_eq!(
            guest_env(host.clone(), &names(&["ORDERS_*", "HOME"]), "warn", true),
            pairs(&[
                ("HOME", "/home/op"),
                ("ORDERS_URL", "https://orders"),
                ("ORDERS_KEY", "secret"),
                ("RUST_LOG", "warn"),
            ])
        );
        assert_eq!(guest_env(host, &[], "warn", true), pairs(&[("RUST_LOG", "warn")]));
    }
}
