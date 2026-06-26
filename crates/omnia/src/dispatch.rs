//! # Host-mediated dynamic linking
//!
//! A caller guest imports an interface (say `omnia:link/echo`) whose
//! implementation the host satisfies at runtime. The host has, on the shared
//! `Linker`, polyfilled that import so invoking it:
//!
//! 1. extracts a target identity from the call via a [`GuestSelector`] (§4.3),
//! 2. rejects any resource handle attempting to cross the seam (§4.5),
//! 3. enforces a dispatch-depth bound (§6.6),
//! 4. instantiates the target *fresh* on a new store and invokes the matching
//!    export over the bound wRPC transport (§4.2), and
//! 5. returns the typed result, discarding the callee instance.
//!
//! Because step 4 is always a fresh instance on a new store, a dispatched call
//! cannot recursively re-enter its caller — the guarantee falls out of the
//! design (`rfcs/guest-registry.md` §4.1).
//!
//! The floor stays generic (Law 2): it links whatever interfaces the manifest
//! names, by opaque string, and resolves opaque [`GuestId`]s. It never parses
//! `augentic:specify` or any consumer scheme.
//!
//! ## Where the selector runs
//!
//! The selector must see the *typed* parameters, so the polyfill is a
//! `func_new_async` closure that runs the selector *before* encoding the call
//! onto wRPC (§4.4 step 3) — then reuses wRPC's own value codec
//! ([`ValEncoder`]/[`read_value`]) and instance-per-call serve integration
//! ([`ServeExt::serve_function`]) for the actual carrier round-trip.

// This module declares a few crate-internal helpers (`link_dynamic`, the
// dispatch-handle constructor) as `pub(crate)`. That is deliberate: `lib.rs`
// re-exports the module's public items with a glob, so `pub` would leak them
// into the crate's API. The nursery lint's `pub` suggestion is wrong here.
#![allow(clippy::redundant_pub_crate)]

use std::collections::{BTreeSet, HashMap};
use std::iter::zip;
use std::pin::pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use anyhow::{Context as _, Result, bail, ensure};
use bytes::BytesMut;
use futures::{FutureExt as _, StreamExt as _};
use tokio_util::codec::Encoder as _;
use wasmtime::component::{Linker, Type, Val, types};
use wasmtime::{AsContextMut as _, Engine, StoreContextMut};
use wasmtime_wasi::WasiView;
use wrpc_transport::Invoke;
use wrpc_wasmtime::{ServeExt as _, ValEncoder, WrpcView, read_value};

use crate::registry::GuestId;
use crate::selector::GuestSelector;
use crate::source::LoadedGuest;
use crate::traits::{FutureResult, Runtime};
use crate::transport::{InProcClient, InProcServer, InProcess, LinkTransport as _};

/// wRPC host-resource map shape (empty for the resource-free dynamic path).
type HostResources = HashMap<
    Box<str>,
    HashMap<Box<str>, (wasmtime::component::ResourceType, wasmtime::component::ResourceType)>,
>;

/// The long-lived dispatch state shared by every polyfilled import.
///
/// It carries the selector strategy, the union of host-mediated interfaces, the
/// bound transport (installed once the serve side is wired), and the
/// process-wide dispatch-depth counter.
pub struct DispatchHandle {
    selector: Arc<dyn GuestSelector>,
    links: BTreeSet<Box<str>>,
    transport: OnceLock<InProcess>,
    depth: AtomicUsize,
    max_depth: usize,
}

impl DispatchHandle {
    /// Create a shared dispatch handle. The transport is installed later by
    /// [`serve_links`], once each target's serve side is wired.
    #[must_use]
    pub(crate) fn new(
        selector: Arc<dyn GuestSelector>, links: BTreeSet<Box<str>>, max_depth: usize,
    ) -> Arc<Self> {
        Arc::new(Self {
            selector,
            links,
            transport: OnceLock::new(),
            depth: AtomicUsize::new(0),
            max_depth,
        })
    }

