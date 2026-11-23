//! Connected runtime: [`Runtime`], [`RuntimeParts`], [`WeakRuntime`],
//! [`ExitStatus`], and the concurrency gate's [`Slot`] and [`Invocations`].

mod command;
mod enter;
mod gate;

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Once, Weak};

use anyhow::Result;
use tokio::sync::watch;
use wasmtime::Store;
use wasmtime::component::InstancePre;

use self::gate::Gate;
pub use self::gate::{Invocations, Slot};
use crate::artifact::component;
use crate::digest::Digest;
use crate::extensions::Extensions;
use crate::mount::MountRegistry;
use crate::registry::{Guest, GuestId, HttpRoutes, PublishError, TriggerRouter, ensure_granted};
use crate::seam::StoreFactory;
use crate::source::{AcquireError, RegistrySource, Source, SourceSpec, Verified};
use crate::store::Stdio;
use crate::{
    Backends, ChainCtx, Dispatcher, Grants, Registry, RuntimeOptions, StoreBase, StoreCtx,
};

/// Guest exit code. [`code_u8`](Self::code_u8) and [`ExitCode`](std::process::ExitCode)
/// keep only the low byte (POSIX semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus(i32);

impl ExitStatus {
    /// Exit code `0`.
    pub const SUCCESS: Self = Self(0);

    /// Full `i32` exit code from the guest.
    #[must_use]
    pub const fn code(self) -> i32 {
        self.0
    }

    /// Low byte of the exit code (POSIX process status).
    #[must_use]
    pub const fn code_u8(self) -> u8 {
        self.0.to_le_bytes()[0]
    }
}

impl From<i32> for ExitStatus {
    fn from(code: i32) -> Self {
        Self(code)
    }
}

impl From<ExitStatus> for std::process::ExitCode {
    fn from(status: ExitStatus) -> Self {
        Self::from(status.code_u8())
    }
}

/// Inputs to [`Runtime::from_parts`].
pub struct RuntimeParts<B: 'static> {
    /// Deployment name read by trigger servers and the bootstrap log.
    pub name: Arc<str>,
    /// Assembled guest registry.
    pub registry: Arc<Registry<B>>,
    /// Guest argv.
    pub args: Vec<String>,
    /// Mount registry opened from the deployment's preopens. Each store
    /// preopens the mounts its guest's grants reach.
    pub mounts: Arc<MountRegistry>,
    /// Connected backend bundle.
    pub backends: B,
    /// How a declared package source is acquired at first use. `None` when
    /// the runtime has no registry client (omnia built without the `loader`
    /// feature), where every package guest's first use fails.
    pub packages: Option<Arc<dyn RegistrySource>>,
    /// The deployment's wasm-pkg client configuration (TOML), resolved to its
    /// contents: how a package source is routed to a registry. `None` when
    /// the deployment declares none, where every package source is refused.
    pub registry_config: Option<String>,
    /// The run's tracing directives — its verbosity flag composed with the
    /// process `RUST_LOG` ([`telemetry::directives`](crate::telemetry::directives))
    /// — set as every guest's `RUST_LOG`.
    pub rust_log: String,
    /// The host environment variables guests see, each a whole name or a
    /// prefix that ends in `*`. Empty, a guest sees only `RUST_LOG`, and
    /// `NO_COLOR` when the console does not colour its lines.
    pub inherit: Vec<String>,
    /// Where every guest's stdout and stderr go; `None` inherits the process
    /// streams.
    pub stdio: Option<Stdio>,
    /// Command-mode guest identity, if any.
    pub command_guest: Option<GuestId>,
}

/// Connected host runtime: registry, argv, mounts, and backend bundle.
///
/// A thin handle over shared state: `clone()` bumps two reference counts, so
/// the per-request and per-message handler clones never copy the backend
/// bundle.
pub struct Runtime<B: 'static> {
    inner: Arc<RuntimeInner<B>>,
    // built once so `store()` hands out clones instead of one per store
    dispatcher: Arc<dyn Dispatcher>,
}

impl<B: Backends> Runtime<B> {
    fn with_inner(inner: Arc<RuntimeInner<B>>) -> Self {
        let dispatcher = Arc::new(RuntimeDispatcher {
            inner: Arc::clone(&inner),
        });
        Self { inner, dispatcher }
    }

