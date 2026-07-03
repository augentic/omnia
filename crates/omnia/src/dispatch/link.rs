//! Linker polyfill for host-mediated imports.

use std::collections::BTreeSet;
use std::iter::zip;
use std::pin::pin;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail, ensure};
use bytes::BytesMut;
use tokio_util::codec::Encoder as _;
use wasmtime::component::{Linker, Type, Val, types};
use wasmtime::{AsContextMut as _, Engine, StoreContextMut};
use wasmtime_wasi::WasiView;
use wrpc_transport::Invoke;
use wrpc_wasmtime::{ValEncoder, WrpcView, read_value};

use super::handle::DispatchHandle;
use super::transport::LinkTransport as _;
use crate::deployment::LoadedGuest;

/// Polyfill every host-mediated import named in the `link` allow-list union onto
/// the shared linker, bound to the dispatch handle.
///
/// Each interface is linked exactly once (the linker is shared, so the per-guest
/// allow-lists are unioned). `wasi:*` imports are never touched here — they are
/// host-satisfied — so only the manifest-declared interfaces are dispatched.
///
/// Runs *before* pre-instantiation, so an import that is neither host-satisfied
/// nor allow-listed remains unresolved and fails fast at `instantiate_pre`.
///
/// # Errors
///
/// Returns an error if a named link target is not an interface import, or if a
/// function cannot be defined on the linker.
pub fn link<T>(
    engine: &Engine, linker: &mut Linker<T>, guests: &[LoadedGuest], handle: &Arc<DispatchHandle>,
) -> Result<()>
where
    T: WasiView + WrpcView + 'static,
{
    if handle.links().is_empty() {
        return Ok(());
    }

    let mut wired: BTreeSet<Box<str>> = BTreeSet::new();
    for LoadedGuest { id, component } in guests {
        let component_ty = component.component_type();
        for (name, types::ComponentExtern { ty, .. }) in component_ty.imports(engine) {
            if !handle.links().contains(name) || wired.contains(name) {
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
                            send(store, &handle, &iface_name, &func_name, &ty, params, results)
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
async fn send<T>(
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

    // Plain records cross by value; a live resource handle never crosses.
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
        let mut encoder = ValEncoder::new(store.as_context_mut(), ty, &[], &[]);
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

/// Recursively reports whether a value carries a live resource handle.
pub(super) fn contains_resource(value: &Val) -> bool {
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

#[cfg(test)]
mod tests {
    use wasmtime::component::Val;

    use super::contains_resource;

    #[test]
    fn detect_nested() {
        // Plain values never count as resources.
        assert!(!contains_resource(&Val::String("x".to_owned())));
        assert!(!contains_resource(&Val::Record(vec![("f".to_owned(), Val::U32(1),)])));
        assert!(!contains_resource(&Val::Option(None)));
        // A nested option/list carrying plain values stays clean.
        assert!(!contains_resource(&Val::List(vec![Val::Bool(true), Val::Bool(false)])));
    }
}