    /// The union of host-mediated interface names across every guest's `link`
    /// allow-list — the set of interfaces to polyfill (caller side) and serve
    /// (callee side).
    #[must_use]
    pub(crate) const fn links(&self) -> &BTreeSet<Box<str>> {
        &self.links
    }

    /// Install the bound transport carrier (called once by [`serve_links`]).
    fn install(&self, transport: InProcess) {
        // Set-once: a second install (there is only ever one) is ignored.
        let _ = self.transport.set(transport);
    }

    /// The bound transport carrier.
    fn transport(&self) -> Result<&InProcess> {
        self.transport
            .get()
            .context("link transport not initialized; `serve_links` must run before dispatch")
    }

    /// Enter a dispatch, bounding nesting depth (§6.6). The returned guard
    /// decrements the shared counter on drop.
    ///
    /// The counter is process-wide and tracks *synchronous* nesting (A->B->C,
    /// each awaited to completion before the caller returns), which is the
    /// unbounded-recursion concern; it is a safety bound, not a precise
    /// per-chain limit under heavy concurrency.
    fn enter(&self, target: &GuestId) -> Result<DepthGuard<'_>> {
        let depth = self.depth.fetch_add(1, Ordering::SeqCst) + 1;
        if depth > self.max_depth {
            self.depth.fetch_sub(1, Ordering::SeqCst);
            bail!(
                "link dispatch depth {depth} exceeds maximum {} (target `{target}`); raise \
                 MAX_DISPATCH_DEPTH if this is intentional",
                self.max_depth
            );
        }
        Ok(DepthGuard { depth: &self.depth })
    }
}

/// Decrements the shared dispatch-depth counter when a dispatch unwinds.
#[derive(Debug)]
struct DepthGuard<'a> {
    depth: &'a AtomicUsize,
}