    /// Build a runtime from already-assembled parts.
    ///
    /// Does not wire the host-mediated link serve side — a caller whose
    /// deployment declares link interfaces must run
    /// [`serve_links`](Self::serve_links) itself before dispatching.
    #[must_use]
    pub fn from_parts(parts: RuntimeParts<B>) -> Self {
        // the environment as it stands; a non-UTF-8 pair cannot cross `wasi:cli`
        let host = std::env::vars_os().filter_map(|(name, value)| {
            Some((name.into_string().ok()?, value.into_string().ok()?))
        });
        let env = crate::store::guest_env(
            host,
            &parts.inherit,
            &parts.rust_log,
            crate::telemetry::ansi(),
        );
        let options = parts.registry.options();
        let gate = options.max_concurrent_invocations.unwrap_or(options.pool_max_instances);

        Self::with_inner(Arc::new(RuntimeInner {
            name: parts.name,
            registry: parts.registry,
            args: Arc::new(parts.args),
            mounts: parts.mounts,
            backends: parts.backends,
            packages: parts.packages,
            command_guest: parts.command_guest,
            registry_config: parts.registry_config,
            env: Arc::new(env),
            inherit: parts.inherit,
            stdio: parts.stdio,
            extensions: Extensions::new(),
            stores_built: AtomicU64::new(0),
            gate: Gate::new(gate as usize),
            hop_warning: Once::new(),
            stop: watch::channel(false).0,
        }))
    }

    /// The deployment's mounts. Each store preopens those its guest's grants
    /// reach.
    #[must_use]
    pub fn mounts(&self) -> &MountRegistry {
        &self.inner.mounts
    }

    /// The deployment's wasm-pkg client configuration (TOML), resolved to its
    /// contents, for the guest loader to route package sources through;
    /// `None` when the deployment declares none.
    #[must_use]
    pub fn registry_config(&self) -> Option<&str> {
        self.inner.registry_config.as_deref()
    }

    /// The guest `id` names, loaded from its declared source on first use.
    ///
    /// A registered guest is returned as it stands. One the deployment
    /// declares but has not loaded is [acquired](Self::acquire) and
    /// [admitted](Self::admit_bytes) under its entry's policy; when a racing
    /// first use admitted it first, that registration stands and is
    /// returned. Every way into a guest resolves through here, so a declared
    /// guest loads the first time a route, the command drive, a link call, a
    /// host dispatch, or the guest loader names it.
    ///
    /// # Errors
    ///
    /// - [`GuestError::Unregistered`] for an identity the deployment neither
    ///   registered nor declares.
    /// - [`GuestError::NotFound`], [`GuestError::Denied`],
    ///   [`GuestError::Unsupported`], or [`GuestError::Unavailable`] when the
    ///   source did not produce its bytes, as [`acquire`](Self::acquire)
    ///   reports.
    /// - [`GuestError::Denied`] when the component imports code from a
    ///   package its entry's grants do not reach.
    /// - [`GuestError::Invalid`] when the bytes miss the pin, are a
    ///   pre-compiled artifact where raw wasm alone is admitted, or do not
    ///   load as a component.
    /// - [`GuestError::Internal`] when the registration failed.
    pub async fn guest(&self, id: &GuestId) -> Result<Arc<Guest<B>>, GuestError> {
        let registry = self.registry();
        if let Some(guest) = registry.get(id) {
            return Ok(guest);
        }
        let Some(entry) = registry.declared(id) else {
            return Err(GuestError::Unregistered(id.clone()));
        };
        let bytes = self.acquire(entry.spec(), None).await?;
        self.admit_bytes(id.clone(), bytes, Policy::Declared(entry)).await
    }

