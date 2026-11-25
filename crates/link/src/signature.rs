//! The check that guests sharing a linked function agree on its signature.

use anyhow::{Result, ensure};
use omnia_core::Guest;
use wasmtime::Engine;
use wasmtime::component::types::{self, ComponentExtern, ComponentItem};

use super::is_host;

pub fn check<B: 'static>(guest: &Guest<B>, standing: &[&Guest<B>]) -> Result<()> {
    let engine = guest.component().engine();
    let component = guest.component().component_type();
    let mut funcs = declared(engine, Side::Import, component.imports(engine));
    funcs.extend(declared(engine, Side::Export, component.exports(engine)));
    if funcs.is_empty() {
        return Ok(());
    }

    for other in standing {
        let theirs = other.component().component_type();
        for Declared {
            side,
            interface,
            func,
            ty,
        } in &funcs
        {
            for &meets in side.meets() {
                let Some(their_ty) = meets.find(engine, &theirs, interface, func) else {
                    continue;
                };
                ensure!(
                    same(ty, &their_ty),
                    "guest `{}` {} `{interface}/{func}` as `{}`, but guest `{}` {} it as `{}`",
                    guest.id(),
                    side.verb(),
                    render(ty),
                    other.id(),
                    meets.verb(),
                    render(&their_ty),
                );
            }
        }
    }
    Ok(())
}

struct Declared<'a> {
    side: Side,
    interface: &'a str,
    func: String,
    ty: types::ComponentFunc,
}

// every function of each instance outside the runtime's own namespaces
fn declared<'a>(
    engine: &Engine, side: Side, items: impl Iterator<Item = (&'a str, ComponentExtern<'a>)>,
) -> Vec<Declared<'a>> {
    let mut funcs = Vec::new();
    for (interface, ComponentExtern { ty, .. }) in items {
        if is_host(interface) {
            continue;
        }
        let ComponentItem::ComponentInstance(instance) = ty else {
            continue;
        };
        for (func, ComponentExtern { ty, .. }) in instance.exports(engine) {
            if let ComponentItem::ComponentFunc(ty) = ty {
                funcs.push(Declared {
                    side,
                    interface,
                    func: func.to_owned(),
                    ty,
                });
            }
        }
    }
    funcs
}

#[derive(Clone, Copy)]
enum Side {
    Import,
    Export,
}

impl Side {
    const fn meets(self) -> &'static [Self] {
        match self {
            Self::Import => &[Self::Export, Self::Import],
            Self::Export => &[Self::Import],
        }
    }

    const fn verb(self) -> &'static str {
        match self {
            Self::Import => "imports",
            Self::Export => "exports",
        }
    }

    // `interface/func` as `component` declares it on this side
    fn find(
        self, engine: &Engine, component: &types::Component, interface: &str, func: &str,
    ) -> Option<types::ComponentFunc> {
        let item = match self {
            Self::Import => component.get_import(engine, interface),
            Self::Export => component.get_export(engine, interface),
        }?;
        let ComponentItem::ComponentInstance(instance) = item.ty else {
            return None;
        };
        let ComponentItem::ComponentFunc(ty) = instance.get_export(engine, func)?.ty else {
            return None;
        };
        Some(ty)
    }
}

// element-wise, since `ComponentFunc` does not compare structurally across components
fn same(a: &types::ComponentFunc, b: &types::ComponentFunc) -> bool {
    a.async_() == b.async_() && a.params().eq(b.params()) && a.results().eq(b.results())
}

fn render(ty: &types::ComponentFunc) -> String {
    let params: Vec<String> = ty.params().map(|(name, ty)| format!("{name}: {ty:?}")).collect();
    let results: Vec<types::Type> = ty.results().collect();
    let prefix = if ty.async_() { "async " } else { "" };
    format!("{prefix}func({}) -> {results:?}", params.join(", "))
}