impl Drop for DepthGuard<'_> {
    fn drop(&mut self) {
        self.depth.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Polyfill every host-mediated import named in the `link` allow-list union onto
/// the shared linker, bound to the dispatch handle.
///
/// Each interface is linked exactly once (the linker is shared, so the per-guest
/// allow-lists are unioned, per §4.4). `wasi:*` imports are never touched here —
/// they are host-satisfied — so only the manifest-declared interfaces are
/// dispatched.
///
/// Runs *before* pre-instantiation, so an import that is neither host-satisfied
/// nor allow-listed remains unresolved and fails at `instantiate_pre` — the
/// explicit, fail-fast startup error of §4.4/§6.4.
///
/// # Errors
///
/// Returns an error if a named link target is not an interface import, or if a
/// function cannot be defined on the linker.
pub(crate) fn link_dynamic<T>(
    engine: &Engine, linker: &mut Linker<T>, guests: &[LoadedGuest], handle: &Arc<DispatchHandle>,
) -> Result<()>
where
    T: WasiView + WrpcView + 'static,
{
    if handle.links.is_empty() {
        return Ok(());
    }

    let mut wired: BTreeSet<Box<str>> = BTreeSet::new();
    for LoadedGuest { id, component } in guests {
        let component_ty = component.component_type();
        for (name, types::ComponentExtern { ty, .. }) in component_ty.imports(engine) {
            if !handle.links.contains(name) || wired.contains(name) {
                continue;
            }
            let types::ComponentItem::ComponentInstance(instance_ty) = ty else {
                bail!("link target `{name}` (imported by guest `{id}`) is not an interface");
            };

            // Snapshot the interface's function types before mutably borrowing
            // the linker.
            let funcs: Vec<Arc<str>> = instance_ty
                .exports(engine)
                .filter_map(|(func, types::ComponentExtern { ty, .. })| {
                    matches!(ty, types::ComponentItem::ComponentFunc(_)).then(|| Arc::from(func))
                })
                .collect();

            let mut root = linker.root();
            let mut interface = root
                .instance(name)
                .map_err(anyhow::Error::from)
                .with_context(|| format!("defining host-mediated interface `{name}`"))?;
            let iface_name: Arc<str> = Arc::from(name);

            for func in &funcs {
                let handle = Arc::clone(handle);
                let iface_name = Arc::clone(&iface_name);
                let func_name = Arc::clone(func);
                interface
                    .func_new_async(func, move |store, ty, params, results| {
                        let handle = Arc::clone(&handle);
                        let iface_name = Arc::clone(&iface_name);
                        let func_name = Arc::clone(&func_name);
                        Box::new(async move {
                            dispatch(store, &handle, &iface_name, &func_name, &ty, params, results)
                                .await
                                .map_err(wasmtime::Error::from_anyhow)
                        })
                    })
                    .map_err(anyhow::Error::from)
                    .with_context(|| format!("polyfilling `{name}` function `{func}`"))?;
            }
            wired.insert(Box::from(name));
        }
    }
    Ok(())
}

/// The per-call dispatch: select the target, reject crossing resources, bound
/// depth, then round-trip the call over the in-process wRPC carrier to a
/// freshly-instantiated target export.
async fn dispatch<T>(
    mut store: StoreContextMut<'_, T>, handle: &DispatchHandle, interface: &str, func: &str,
    ty: &types::ComponentFunc, params: &[Val], results: &mut [Val],
) -> Result<()>
where
    T: WrpcView + 'static,
{
    let start = std::time::Instant::now();

    let (target, forwarded) = handle
        .selector
        .select(interface, func, params)
        .with_context(|| format!("selecting target for `{interface}/{func}`"))?;

    // §4.5: plain records cross by value; a live resource handle never crosses.
    for value in &forwarded {
        if contains_resource(value) {
            bail!(
                "a resource handle cannot cross the link seam (call to `{interface}/{func}`, \
                 target `{target}`)"
            );
        }
    }

    let _guard = handle.enter(&target)?;

    let param_types: Vec<Type> = ty.params().map(|(_, ty)| ty).collect();
    let result_types: Vec<Type> = ty.results().collect();
    ensure!(
        forwarded.len() == param_types.len(),
        "selector forwarded {} arguments but `{interface}/{func}` expects {}",
        forwarded.len(),
        param_types.len()
    );

    let client = handle.transport()?.connect(&target)?;

    // Encode the forwarded parameters with wRPC's value codec.
    let mut buf = BytesMut::new();
    for (value, ty) in zip(&forwarded, &param_types) {
        let mut encoder: ValEncoder<'_, T, <InProcClient as Invoke>::Outgoing> =
            ValEncoder::new(store.as_context_mut(), ty, &[], &[]);
        encoder
            .encode(value, &mut buf)
            .map_err(anyhow::Error::from)
            .with_context(|| format!("encoding parameter for `{interface}/{func}`"))?;
        ensure!(
            encoder.deferred.is_none(),
            "async/stream parameters cannot cross the link seam (`{interface}/{func}`)"
        );
    }

    // Invoke over the carrier; the request is written and flushed here, the
    // results stream back on `incoming`. No deferred (async) parameters, so the
    // outgoing half carries nothing further and is dropped.
    let (_outgoing, incoming) = client
        .invoke((), interface, func, buf.freeze(), &[[]; 0])
        .await
        .with_context(|| format!("invoking link target `{target}` for `{interface}/{func}`"))?;

    let mut incoming = pin!(incoming);
    for (index, (value, ty)) in zip(results.iter_mut(), &result_types).enumerate() {
        read_value(&mut store, &mut incoming, &[], &[], value, ty, &[index])
            .await
            .map_err(anyhow::Error::from)
            .with_context(|| format!("decoding result {index} from `{target}`"))?;
    }

    let elapsed_us = u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX);
    tracing::debug!(
        target = %target,
        interface,
        func,
        transport = "in-process",
        histogram.link_dispatch_duration_us = elapsed_us,
        monotonic_counter.link_dispatches = 1_u64,
        "dispatched host-mediated call",
    );
    Ok(())
}