    /// The bytes a source names, acquired as its kind demands: embedded
    /// bytes copied, a path read, a package fetched through the deployment's
    /// registry source — from `endpoint` when the load names one and the
    /// deployment's routing leaves the package's namespace open. Nothing is
    /// verified here; [`admit_bytes`](Self::admit_bytes) does that under the
    /// caller's [`Policy`].
    ///
    /// # Errors
    ///
    /// - [`GuestError::NotFound`] when no file is at the path, or nothing
    ///   routes the package or holds its release.
    /// - [`GuestError::Denied`] or [`GuestError::Invalid`] when the
    ///   [`RegistrySource`] refuses the package as its [`AcquireError`]
    ///   names.
    /// - [`GuestError::Unsupported`] when the runtime has no registry client
    ///   to fetch a package, or its client cannot serve the request.
    /// - [`GuestError::Unavailable`] when the file, the store, or the
    ///   registry did not produce the bytes.
    pub async fn acquire(
        &self, spec: &SourceSpec, endpoint: Option<&str>,
    ) -> Result<Vec<u8>, GuestError> {
        match spec {
            SourceSpec::Package(package) => {
                let Some(packages) = &self.inner.packages else {
                    return Err(GuestError::Unsupported(format!(
                        "package `{package}` cannot be fetched: this runtime has no registry \
                         client; build omnia with the `loader` feature"
                    )));
                };
                packages.acquire(package, endpoint).await.map_err(|error| match error {
                    AcquireError::NotFound(reason) => GuestError::NotFound(reason),
                    AcquireError::Denied(reason) => GuestError::Denied(reason),
                    AcquireError::Invalid(reason) => GuestError::Invalid(reason),
                    AcquireError::Unsupported(reason) => GuestError::Unsupported(reason),
                    AcquireError::Unavailable(reason) => GuestError::Unavailable(reason),
                })
            }
            SourceSpec::Path(path) => tokio::fs::read(path).await.map_err(|error| {
                let reason = format!("reading `{}`: {error}", path.display());
                if error.kind() == io::ErrorKind::NotFound {
                    GuestError::NotFound(reason)
                } else {
                    GuestError::Unavailable(reason)
                }
            }),
            SourceSpec::Bytes(bytes) => Ok(bytes.to_vec()),
        }
    }

    /// Whether bytes may seat as `id` under `policy` at all: the check
    /// [`admit_bytes`](Self::admit_bytes) makes before hashing anything,
    /// exposed so a caller can refuse before acquiring. A name the
    /// deployment declares is bound by its entry alone — [`Policy::Declared`]
    /// with that entry passes, and anything else offered under the name is
    /// refused. Caller-named grants must pass
    /// [`validate_grants`](Self::validate_grants).
    ///
    /// # Errors
    ///
    /// - [`GuestError::Denied`] for caller-named bytes under a name the
    ///   deployment declares.
    /// - [`GuestError::Invalid`] for caller-named grants that do not
    ///   validate.
    /// - [`GuestError::Internal`] for a declared policy whose entry is not
    ///   the deployment's own for `id`.
    pub fn admits(&self, id: &GuestId, policy: &Policy<'_>) -> Result<(), GuestError> {
        match (policy, self.registry().declared(id)) {
            (Policy::Declared(entry), Some(own)) if entry.id() == id && *entry == own => Ok(()),
            (Policy::Declared(_), _) => Err(GuestError::Internal(format!(
                "guest `{id}` was offered under a declared policy whose entry is not this \
                 deployment's own"
            ))),
            (Policy::CallerNamed { .. }, Some(_)) => Err(GuestError::Denied(format!(
                "`{id}` is a guest this deployment declares, bound by its entry alone; load it \
                 as the declared guest `{id}`, whose source and pin are the deployment's"
            ))),
            (Policy::CallerNamed { grants, .. }, None) => {
                self.validate_grants(grants).map_err(|error| {
                    GuestError::Invalid(format!("guest `{id}` has invalid grants: {error:#}"))
                })
            }
        }
    }

    /// Check `grants` against this deployment's mounts and inherited host
    /// variables, as startup checks a declared guest's.
    ///
    /// # Errors
    ///
    /// Returns an error naming the first entry [`Grants::validate`] refuses.
    pub fn validate_grants(&self, grants: &Grants) -> Result<()> {
        let mounts: Vec<&str> =
            self.inner.mounts.entries().iter().map(|mount| mount.name.as_str()).collect();
        grants.validate(&mounts, &self.inner.inherit)
    }

