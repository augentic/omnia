//! Route construction for a guest's host-mediated exports.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use anyhow::{Context as _, Result, ensure};
use omnia_core::{GuestId, StoreFactory};
use wasmtime::component::{InstancePre, types};

use super::polyfill::{Wired, WiredLinks};
use super::route::{Linked, Route, RouteInvoke, Routes};

/// Resolve one guest's exports of the declared `interfaces` into a route and
/// park it as pending on `routes` — with no route when the guest exports none
/// of them, so a later call to it is diagnosed as "registered but unlinked"
/// rather than "not registered".
///
/// Pure introspection: every export index is resolved on the component here,
/// once, and each call then instantiates the guest fresh on a store from
/// `factory`. Every linked export an importer `wired` is checked against that
/// importer's signature, so a WIT skew between the two fails here rather than
/// mid-call. The registry's transactional publish moves the pending route live
/// together with the registry entry.
///
/// # Errors
///
/// Returns an error if a linked export cannot be resolved on the component,
/// its signature differs from what an importer wired, or the guest already
/// has a pending route.
pub fn serve_guest<T: Send + 'static>(
    routes: &Routes, interfaces: &BTreeSet<Box<str>>, wired: &WiredLinks, factory: StoreFactory<T>,
    id: &GuestId, instance_pre: InstancePre<T>,
) -> Result<()> {
    let engine = instance_pre.engine();
    let component = instance_pre.component();
    let component_ty = component.component_type();
    let mut funcs: HashMap<Box<str>, HashMap<Box<str>, Linked>> = HashMap::new();

    for (interface, types::ComponentExtern { ty, .. }) in component_ty.exports(engine) {
        if !interfaces.contains(interface) {
            continue;
        }
        let types::ComponentItem::ComponentInstance(instance_ty) = ty else {
            continue;
        };
        let iface_idx = component
            .get_export_index(None, interface)
            .with_context(|| format!("resolving `{interface}` on guest `{id}`"))?;
        let linked = funcs.entry(Box::from(interface)).or_default();
        for (func, types::ComponentExtern { ty, .. }) in instance_ty.exports(engine) {
            let types::ComponentItem::ComponentFunc(func_ty) = ty else {
                continue;
            };
            // Only the bootstrap importers are in the snapshot; a skew a late
            // importer introduces is still refused at lower time by
            // wasmtime's name-checked `Val` typing.
            if let Some(import) = wired.get(interface).and_then(|funcs| funcs.get(func)) {
                check_signature(id, interface, func, &func_ty, import)?;
            }
            let (_, export) = component
                .get_export(Some(&iface_idx), func)
                .with_context(|| format!("resolving `{interface}/{func}` on guest `{id}`"))?;
            linked.insert(
                Box::from(func),
                Linked {
                    export,
                    results: func_ty.results().len(),
                },
            );
        }
    }

    let route = (!funcs.is_empty()).then(|| {
        Arc::new(Route {
            factory,
            instance_pre,
            funcs,
        }) as Arc<dyn RouteInvoke>
    });
    routes.park(id, route)
}

// `types::Type` compares structurally across components; `ComponentFunc` does
// not, hence element-wise.
fn check_signature(
    exporter: &GuestId, interface: &str, func: &str, export: &types::ComponentFunc, import: &Wired,
) -> Result<()> {
    let same = export.async_() == import.ty.async_()
        && export.params().eq(import.ty.params())
        && export.results().eq(import.ty.results());
    ensure!(
        same,
        "guest `{exporter}` exports `{interface}/{func}` with a signature that differs from what \
         guest `{}` imports: exported `{}`, imported `{}`",
        import.importer,
        render(export),
        render(&import.ty),
    );
    Ok(())
}

fn render(ty: &types::ComponentFunc) -> String {
    let params: Vec<String> = ty.params().map(|(name, ty)| format!("{name}: {ty:?}")).collect();
    let results: Vec<types::Type> = ty.results().collect();
    let prefix = if ty.async_() { "async " } else { "" };
    format!("{prefix}func({}) -> {results:?}", params.join(", "))
}