/// Host-originated dynamic dispatch into a *known* guest export — the host→guest
/// counterpart of [`dispatch`] (which is guest→guest and selector-driven).
///
/// This reuses the landed machinery rather than adding a parallel one: the same
/// dispatch-depth bound ([`DispatchHandle::enter`]) so a `complete`→`resolve`
/// →adapter chain is depth-counted exactly like a guest→guest hop, and the same
/// §4.5 resource rejection. The target is instantiated *fresh* on a new store and
/// the matching export invoked directly (the `wasi-http` `server.rs` pattern), so
/// the callee can never re-enter its caller (instance-per-call) and needs no
/// `link` declaration for `interface`.
///
/// `args` and the returned values are plain `Val`s; a live resource handle on
/// either side is rejected.
///
/// # Errors
///
/// Returns an error if the depth bound is exceeded, an argument or result carries
/// a resource handle, the target is not registered, the named `interface`/`func`
/// export is absent or is not a function, or the guest call traps.
pub async fn dispatch_to_guest<R>(
    runtime: &R, target: &GuestId, interface: &str, func: &str, args: Vec<Val>,
) -> Result<Vec<Val>>
where
    R: Runtime,
{
    // Depth-count this hop exactly like a guest→guest dispatch (§6.6). The guard
    // is held here (borrowing `runtime`) across the awaited callee task below.
    let _guard = runtime.registry().dispatch().enter(target)?;

    // §4.5: plain records cross by value; a live resource handle never crosses.
    for value in &args {
        if contains_resource(value) {
            bail!(
                "a resource handle cannot cross the link seam \
                 (host→guest `{interface}/{func}`, target `{target}`)"
            );
        }
    }

    let instance_pre = runtime
        .registry()
        .get(target)
        .with_context(|| format!("dispatch target `{target}` is not registered"))?
        .instance_pre()
        .clone();

    // Run the callee on its own task. `resolve` is invoked from *within* the
    // caller guest's concurrent event loop (the backend's loop awaits it inside
    // the `complete` host call), and wasmtime forbids a recursive
    // `StoreContextMut::run_concurrent` on the same thread. Spawning gives the
    // callee a fresh event loop: when the caller's loop parks awaiting this task,
    // its ambient store clears, so the callee's call runs unnested. The task owns
    // the whole store lifecycle (build → instantiate → call → drop), so the
    // callee is a fresh instance that cannot re-enter its caller
    // (instance-per-call) and needs no `link` declaration for `interface`.
    let task_runtime = (*runtime).clone();
    let target_owned = target.clone();
    let interface_owned = interface.to_owned();
    let func_owned = func.to_owned();
    let results = tokio::spawn(async move {
        let mut store = task_runtime.build_store(task_runtime.store());
        let instance = task_runtime
            .instantiate(&instance_pre, &mut store)
            .await
            .with_context(|| format!("instantiating dispatch target `{target_owned}`"))?;

        let interface_idx =
            instance.get_export_index(&mut store, None, &interface_owned).with_context(|| {
                format!("guest `{target_owned}` exports no interface `{interface_owned}`")
            })?;
        let (item, func_idx) = instance
            .get_export(&mut store, Some(&interface_idx), &func_owned)
            .with_context(|| {
            format!(
                "interface `{interface_owned}` (guest `{target_owned}`) exports no \
                     `{func_owned}`"
            )
        })?;
        let types::ComponentItem::ComponentFunc(func_ty) = item else {
            bail!("`{interface_owned}/{func_owned}` (guest `{target_owned}`) is not a function");
        };
        let result_count = func_ty.results().count();
        let function = instance.get_func(&mut store, func_idx).with_context(|| {
            format!("resolving `{interface_owned}/{func_owned}` on guest `{target_owned}`")
        })?;

        let mut results = vec![Val::Bool(false); result_count];
        function
            .call_async(&mut store, &args, &mut results)
            .await
            .map_err(anyhow::Error::from)
            .with_context(|| {
                format!("calling `{interface_owned}/{func_owned}` on guest `{target_owned}`")
            })?;
        Ok::<Vec<Val>, anyhow::Error>(results)
    })
    .await
    .with_context(|| format!("joining dispatch target `{target}` task"))?
    .with_context(|| format!("dispatching `{interface}/{func}` to guest `{target}`"))?;

    // A target must not hand back a resource handle either (§4.5).
    for value in &results {
        if contains_resource(value) {
            bail!(
                "a resource handle cannot cross the link seam \
                 (result of `{interface}/{func}`, target `{target}`)"
            );
        }
    }

    Ok(results)
}