    /// Verify `bytes` under `policy`, admit them as `id`, and answer the
    /// guest standing under `id` afterwards.
    ///
    /// A declared entry's bytes are verified as the entry declares (its pin,
    /// the format its source kind admits) and run under its grants;
    /// caller-named bytes are held to the pin, admitted as raw wasm alone,
    /// and run under the grants the policy names. Either way the component
    /// may import code only from packages its grants reach, and the bytes
    /// are loaded as a boot guest's are, pre-instantiated against the shared
    /// host set, wired into the host-mediated link serve side, and
    /// published with their endpoint as one lifecycle transition, the digest
    /// and grants recorded on the entry ([`Guest::digest`],
    /// [`Guest::grants`]). A name already active when the bytes arrive
    /// stands for a declared entry (a racing first use) and attests
    /// caller-named bytes on the same digest and grants alone. Whether the
    /// component exports a linked interface is not checked: a guest that
    /// exports none is still reachable through the host [`Dispatcher`], and
    /// a link call to it fails at the call site.
    ///
    /// Every admitter passes here — [`guest`](Self::guest), the guest
    /// loader, [`register`](Self::register) — so the rule that a declared
    /// name is bound by its entry alone ([`admits`](Self::admits)) holds for
    /// all of them.
    ///
    /// # Errors
    ///
    /// - [`GuestError::Denied`] for caller-named bytes under a name the
    ///   deployment declares or active under other grants, or a component
    ///   importing code from a package its grants do not reach.
    /// - [`GuestError::Invalid`] when a declared entry's bytes miss its pin,
    ///   caller-named grants do not [validate](Grants::validate), or the
    ///   bytes are a pre-compiled artifact where raw wasm alone is admitted,
    ///   or do not load as a component.
    /// - [`GuestError::Conflict`] when caller-named bytes miss the pin, or
    ///   the name is active under other bytes.
    /// - [`GuestError::Internal`] on a serve or publication failure.
    pub async fn admit_bytes(
        &self, id: GuestId, bytes: Vec<u8>, policy: Policy<'_>,
    ) -> Result<Arc<Guest<B>>, GuestError> {
        self.admits(&id, &policy)?;
        let grants = match policy {
            Policy::Declared(entry) => entry.grants(),
            Policy::CallerNamed { grants, .. } => grants,
        };

        // verify under the policy
        let invalid = |error: anyhow::Error| GuestError::Invalid(format!("{error:#}"));
        let verified = match policy {
            Policy::Declared(entry) => entry.verified(bytes).map_err(invalid)?,
            Policy::CallerNamed { pin, .. } => {
                let found = Digest::of(&bytes);
                if let Some(pin) = pin
                    && pin != found
                {
                    return Err(GuestError::Conflict {
                        found,
                        detail: format!(
                            "guest `{id}` resolved to {found}, not its declared digest {pin}"
                        ),
                    });
                }
                Verified::wasm(bytes).map_err(invalid)?
            }
        };
        let digest = verified.digest();

        // seat them, or settle with the registration that got there first
        match self.seat(id.clone(), verified, grants.clone()).await {
            Ok(guest) => Ok(guest),
            Err(SeatError::Occupied) => {
                let standing = self.registry().get(&id).ok_or_else(|| {
                    GuestError::Internal(format!(
                        "guest `{id}` was admitted by a racing load and deregistered before it \
                         could be attested"
                    ))
                })?;
                match policy {
                    Policy::CallerNamed { .. } if standing.digest() != digest => {
                        Err(GuestError::Conflict {
                            found: digest,
                            detail: format!(
                                "guest `{id}` is active under other bytes ({}, not {digest})",
                                standing.digest()
                            ),
                        })
                    }
                    Policy::CallerNamed { grants, .. } if standing.grants().as_ref() != grants => {
                        Err(GuestError::Denied(format!(
                            "guest `{id}` is active under other grants than this admission names"
                        )))
                    }
                    Policy::Declared(_) | Policy::CallerNamed { .. } => Ok(standing),
                }
            }
            Err(SeatError::Denied(reason)) => Err(GuestError::Denied(reason)),
            Err(SeatError::Invalid(reason)) => Err(GuestError::Invalid(reason)),
            Err(SeatError::Internal(reason)) => Err(GuestError::Internal(reason)),
        }
    }

