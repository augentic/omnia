//! Linker polyfill for host-mediated imports.

use std::collections::{BTreeMap, BTreeSet};
use std::iter::zip;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use omnia_core::{ChainPolicy, GuestId, InvokeError, contains_handle, plain_signature};
use wasmtime::Engine;
use wasmtime::component::{Linker, Val, types};

use super::route::Routes;
use super::selector::GuestSelector;

/// The functions polyfilled onto a linker — the union across guests at
/// function granularity, since components import only the functions they use
/// and so per-guest imports of one interface are arbitrary subsets. Keyed by
/// interface then function name; the value records how the first importer
/// declared the function, so a later guest whose import disagrees on
/// asyncness is rejected instead of failing wasmtime's pre-instantiation
/// typecheck with no cross-guest context, and an exporter's signature can be
/// checked against the importer's when it is served.
pub type WiredLinks = BTreeMap<Box<str>, BTreeMap<Box<str>, Wired>>;

/// One polyfilled function as its first importer declared it.
#[derive(Clone)]
pub struct Wired {
    pub is_async: bool,
    pub ty: types::ComponentFunc,
    pub importer: GuestId,
}

/// The caller-side state every polyfilled import shares: the selector
/// strategy, the chain policy, and the live route table.
pub struct Caller {
    pub selector: Arc<dyn GuestSelector>,
    pub policy: ChainPolicy,
    pub routes: Routes,
}

/// Polyfill one component's imports of the declared `interfaces` not already
/// in `wired`, bound to `caller`.
///
/// Each function is linked exactly once (the linker is shared, so the
/// per-guest imports are unioned function-by-function, reopening an
/// interface's [`LinkerInstance`](wasmtime::component::LinkerInstance) as
/// later guests add functions). `wasi:*` imports are never touched here —
/// they are host-satisfied — so only the manifest-declared interfaces are
/// dispatched. Runs *before* pre-instantiation, so an import that is neither
/// host-satisfied nor allow-listed remains unresolved and fails fast at
/// `instantiate_pre`.
///
/// Registration matches the import's type-level asyncness: a plain `func` is
/// polyfilled with `func_new_async`, an `async func` with
/// `func_new_concurrent` — the sync-typed registration would fail the
/// pre-instantiation asyncness typecheck. Both share one body ([`relay`]),
/// which never touches the caller's store. A function an earlier guest wired
/// with the *other* asyncness is a cross-guest interface disagreement,
/// rejected here with both views named. A signature carrying a store-bound
/// handle (resource, future, stream, error-context) is refused before it is
/// wired: only plain values cross the seam.
///
/// # Errors
///
/// Returns an error if a named link target is not an interface import, a
/// function's signature is not plain, or a function cannot be defined on the
/// linker.
pub fn polyfill_component<T: 'static>(
    engine: &Engine, linker: &mut Linker<T>, id: &GuestId,
    component: &wasmtime::component::Component, interfaces: &BTreeSet<Box<str>>,
    caller: &Arc<Caller>, wired: &mut WiredLinks,
) -> Result<()> {
    let component_ty = component.component_type();
    for (name, types::ComponentExtern { ty, .. }) in component_ty.imports(engine) {
        if !interfaces.contains(name) {
            continue;
        }
        let types::ComponentItem::ComponentInstance(instance_ty) = ty else {
            bail!("link target `{name}` (imported by guest `{id}`) is not an interface");
        };

        // Snapshot the missing function names and asyncness before mutably
        // borrowing the linker, skipping functions an earlier guest wired.
        let wired_funcs = wired.entry(Box::from(name)).or_default();
        let describe = |is_async: bool| if is_async { "an async func" } else { "a plain func" };
        let mut funcs: Vec<(Arc<str>, types::ComponentFunc)> = Vec::new();
        for (func, types::ComponentExtern { ty, .. }) in instance_ty.exports(engine) {
            let types::ComponentItem::ComponentFunc(ty) = ty else {
                continue;
            };
            let is_async = ty.async_();
            match wired_funcs.get(func) {
                Some(earlier) if earlier.is_async == is_async => {}
                Some(earlier) => bail!(
                    "guest `{id}` imports `{name}/{func}` as {}, but an earlier guest wired it \
                     as {}; every importer of a host-mediated function must agree on asyncness",
                    describe(is_async),
                    describe(earlier.is_async),
                ),
                None => {
                    if let Err(kind) = plain_signature(&ty) {
                        bail!(
                            "guest `{id}` imports `{name}/{func}` whose signature carries a \
                             {kind}; only plain values cross the link seam"
                        );
                    }
                    funcs.push((Arc::from(func), ty));
                }
            }
        }

        // Opening the instance also (re)defines it on the linker, so an
        // allow-listed interface resolves even when every function is already
        // wired (or it has none).
        let mut root = linker.root();
        let mut interface = root
            .instance(name)
            .map_err(anyhow::Error::from)
            .with_context(|| format!("defining host-mediated interface `{name}`"))?;
        let iface_name: Arc<str> = Arc::from(name);

        for (func, ty) in &funcs {
            let caller = Arc::clone(caller);
            let iface_name = Arc::clone(&iface_name);
            let func_name = Arc::clone(func);
            let registered = if ty.async_() {
                interface.func_new_concurrent(func, move |_accessor, ty, params, results| {
                    let caller = Arc::clone(&caller);
                    let iface_name = Arc::clone(&iface_name);
                    let func_name = Arc::clone(&func_name);
                    Box::pin(async move {
                        relay(&caller, &iface_name, &func_name, &ty, params, results)
                            .await
                            .map_err(wasmtime::Error::from_anyhow)
                    })
                })
            } else {
                interface.func_new_async(func, move |_store, ty, params, results| {
                    let caller = Arc::clone(&caller);
                    let iface_name = Arc::clone(&iface_name);
                    let func_name = Arc::clone(&func_name);
                    Box::new(async move {
                        relay(&caller, &iface_name, &func_name, &ty, params, results)
                            .await
                            .map_err(wasmtime::Error::from_anyhow)
                    })
                })
            };
            registered
                .map_err(anyhow::Error::from)
                .with_context(|| format!("polyfilling `{name}` function `{func}`"))?;
        }
        wired_funcs.extend(funcs.into_iter().map(|(func, ty)| {
            let wired = Wired {
                is_async: ty.async_(),
                ty,
                importer: id.clone(),
            };
            (Box::from(&*func), wired)
        }));
    }
    Ok(())
}