/// The conventional export-function name a `references` shelf exposes for
/// host-mediated `resolve`. This is a floor concept — it mirrors `ToolHost`'s
/// `resolve` and the RFC tool name — so the floor invokes it without baking in a
/// consumer package/interface name (Law 2).
const RESOLVE_FUNC: &str = "resolve";

/// A host→guest call capability, type-erased so a host binding (e.g. `wasi-model`'s
/// `resolve`) can invoke a guest without naming the concrete [`Runtime`].
///
/// Implemented blanket for every [`Runtime`]; the `runtime!` macro threads an
/// `Arc<dyn HostDispatch>` into each store context (like the per-store wRPC state)
/// so host bindings can reach it.
pub trait HostDispatch: Send + Sync + 'static {
    /// Resolve a reference against a guest's `references` shelf: instantiate
    /// `target` fresh, invoke its exported [`RESOLVE_FUNC`] function with
    /// `reference`, and return the typed bytes. Always a fresh instance
    /// (instance-per-call), depth-bounded like any host-mediated hop.
    fn resolve(&self, target: GuestId, reference: String) -> FutureResult<Vec<u8>>;
}

impl<R: Runtime> HostDispatch for R {
    fn resolve(&self, target: GuestId, reference: String) -> FutureResult<Vec<u8>> {
        let runtime = self.clone();
        async move {
            let interface = resolve_interface(&runtime, &target)?;
            let results = dispatch_to_guest(
                &runtime,
                &target,
                &interface,
                RESOLVE_FUNC,
                vec![Val::String(reference)],
            )
            .await?;
            vals_to_bytes(results)
        }
        .boxed()
    }
}

/// Find the exported interface on `target`'s component that carries a
/// [`RESOLVE_FUNC`] function, so the floor invokes it without hardcoding a
/// consumer interface name.
fn resolve_interface<R: Runtime>(runtime: &R, target: &GuestId) -> Result<Box<str>> {
    let registry = runtime.registry();
    let engine = registry.engine();
    let guest = registry
        .get(target)
        .with_context(|| format!("resolve target `{target}` is not registered"))?;
    let component_ty = guest.component().component_type();
    for (interface, types::ComponentExtern { ty, .. }) in component_ty.exports(engine) {
        let types::ComponentItem::ComponentInstance(instance_ty) = ty else {
            continue;
        };
        let has_resolve =
            instance_ty.exports(engine).any(|(func, types::ComponentExtern { ty, .. })| {
                func == RESOLVE_FUNC && matches!(ty, types::ComponentItem::ComponentFunc(_))
            });
        if has_resolve {
            return Ok(Box::from(interface));
        }
    }
    bail!("resolve target `{target}` exports no interface with a `{RESOLVE_FUNC}` function")
}

/// Convert a `resolve` export's return value into raw bytes. Accepts `list<u8>`
/// (the canonical shape) or `string` (a convenience for text shelves).
fn vals_to_bytes(results: Vec<Val>) -> Result<Vec<u8>> {
    let first = results.into_iter().next().context("resolve export returned no value")?;
    match first {
        Val::List(items) => items
            .into_iter()
            .map(|value| match value {
                Val::U8(byte) => Ok(byte),
                other => bail!("resolve result list element is not a u8: {other:?}"),
            })
            .collect(),
        Val::String(text) => Ok(text.into_bytes()),
        other => bail!("resolve export must return list<u8> or string, got {other:?}"),
    }
}

/// Recursively reports whether a value carries a live resource handle.
fn contains_resource(value: &Val) -> bool {
    match value {
        Val::Resource(_) => true,
        Val::List(values) | Val::Tuple(values) => values.iter().any(contains_resource),
        Val::Record(fields) => fields.iter().any(|(_, value)| contains_resource(value)),
        Val::Variant(_, Some(value))
        | Val::Option(Some(value))
        | Val::Result(Ok(Some(value)) | Err(Some(value))) => contains_resource(value),
        _ => false,
    }
}