    /// The deployment name — read by trigger servers and the bootstrap log.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.inner.name
    }

    /// Build the HTTP trigger's [`TriggerRouter`] over this runtime's
    /// registry and static route table so the boot-time routing decision
    /// lives in one place.
    ///
    /// `probe` resolves a guest's typed handler indices; a loaded guest is
    /// capable exactly when it succeeds. A route may also name a declared
    /// guest, probed when its first request loads it.
    ///
    /// # Errors
    ///
    /// Returns an error if a route names a guest that neither exports the
    /// handler nor is declared, or two or more guests export it with no
    /// routes.
    pub fn http_trigger_router<I, E, F>(&self, probe: F) -> Result<TriggerRouter<HttpRoutes>>
    where
        F: FnMut(&InstancePre<StoreCtx<B>>) -> std::result::Result<I, E>,
    {
        TriggerRouter::build(
            self.registry(),
            "http",
            self.registry().routes().http().clone(),
            probe,
        )
    }

    /// The command-mode guest identity (the manifest entry marked
    /// `command = true`), if any.
    #[must_use]
    pub fn command_guest(&self) -> Option<&GuestId> {
        self.inner.command_guest.as_ref()
    }

    /// Guest registry.
    #[must_use]
    pub fn registry(&self) -> &Registry<B> {
        &self.inner.registry
    }

    /// The deployment's connected backend bundle.
    #[must_use]
    pub fn backends(&self) -> &B {
        &self.inner.backends
    }

    /// The capability-crate state installed at assembly — the same set every
    /// store context carries.
    #[must_use]
    pub fn extensions(&self) -> &Extensions {
        &self.inner.extensions
    }

    /// A non-owning handle for state that must call back into the runtime;
    /// see [`Extensions`] for why extensions never hold a [`Runtime`].
    #[must_use]
    pub fn downgrade(&self) -> WeakRuntime<B> {
        WeakRuntime {
            inner: Arc::downgrade(&self.inner),
        }
    }

    /// The cached host→guest dispatch capability — the same handle
    /// every store context carries, for host-side callers (tests,
    /// embedders) that invoke a guest export directly.
    #[must_use]
    pub fn dispatcher(&self) -> Arc<dyn Dispatcher> {
        Arc::clone(&self.dispatcher)
    }

    /// Runtime options from the environment.
    #[must_use]
    pub fn options(&self) -> &RuntimeOptions {
        self.registry().options()
    }

    /// How many guest stores this runtime has built: one per guest invocation.
    ///
    /// Monotonic, so a reader holding the last value it saw knows whether a
    /// guest has run since, however briefly.
    #[must_use]
    pub fn stores_built(&self) -> u64 {
        self.inner.stores_built.load(Ordering::Relaxed)
    }

    // A fresh store context for `guest` at `chain`. The guest's environment
    // is the inherited host variables its grants reach, as they stood when
    // the runtime was assembled, with `RUST_LOG` set to the run's tracing
    // directives and `NO_COLOR` set when the console the guest shares does
    // not colour its lines. Its preopens are the mounts its grants reach.
    fn store_ctx(&self, guest: &Guest<B>, chain: ChainCtx) -> StoreCtx<B> {
        let grants = guest.grants();
        let (mounts, env) = if grants.is_all() {
            (Arc::clone(&self.inner.mounts), Arc::clone(&self.inner.env))
        } else {
            let mounts = self.inner.mounts.only(|mount| grants.allows_mount(&mount.name));
            (Arc::new(mounts), Arc::new(crate::store::granted_env(&self.inner.env, grants)))
        };
        StoreCtx {
            base: StoreBase::new(crate::StoreConfig {
                guest: guest.id().clone(),
                grants: Arc::clone(grants),
                options: self.options(),
                dispatcher: Arc::clone(&self.dispatcher),
                chain,
                args: Some(Arc::clone(&self.inner.args)),
                mounts: Some(mounts),
                env: Some(env),
                stdio: self.inner.stdio.clone(),
                extensions: self.inner.extensions.clone(),
            }),
            backends: self.inner.backends.clone(),
        }
    }

    // A store with epoch deadline, optional fuel, and memory limiter
    // installed. Panics when `MAX_FUEL` is set but the engine was built
    // without fuel metering, which would otherwise run guests unmetered.
    fn build_store(&self, data: StoreCtx<B>) -> Store<StoreCtx<B>> {
        self.inner.stores_built.fetch_add(1, Ordering::Relaxed);
        let options = self.options();
        let mut store = Store::new(self.registry().engine(), data);

        store.set_epoch_deadline(1);
        store.epoch_deadline_async_yield_and_update(1);

        if options.max_fuel > 0 {
            // a failure means the engine was built from different options
            store.set_fuel(options.max_fuel).expect("engine was built without fuel metering");
        }

        store.limiter(|ctx| &mut ctx.base.limits);
        store
    }

    // A shareable factory for fresh, fully configured stores for `guest` at a
    // given chain context: what the link serve side and host dispatch
    // instantiate the guest on, per call.
    #[must_use]
    pub(crate) fn store_factory(&self, guest: &Arc<Guest<B>>) -> StoreFactory<B> {
        let runtime = self.clone();
        let guest = Arc::clone(guest);
        Arc::new(move |chain| {
            runtime.warn_unsized_gate();
            runtime.build_store(runtime.store_ctx(&guest, chain))
        })
    }

    // A hop's instance takes no slot, so a gate as wide as the pool lets hops
    // fill the pool before the gate refuses anything.
    fn warn_unsized_gate(&self) {
        let options = self.options();
        if !options.pooling || options.max_concurrent_invocations.is_some() {
            return;
        }
        self.inner.hop_warning.call_once(|| {
            tracing::warn!(
                pool_max_instances = options.pool_max_instances,
                "a call between guests takes an instance outside the concurrency gate; with \
                 MAX_CONCURRENT_INVOCATIONS unset the gate is as wide as the pool, so set it \
                 below POOL_MAX_INSTANCES to leave these calls room"
            );
        });
    }

    /// Drive the deployment's `wasi:cli/run` command once, returning the
    /// guest's exit status.
    ///
    /// The run waits for a [slot](Self::slot) of the concurrency gate, as a
    /// message delivery does, and holds it for as long as it runs.
    ///
    /// # Errors
    ///
    /// Returns an error if the command guest is not registered, routing is
    /// ambiguous, the guest cannot be instantiated, or the command traps
    /// without a guest exit code.
    pub async fn run_command(&self) -> Result<ExitStatus> {
        command::drive(self).await
    }

    /// Register a raw wasm component at run time under `id`, running under
    /// `grants`: the embedder's caller-named admission,
    /// [`admit_bytes`](Self::admit_bytes) under [`Policy::CallerNamed`] with
    /// no pin. Pre-compiled bytes are refused here: an embedder holding
    /// `omnia compile` output its own pipeline produced admits it through
    /// [`admit`](Self::admit) with an `unsafe` [`Verified::trusted`].
    ///
    /// The identity is opaque and must not be one the deployment declares;
    /// an upgrade is [`deregister`](Self::deregister) + `register` (or a new
    /// id), and an identity already active attests on the same bytes and
    /// grants alone. A failed registration leaves no partial state. The
    /// registry entry records the content digest of `bytes`, as it does for
    /// every guest.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes are a pre-compiled artifact, `id` is a
    /// name the deployment declares or is active under other bytes or other
    /// grants, `grants` do not [validate](Grants::validate), the component
    /// imports code from a package `grants` do not reach, the bytes cannot
    /// be loaded, the component's imports exceed the deployment's linked
    /// host set and declared link interfaces, or its linked exports cannot
    /// be served.
    pub async fn register(
        &self, id: impl Into<GuestId>, bytes: Vec<u8>, grants: Grants,
    ) -> Result<()> {
        let policy = Policy::CallerNamed {
            pin: None,
            grants: &grants,
        };
        self.admit_bytes(id.into(), bytes, policy).await?;
        Ok(())
    }

    /// Admit verified component bytes as a late guest under `id`, running
    /// under `grants`: the embedder's path for bytes it vouches for itself,
    /// seated exactly as [`admit_bytes`](Self::admit_bytes) seats them and
    /// held to the same rule that a name the deployment declares is bound by
    /// its entry alone. The token's digest is recorded on the registry
    /// entry, so the attestation lives exactly as long as the entry.
    ///
    /// # Errors
    ///
    /// - [`GuestError::Denied`] for a name the deployment declares, or a
    ///   component importing code from a package `grants` do not reach.
    /// - [`GuestError::Conflict`] for a name already registered, by an
    ///   earlier or a racing registration.
    /// - [`GuestError::Invalid`] for `grants` that do not
    ///   [validate](Grants::validate), or bytes that do not load as a
    ///   component against the deployment's host set.
    /// - [`GuestError::Internal`] on a serve or publication failure.
    pub async fn admit(
        &self, id: GuestId, verified: Verified, grants: Grants,
    ) -> Result<(), GuestError> {
        let policy = Policy::CallerNamed {
            pin: None,
            grants: &grants,
        };
        self.admits(&id, &policy)?;
        let found = verified.digest();
        match self.seat(id.clone(), verified, grants).await {
            Ok(_) => Ok(()),
            Err(SeatError::Occupied) => Err(GuestError::Conflict {
                found,
                detail: format!("guest `{id}` is already registered"),
            }),
            Err(SeatError::Denied(reason)) => Err(GuestError::Denied(reason)),
            Err(SeatError::Invalid(reason)) => Err(GuestError::Invalid(reason)),
            Err(SeatError::Internal(reason)) => Err(GuestError::Internal(reason)),
        }
    }

    // Seat verified bytes as the guest `id` under `grants`: load them as a
    // boot guest's are loaded, check their imports against the grants,
    // pre-instantiate against the shared host set, wire the host-mediated
    // link serve side, then publish entry and endpoint as one lifecycle
    // transition, so no dispatch resolves the entry and misses the endpoint
    // or vice versa.
    async fn seat(
        &self, id: GuestId, verified: Verified, grants: Grants,
    ) -> Result<Arc<Guest<B>>, SeatError> {
        let registry = self.registry();

        // early occupancy check; the publish below re-checks transactionally
        if registry.get(&id).is_some() {
            return Err(SeatError::Occupied);
        }

        let digest = verified.digest();
        let component = component(registry.engine(), verified)
            .await
            .map_err(|error| SeatError::Invalid(format!("validating `{id}`: {error:#}")))?;
        ensure_granted(&id, &component, registry.engine(), &grants)
            .map_err(|error| SeatError::Denied(format!("{error:#}")))?;
        let instance_pre = registry
            .instantiate_late(&id, &component)
            .map_err(|error| SeatError::Invalid(format!("pre-instantiating `{id}`: {error:#}")))?;
        let guest = Arc::new(Guest::local(id.clone(), instance_pre, digest, grants));

        // serve as a pending endpoint, then publish endpoint and entry as one step
        registry.seam().serve(self.store_factory(&guest), &guest).await.map_err(|error| {
            SeatError::Internal(format!("serving `{id}` seam exports: {error:#}"))
        })?;
        registry.publish(Arc::clone(&guest)).map_err(|error| match error {
            PublishError::Occupied => SeatError::Occupied,
            PublishError::Transport(error) => {
                SeatError::Internal(format!("publishing `{id}`: {error:#}"))
            }
        })?;

        tracing::debug!(guest = %id, "guest registered");
        Ok(guest)
    }

    /// Remove a late guest — one registered at run time, or a declared guest
    /// loaded at first use, which its next use reloads from its source.
    /// In-flight calls complete on the instance they hold
    /// (instance-per-call). Guests loaded at boot are refused.
    ///
    /// # Errors
    ///
    /// Returns an error if `id` names a guest loaded at boot or is not
    /// registered.
    pub fn deregister(&self, id: &GuestId) -> Result<()> {
        self.registry().remove(id)?;
        tracing::debug!(guest = %id, "guest deregistered");
        Ok(())
    }

    /// Signal long-lived trigger servers to stop accepting new work and drain.
    pub fn stop(&self) {
        let _ = self.inner.stop.send(true);
    }

    /// Resolves once [`stop`](Self::stop) or [`shutdown`](Self::shutdown) has
    /// been called.
    pub async fn stopped(&self) {
        let mut rx = self.inner.stop.subscribe();
        let _ = rx.wait_for(|&stopped| stopped).await;
    }

    /// Release every link-serve endpoint, aborting the drain tasks that pin
    /// `Runtime` clones (and with them the engine's pooling reservation).
    ///
    /// `run` does this as the drive completes; an embedder holding a
    /// [`from_parts`](Self::from_parts) runtime calls it when the deployment
    /// is finished. In-flight invocations hold their own server handles and
    /// complete; only new dispatches are cut off.
    pub fn shutdown(&self) {
        self.stop();
        self.registry().seam().shutdown();
    }

    /// Wire the serve side of every registered guest's linked exports, then
    /// publish them all under one lifecycle transition so polyfilled imports
    /// can reach them. `Deployment::assemble` calls this during bootstrap; only
    /// a runtime assembled through [`from_parts`](Self::from_parts) wires it
    /// explicitly. A no-op for a deployment that declares no link interfaces.
    ///
    /// # Errors
    ///
    /// Returns an error if a guest's export cannot be served, or a served guest
    /// already has an endpoint (`serve_links` ran twice).
    pub async fn serve_links(&self) -> Result<()> {
        let registry = self.registry();
        let seam = registry.seam();
        let guests: Vec<_> = registry.guests().collect();

        // on failure, release what is still parked so a failed bootstrap pins nothing
        let discard_from = |first_unpublished: usize| {
            for guest in &guests[first_unpublished..] {
                seam.discard(guest.id());
            }
        };

        for guest in &guests {
            if let Err(error) = seam.serve(self.store_factory(guest), guest).await {
                discard_from(0);
                return Err(error);
            }
        }

        let _lifecycle = registry.lifecycle_write();
        for (published, guest) in guests.iter().enumerate() {
            if let Err(error) = seam.publish(guest.id()) {
                discard_from(published + 1);
                return Err(error);
            }
        }
        Ok(())
    }
}