/// The per-call dispatch: select the target, reject crossing handles, take a
/// depth slot, resolve the live route, and move the lifted parameters to a
/// fresh callee instance on its own task, writing its results back.
async fn relay(
    caller: &Caller, interface: &str, func: &str, ty: &types::ComponentFunc, params: &[Val],
    results: &mut [Val],
) -> Result<()> {
    let start = Instant::now();

    let (target, forwarded) = caller
        .selector
        .select(interface, func, params)
        .with_context(|| format!("selecting target for `{interface}/{func}`"))?;

    // Plain records cross by value; a live resource handle never crosses.
    for value in &*forwarded {
        if contains_handle(value) {
            bail!(
                "a resource handle cannot cross the link seam (call to `{interface}/{func}`, \
                 target `{target}`)"
            );
        }
    }

    let ctx = caller.policy.enter(&target)?;

    let expected = ty.params().len();
    ensure!(
        forwarded.len() == expected,
        "selector forwarded {} arguments but `{interface}/{func}` expects {expected}",
        forwarded.len(),
    );

    let route = caller.routes.resolve(&target, interface)?;
    // A server-rooted chain is wall-clock bounded so a hung target cannot stall
    // the caller; a command-rooted chain runs uncapped.
    let bound = (!ctx.uncapped).then_some(caller.policy.timeout);
    let out =
        route.invoke(interface, func, forwarded.into_owned(), ctx, bound).await.map_err(|err| {
            match err.downcast_ref::<InvokeError>() {
                Some(InvokeError::Timeout(bound)) => anyhow!(
                    "link dispatch to `{target}` for `{interface}/{func}` timed out after {bound:?}"
                ),
                _ => err,
            }
        })?;
    for (slot, value) in zip(results, out) {
        *slot = value;
    }

    let elapsed_us = u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX);
    tracing::debug!(
        target = %target,
        interface,
        func,
        depth = ctx.depth,
        carrier = "in-memory",
        histogram.link_dispatch_duration_us = elapsed_us,
        monotonic_counter.link_dispatches = 1_u64,
        "dispatched host-mediated call",
    );
    Ok(())
}