/// Wire the serve side of every host-mediated interface.
///
/// Each target guest that exports a linked interface runs a wRPC server whose
/// handlers instantiate the guest *fresh per call* (instance-per-call); the
/// bound transport is then installed so polyfilled imports can reach it.
///
/// Spawns one detached task per served function to drain its invocation stream.
/// A no-op when no guest declares any `link` interface.
///
/// # Errors
///
/// Returns an error if a guest's export cannot be served over the carrier.
pub async fn serve_links<R>(state: &R) -> Result<()>
where
    R: Runtime,
    R::StoreCtx: WasiView + WrpcView + 'static,
{
    let registry = state.registry();
    let handle = registry.dispatch();
    if handle.links().is_empty() {
        return Ok(());
    }
    let engine = registry.engine().clone();

    let mut servers: HashMap<GuestId, Arc<InProcServer>> = HashMap::new();
    for guest in registry.guests() {
        let component_ty = guest.component().component_type();
        let mut server: Option<Arc<InProcServer>> = None;

        for (interface, types::ComponentExtern { ty, .. }) in component_ty.exports(&engine) {
            if !handle.links().contains(interface) {
                continue;
            }
            let types::ComponentItem::ComponentInstance(instance_ty) = ty else {
                continue;
            };
            for (func, types::ComponentExtern { ty, .. }) in instance_ty.exports(&engine) {
                let types::ComponentItem::ComponentFunc(func_ty) = ty else {
                    continue;
                };
                let server =
                    Arc::clone(server.get_or_insert_with(|| Arc::new(InProcServer::default())));
                let runtime = state.clone();
                let factory = move || runtime.build_store(runtime.store());
                let stream = server
                    .serve_function(
                        factory,
                        guest.instance_pre().clone(),
                        Arc::<HostResources>::default(),
                        func_ty,
                        interface,
                        func,
                    )
                    .await
                    .with_context(|| {
                        format!("serving `{interface}/{func}` from guest `{}`", guest.id())
                    })?;

                tokio::spawn(async move {
                    let mut stream = pin!(stream);
                    while let Some(invocation) = stream.next().await {
                        match invocation {
                            Ok((_cx, fut)) => {
                                tokio::spawn(async move {
                                    if let Err(error) = fut.await {
                                        tracing::error!(%error, "link serve invocation failed");
                                    }
                                });
                            }
                            Err(error) => tracing::error!(%error, "link serve accept failed"),
                        }
                    }
                });
            }
        }

        if let Some(server) = server {
            servers.insert(guest.id().clone(), server);
        }
    }

    handle.install(InProcess::new(servers));
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    use wasmtime::component::Val;

    use super::{DispatchHandle, contains_resource};
    use crate::registry::GuestId;
    use crate::selector::FirstArgSelector;

    fn handle(max_depth: usize) -> Arc<DispatchHandle> {
        DispatchHandle::new(Arc::new(FirstArgSelector), std::iter::empty().collect(), max_depth)
    }

    #[test]
    fn depth_guard_bounds_nesting() {
        let handle = handle(2);
        let target = GuestId::from("t");

        let first = handle.enter(&target).expect("depth 1 within bound");
        let second = handle.enter(&target).expect("depth 2 within bound");
        handle.enter(&target).expect_err("depth 3 exceeds the maximum");

        // Unwinding the guards frees the budget again.
        drop(second);
        drop(first);
        assert_eq!(handle.depth.load(Ordering::SeqCst), 0);
        handle.enter(&target).expect("budget freed after guards drop");
    }

    #[test]
    fn detects_nested_resources() {
        // Plain values never count as resources.
        assert!(!contains_resource(&Val::String("x".to_owned())));
        assert!(!contains_resource(&Val::Record(vec![("f".to_owned(), Val::U32(1),)])));
        assert!(!contains_resource(&Val::Option(None)));
        // A nested option/list carrying plain values stays clean.
        assert!(!contains_resource(&Val::List(vec![Val::Bool(true), Val::Bool(false)])));
    }
}