// Manual: `StoreCtx<B>` is not `Clone`; both fields are `Arc`-backed.
impl<B: Backends> Clone for Runtime<B> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            dispatcher: Arc::clone(&self.dispatcher),
        }
    }
}

struct RuntimeInner<B: 'static> {
    // carried state, never a process environment variable
    name: Arc<str>,
    registry: Arc<Registry<B>>,
    args: Arc<Vec<String>>,
    mounts: Arc<MountRegistry>,
    backends: B,
    packages: Option<Arc<dyn RegistrySource>>,
    // absent, command mode routes to the sole static `wasi:cli/run` exporter
    command_guest: Option<GuestId>,
    // for an embedder that installs the loader capability by hand
    registry_config: Option<String>,
    // every store's environment, taken once at assembly
    env: Arc<Vec<(String, String)>>,
    // what caller-named grants' `env` entries are checked against
    inherit: Vec<String>,
    stdio: Option<Stdio>,
    extensions: Extensions,
    // one per `build_store`, so an observer can tell a quiet pool from one
    // whose guests finished between its looks
    stores_built: AtomicU64,
    gate: Gate,
    hop_warning: Once,
    stop: watch::Sender<bool>,
}

// How seating verified bytes as a guest failed.
enum SeatError {
    // an earlier or racing registration holds the identity
    Occupied,
    // the component imports code from a package its grants do not reach
    Denied(String),
    Invalid(String),
    Internal(String),
}

