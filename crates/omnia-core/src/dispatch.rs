//! Host-originated dynamic dispatch into guest exports.
//!
//! The host→guest counterpart of guest→guest linking: a host binding invokes a
//! *known* guest's export by identity, with no selector or declared link
//! interface involved. The hop is depth-counted and wall-clock-bounded exactly
//! like a guest→guest hop, and is driven by the same [`call_fresh`] primitive.

use anyhow::{Context as _, Result, bail};
use futures::FutureExt as _;
use wasmtime::component::{Val, types};

use crate::chain::ChainPolicy;
use crate::host::FutureResult;
use crate::invoke::{FreshCall, call_fresh};
use crate::registry::GuestId;
use crate::runtime::Runtime;
use crate::value::contains_handle;

/// Host-originated dynamic dispatch into a *known* guest export — the host→guest
/// counterpart of the selector-driven guest→guest `dispatch`.
///
/// Shares the depth bound (`ChainPolicy::enter`), the wall-clock bound on
/// server-rooted chains and the handle rejection with guest→guest dispatch. The
/// target is instantiated *fresh* on a new store and the matching export
/// invoked directly, so the callee can never re-enter its caller and needs no
/// declared link interface for `interface`.
///
/// `args` and the returned values are plain `Val`s; a store-bound handle on
/// either side is rejected.
///
/// # Errors
///
/// Returns an error if the depth bound is exceeded, an argument or result carries
/// a store-bound handle, the target is not registered, the named
/// `interface`/`func` export is absent or is not a function, the call exceeds
/// the wall-clock bound, or the guest call traps.
pub async fn dispatch<B>(
    runtime: &Runtime<B>, target: &GuestId, interface: &str, func: &str, args: Vec<Val>,
) -> Result<Vec<Val>>
where
    B: Clone + Send + Sync + 'static,
{
    // Plain records cross by value; a live resource handle never crosses.
    for value in &args {
        if contains_handle(value) {
            bail!(
                "a resource handle cannot cross the link seam \
                 (host→guest `{interface}/{func}`, target `{target}`)"
            );
        }
    }

    let instance_pre = runtime
        .registry()
        .get(target)
        .with_context(|| {
            format!("dispatching `{interface}/{func}` to guest `{target}`: guest is not registered")
        })?
        .instance_pre()
        .clone();

    // Resolve the export on the component so no store is touched before the
    // callee task owns one.
    let component = instance_pre.component();
    let iface_idx = component
        .get_export_index(None, interface)
        .with_context(|| format!("guest `{target}` exports no interface `{interface}`"))?;
    let (item, func_idx) = component.get_export(Some(&iface_idx), func).with_context(|| {
        format!("interface `{interface}` (guest `{target}`) exports no `{func}`")
    })?;
    let types::ComponentItem::ComponentFunc(func_ty) = item else {
        bail!("`{interface}/{func}` (guest `{target}`) is not a function");
    };

    let policy = ChainPolicy::from(runtime.options());
    let ctx = policy.enter(target)?;
    let bound = (!ctx.uncapped).then_some(policy.timeout);
    let call = FreshCall {
        factory: runtime.store_factory(),
        instance_pre,
        export: func_idx,
        results: func_ty.results().count(),
    };
    call_fresh(call, args, ctx, bound)
        .await
        .with_context(|| format!("dispatching `{interface}/{func}` to guest `{target}`"))
}

/// A host→guest call capability, type-erased so a host binding can invoke a
/// guest without naming the concrete [`Runtime`].
///
/// The `runtime!` macro threads an `Arc<dyn Dispatcher>` into each store
/// context so any host binding gets dynamic host→guest calls for free. It
/// carries no consumer vocabulary — a consumer owns its own verb names and
/// return shapes and composes this generic seam.
pub trait Dispatcher: Send + Sync + 'static {
    /// Invoke `target`'s `interface`/`func` with `args`, returning the typed
    /// results. The target is instantiated *fresh* (instance-per-call), the hop
    /// is depth-bounded like any host-mediated call, and a live resource handle
    /// on either side is rejected.
    ///
    /// A `None` `interface` discovers the unique exported interface carrying a
    /// function named `func` — a structural component-model query that names no
    /// consumer scheme.
    fn invoke(
        &self, target: GuestId, interface: Option<String>, func: String, args: Vec<Val>,
    ) -> FutureResult<Vec<Val>>;
}

impl<B: Clone + Send + Sync + 'static> Dispatcher for crate::runtime::RuntimeDispatcher<B> {
    fn invoke(
        &self, target: GuestId, interface: Option<String>, func: String, args: Vec<Val>,
    ) -> FutureResult<Vec<Val>> {
        let runtime = self.runtime();
        async move {
            let interface: Box<str> = match interface {
                Some(name) => Box::from(name),
                None => find_interface(&runtime, &target, &func)?,
            };
            dispatch(&runtime, &target, &interface, &func, args).await
        }
        .boxed()
    }
}

/// Find the *unique* exported interface on `target`'s component that carries a
/// function named `func`, so a host can invoke it without hardcoding a
/// consumer interface name. Ambiguity is an error, never a silent first-match.
fn find_interface<B: Clone + Send + Sync + 'static>(
    runtime: &Runtime<B>, target: &GuestId, func: &str,
) -> Result<Box<str>> {
    let registry = runtime.registry();
    let engine = registry.engine();
    let guest = registry
        .get(target)
        .with_context(|| format!("dispatch target `{target}` is not registered"))?;
    let component_ty = guest.component().component_type();
    let mut matches: Vec<Box<str>> = Vec::new();
    for (interface, types::ComponentExtern { ty, .. }) in component_ty.exports(engine) {
        let types::ComponentItem::ComponentInstance(instance_ty) = ty else {
            continue;
        };
        let has_func =
            instance_ty.exports(engine).any(|(name, types::ComponentExtern { ty, .. })| {
                name == func && matches!(ty, types::ComponentItem::ComponentFunc(_))
            });
        if has_func {
            matches.push(Box::from(interface));
        }
    }
    match matches.len() {
        0 => bail!("dispatch target `{target}` exports no interface with a `{func}` function"),
        1 => Ok(matches.remove(0)),
        _ => bail!(
            "dispatch target `{target}` exports `{func}` from several interfaces ({}); name the \
             interface explicitly",
            matches.join(", ")
        ),
    }
}
