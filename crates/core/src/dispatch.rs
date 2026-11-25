//! Dynamic dispatch into a guest's exports, for a host binding and a guest's
//! link imports alike.
//!
//! A caller names its target and the export beside the arguments. The call
//! counts against the caller's dispatch chain and runs on a fresh instance of
//! its target.

use anyhow::{Context as _, Result, bail};
use futures::FutureExt as _;
use wasmtime::component::{Val, types};

use crate::chain::{ChainCtx, ChainPolicy};
use crate::host::{Backends, FutureResult};
use crate::invoke::{FreshCall, call_fresh};
use crate::registry::{Guest, GuestId};
use crate::runtime::Runtime;
use crate::value::handle_kind;

// A host binding's call and a link import's both take this path. The depth
// bound is checked before the target loads, so a refused call loads nothing.
// A root call waits for a slot of the gate, which frees once the callee's
// store has dropped.
pub async fn dispatch<B>(
    runtime: &Runtime<B>, caller: &ChainCtx, target: &GuestId, interface: Option<&str>, func: &str,
    args: Vec<Val>, root: bool,
) -> Result<Vec<Val>>
where
    B: Backends,
{
    let named = || {
        interface.map_or_else(|| format!("`{func}`"), |interface| format!("`{interface}/{func}`"))
    };

    // refuse what cannot cross, then enter the chain before anything loads
    if let Some(kind) = args.iter().find_map(handle_kind) {
        bail!(
            "a {kind} handle cannot cross the link seam (call to {}, target `{target}`)",
            named()
        );
    }
    let policy = ChainPolicy::from(runtime.options());
    let ctx = policy.enter(caller, target)?;

    // resolve the export on the component, so no store is touched before the callee task owns one
    let guest = runtime
        .guest(target)
        .await
        .with_context(|| format!("dispatching {} to guest `{target}`", named()))?;
    let interface = match interface {
        Some(interface) => Box::from(interface),
        None => find_interface(&guest, func)?,
    };
    let component = guest.component();
    let iface_idx = component
        .component_type()
        .get_export(component.engine(), &interface)
        .and_then(|_| component.get_export_index(None, &*interface))
        .with_context(|| format!("guest `{target}` exports no interface `{interface}`"))?;
    let (item, func_idx) = component.get_export(Some(&iface_idx), func).with_context(|| {
        format!("interface `{interface}` (guest `{target}`) exports no `{func}`")
    })?;
    let types::ComponentItem::ComponentFunc(func_ty) = item else {
        bail!("`{interface}/{func}` (guest `{target}`) is not a function");
    };

    let bound = (!ctx.uncapped).then_some(policy.timeout);
    let call = FreshCall {
        factory: runtime.store_factory(&guest),
        instance_pre: guest.instance_pre().clone(),
        export: func_idx,
        results: func_ty.results().count(),
    };
    let slot = if root { Some(runtime.slot().await) } else { None };
    call_fresh(call, slot, args, ctx, bound)
        .await
        .with_context(|| format!("dispatching `{interface}/{func}` to guest `{target}`"))
}

/// A capability to call a guest's exports, type-erased so a caller need not
/// name the [`Runtime`].
///
/// Every store context the runtime builds carries one, so a host binding
/// reaches it through [`StoreCtx::dispatcher`](crate::StoreCtx::dispatcher).
/// A guest's link imports call through the same one. It carries no consumer
/// vocabulary: a consumer owns its own verb names and return shapes and
/// composes this generic seam.
pub trait Dispatcher: Send + Sync + 'static {
    /// Call `target`'s `interface`/`func` with `args` on a fresh instance of
    /// `target`, returning the typed results.
    ///
    /// - The call is entered from `caller`, the chain context of the store
    ///   the host binding serves, and counts against the depth bound. A call
    ///   over the bound fails before its target loads.
    /// - A call beneath a server root must finish within `GUEST_TIMEOUT_MS`.
    /// - `interface` names an export exactly, so a semver-compatible name is
    ///   another interface. `None` discovers the unique exported interface
    ///   carrying a function named `func`.
    /// - A store-bound handle in `args` or the results fails the call.
    fn invoke(
        &self, caller: ChainCtx, target: GuestId, interface: Option<String>, func: String,
        args: Vec<Val>,
    ) -> FutureResult<Vec<Val>>;
}

impl<B: Backends> Dispatcher for crate::runtime::RuntimeDispatcher<B> {
    fn invoke(
        &self, caller: ChainCtx, target: GuestId, interface: Option<String>, func: String,
        args: Vec<Val>,
    ) -> FutureResult<Vec<Val>> {
        let runtime = self.runtime();
        let root = self.root();
        async move {
            dispatch(&runtime, &caller, &target, interface.as_deref(), &func, args, root).await
        }
        .boxed()
    }
}

// The unique exported interface carrying `func`; ambiguity is an error, never
// a silent first match.
fn find_interface<B: 'static>(guest: &Guest<B>, func: &str) -> Result<Box<str>> {
    let target = guest.id();
    let engine = guest.instance_pre().engine();
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