/// [`Dispatcher`] over the runtime's shared state.
///
/// A separate type (rather than `Runtime` itself) so the cached
/// `Arc<dyn Dispatcher>` inside [`Runtime`] does not create a reference cycle.
pub struct RuntimeDispatcher<B: 'static> {
    inner: Arc<RuntimeInner<B>>,
}

impl<B: Backends> RuntimeDispatcher<B> {
    /// Rehydrate a full runtime handle for a dispatched call.
    pub fn runtime(&self) -> Runtime<B> {
        Runtime::with_inner(Arc::clone(&self.inner))
    }
}

/// A non-owning [`Runtime`] handle, the form a runtime extension holds to
/// call back into the runtime without leaking it through a reference cycle.
pub struct WeakRuntime<B: 'static> {
    inner: Weak<RuntimeInner<B>>,
}

impl<B: Backends> WeakRuntime<B> {
    /// Upgrade to a full handle; `None` once the runtime has shut down.
    #[must_use]
    pub fn upgrade(&self) -> Option<Runtime<B>> {
        Some(Runtime::with_inner(self.inner.upgrade()?))
    }
}

// Manual: a handle clone must not require `B: Clone`.
impl<B: 'static> Clone for WeakRuntime<B> {
    fn clone(&self) -> Self {
        Self {
            inner: Weak::clone(&self.inner),
        }
    }
}

