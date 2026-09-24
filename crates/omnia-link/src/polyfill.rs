//! Linker polyfill for host-mediated imports.

use std::collections::BTreeMap;
use std::iter::zip;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use omnia_core::{ChainCtx, ChainPolicy, GuestId, HasChain, InvokeError, handle_kind};
use wasmtime::Engine;
use wasmtime::component::{Linker, Type, Val, types};

use super::is_host;
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

/// Polyfill one component's imports outside the runtime's own namespaces
/// ([`is_host`]) not already in `wired`, bound to `caller`.
///
/// Each function is linked exactly once (the linker is shared, so the
/// per-guest imports are unioned function-by-function, reopening an
/// interface's [`LinkerInstance`](wasmtime::component::LinkerInstance) as
/// later guests add functions). `wasi:*` and `omnia:*` imports are never
/// touched here — they are host-satisfied — and a bare function or type
/// import is not a service, so it is skipped. Runs *before*
/// pre-instantiation, so a host import nothing links remains unresolved and
/// fails fast at `instantiate_pre`, and a relayed import that a host also
/// links collides with that host's definition rather than shadowing it.
///
/// Registration matches the import's type-level asyncness: a plain `func` is
/// polyfilled with `func_new_async`, an `async func` with
/// `func_new_concurrent` — the sync-typed registration would fail the
/// pre-instantiation asyncness typecheck. Both share one body ([`relay`]),
/// whose only read of the caller's store is a snapshot of its chain context,
/// taken before the dispatch. A function an earlier guest wired with the
/// *other* asyncness is a cross-guest interface disagreement, rejected here
/// with both views named. A signature carrying a store-bound handle
/// (resource, future, stream, error-context) is refused before it is wired:
/// only plain values cross the seam.
///
/// # Errors
///
/// Returns an error if a function's signature is not plain, two importers
/// disagree on a function's asyncness, or a function cannot be defined on
/// the linker.
pub fn polyfill_component<T: HasChain + 'static>(
    engine: &Engine, linker: &mut Linker<T>, id: &GuestId,
    component: &wasmtime::component::Component, caller: &Arc<Caller>, wired: &mut WiredLinks,
) -> Result<()> {
    let component_ty = component.component_type();
    for (name, types::ComponentExtern { ty, .. }) in component_ty.imports(engine) {
        if is_host(name) {
            continue;
        }
        // A bare function or type import is not a service; if nothing
        // satisfies it, `instantiate_pre` says so.
        let types::ComponentItem::ComponentInstance(instance_ty) = ty else {
            continue;
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
                Some(earlier) if earlier.ty.async_() == is_async => {}
                Some(earlier) => bail!(
                    "guest `{id}` imports `{name}/{func}` as {}, but an earlier guest wired it \
                     as {}; every importer of a host-mediated function must agree on asyncness",
                    describe(is_async),
                    describe(earlier.ty.async_()),
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

        // Opening the instance also (re)defines it on the linker, so a relayed
        // interface resolves even when every function is already wired (or it
        // has none).
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
            // The caller's chain context is read here, before the future is
            // built, so no store borrow crosses the dispatch.
            let registered = if ty.async_() {
                interface.func_new_concurrent(func, move |accessor, ty, params, results| {
                    let caller = Arc::clone(&caller);
                    let iface_name = Arc::clone(&iface_name);
                    let func_name = Arc::clone(&func_name);
                    let chain = accessor.with(|mut access| access.data_mut().chain());
                    Box::pin(async move {
                        relay(&caller, chain, &iface_name, &func_name, &ty, params, results)
                            .await
                            .map_err(wasmtime::Error::from_anyhow)
                    })
                })
            } else {
                interface.func_new_async(func, move |store, ty, params, results| {
                    let caller = Arc::clone(&caller);
                    let iface_name = Arc::clone(&iface_name);
                    let func_name = Arc::clone(&func_name);
                    let chain = store.data().chain();
                    Box::new(async move {
                        relay(&caller, chain, &iface_name, &func_name, &ty, params, results)
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
                ty,
                importer: id.clone(),
            };
            (Box::from(&*func), wired)
        }));
    }
    Ok(())
}

/// The per-call dispatch: select the target, reject crossing handles, take a
/// depth slot beneath the calling guest's `chain`, resolve the live route, and
/// move the lifted parameters to a fresh callee instance on its own task,
/// writing its results back.
async fn relay(
    caller: &Caller, chain: ChainCtx, interface: &str, func: &str, ty: &types::ComponentFunc,
    params: &[Val], results: &mut [Val],
) -> Result<()> {
    let start = Instant::now();

    let (target, forwarded) = caller
        .selector
        .select(interface, func, params)
        .with_context(|| format!("selecting target for `{interface}/{func}`"))?;

    // Plain records cross by value; a live handle never crosses.
    if let Some(kind) = forwarded.iter().find_map(handle_kind) {
        bail!(
            "a {kind} handle cannot cross the link seam (call to `{interface}/{func}`, \
             target `{target}`)"
        );
    }

    let ctx = caller.policy.enter(&chain, &target)?;

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
        elapsed_us,
        "dispatched host-mediated call",
    );
    Ok(())
}

/// Checks that every parameter and result type of `func` is a plain value,
/// naming the kind (`resource`, `future`, `stream`, or `error-context`) of the
/// first store-bound handle type the signature carries.
fn plain_signature(func: &types::ComponentFunc) -> Result<(), &'static str> {
    func.params().map(|(_, ty)| ty).chain(func.results()).try_for_each(|ty| plain_type(&ty))
}

fn plain_type(ty: &Type) -> Result<(), &'static str> {
    match ty {
        Type::Own(_) | Type::Borrow(_) => Err("resource"),
        Type::Future(_) => Err("future"),
        Type::Stream(_) => Err("stream"),
        Type::ErrorContext => Err("error-context"),
        Type::List(list) => plain_type(&list.ty()),
        Type::FixedLengthList(list) => plain_type(&list.ty()),
        Type::Map(map) => plain_type(&map.key()).and_then(|()| plain_type(&map.value())),
        Type::Record(record) => record.fields().try_for_each(|field| plain_type(&field.ty)),
        Type::Tuple(tuple) => tuple.types().try_for_each(|ty| plain_type(&ty)),
        Type::Variant(variant) => {
            variant.cases().try_for_each(|case| case.ty.as_ref().map_or(Ok(()), plain_type))
        }
        Type::Option(option) => plain_type(&option.ty()),
        Type::Result(result) => result
            .ok()
            .as_ref()
            .map_or(Ok(()), plain_type)
            .and_then(|()| result.err().as_ref().map_or(Ok(()), plain_type)),
        Type::Bool
        | Type::S8
        | Type::U8
        | Type::S16
        | Type::U16
        | Type::S32
        | Type::U32
        | Type::S64
        | Type::U64
        | Type::Float32
        | Type::Float64
        | Type::Char
        | Type::String
        | Type::Enum(_)
        | Type::Flags(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use wasmtime::component::types::ComponentItem;
    use wasmtime::component::{Component, types};
    use wasmtime::{Config, Engine};

    use super::plain_signature;

    fn signatures() -> (Engine, Component) {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.wasm_component_model_async(true);
        let engine = Engine::new(&config).expect("engine");
        let component = Component::new(
            &engine,
            r#"
            (component
              (import "sigs" (instance
                (export "r" (type $r (sub resource)))
                (type $rec-def (record (field "n" u32) (field "r" (own $r))))
                (export "rec" (type $rec (eq $rec-def)))
                (export "plain" (func (param "n" u32) (param "s" string) (result (list u8))))
                (export "own" (func (param "r" (own $r))))
                (export "borrow" (func (param "r" (borrow $r))))
                (export "future" (func (result (future u32))))
                (export "stream" (func (param "bytes" (stream u8))))
                (export "nested" (func (param "rec" (option $rec))))
              ))
            )
            "#,
        )
        .expect("type-only component");
        (engine, component)
    }

    fn signature(engine: &Engine, component: &Component, name: &str) -> types::ComponentFunc {
        let ComponentItem::ComponentInstance(sigs) =
            component.component_type().get_import(engine, "sigs").expect("sigs import").ty
        else {
            panic!("sigs import is not an instance");
        };
        let (_, types::ComponentExtern { ty, .. }) =
            sigs.exports(engine).find(|(func, _)| *func == name).expect(name);
        match ty {
            ComponentItem::ComponentFunc(func) => func,
            other => panic!("{name} export is {other:?}"),
        }
    }

    #[test]
    fn plain_params_and_results() {
        let (engine, component) = signatures();
        assert_eq!(plain_signature(&signature(&engine, &component, "plain")), Ok(()));
    }

    #[test]
    fn handle_kinds() {
        let (engine, component) = signatures();
        for (name, kind) in [
            ("own", "resource"),
            ("borrow", "resource"),
            ("future", "future"),
            ("stream", "stream"),
        ] {
            assert_eq!(plain_signature(&signature(&engine, &component, name)), Err(kind), "{name}");
        }
    }

    #[test]
    fn handle_nested_in_record() {
        let (engine, component) = signatures();
        assert_eq!(plain_signature(&signature(&engine, &component, "nested")), Err("resource"));
    }
}