/// What bytes must satisfy before they become the guest an identity names:
/// the two admission policies, chosen by the caller of
/// [`Runtime::admit_bytes`].
#[derive(Clone, Copy, Debug)]
pub enum Policy<'a> {
    /// The deployment's entry is the authority: the bytes hash to its pin,
    /// a pre-compiled artifact is admitted as its source kind allows, the
    /// guest runs under its grants, and a racing first use that seated the
    /// name first stands.
    Declared(&'a Source),
    /// A caller named the bytes: raw wasm alone, hashing to `pin` when one
    /// is given, under a name the deployment does not declare, running under
    /// `grants`; a name already active attests on the same digest and
    /// grants alone.
    CallerNamed {
        /// The digest the bytes must hash to, when the caller knows it.
        pin: Option<Digest>,
        /// What of the deployment the admitted guest reaches.
        grants: &'a Grants,
    },
}

/// Why the runtime could not produce the guest an identity names, from
/// [`Runtime::guest`] or [`Runtime::admit_bytes`].
#[derive(Clone, Debug, thiserror::Error)]
pub enum GuestError {
    /// The deployment neither registered nor declares the identity.
    #[error("guest `{0}` is not registered")]
    Unregistered(GuestId),
    /// The source names nothing: no file is at its path, or nothing routes
    /// its package or holds the release.
    #[error("{0}")]
    NotFound(String),
    /// The deployment does not grant the bytes, such as caller-named bytes
    /// under a name it declares.
    #[error("{0}")]
    Denied(String),
    /// The request or the bytes are invalid. The bytes miss the
    /// deployment's pin, are a pre-compiled artifact where raw wasm alone is
    /// admitted, or do not load as a component against the deployment's
    /// host set.
    #[error("{0}")]
    Invalid(String),
    /// The runtime cannot serve the request, such as a package with no
    /// registry client to fetch it.
    #[error("{0}")]
    Unsupported(String),
    /// The source could not produce its bytes. A retry may succeed.
    #[error("{0}")]
    Unavailable(String),
    /// Caller-named bytes missed their pin, or the name is active under
    /// other bytes.
    #[error("{detail}")]
    Conflict {
        /// The digest of the bytes offered.
        found: Digest,
        /// What did not hold.
        detail: String,
    },
    /// The registration itself failed.
    #[error("{0}")]
    Internal(String),
}
