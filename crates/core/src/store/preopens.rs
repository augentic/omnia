//! `wasi:filesystem/preopens`, served from each store's mount registry.

use anyhow::Result;
use wasmtime::component::{HasData, Linker, Resource, ResourceTable};
use wasmtime_wasi::filesystem::Descriptor;
use wasmtime_wasi::p3::bindings::filesystem::preopens;

use crate::{MountRegistry, StoreCtx};

// Replaces the `get-directories` wasmtime-wasi linked, which reads the
// `WasiCtx` preopens a store would otherwise open by path.
pub fn add_to_linker<B: Send + 'static>(linker: &mut Linker<StoreCtx<B>>) -> Result<()> {
    linker.allow_shadowing(true);
    let shadowed = preopens::add_to_linker::<_, Preopens>(linker, |ctx| PreopensView {
        mounts: &ctx.base.mounts,
        table: &mut ctx.base.table,
    });
    linker.allow_shadowing(false);
    Ok(shadowed?)
}

struct Preopens;

impl HasData for Preopens {
    type Data<'a> = PreopensView<'a>;
}

struct PreopensView<'a> {
    mounts: &'a MountRegistry,
    table: &'a mut ResourceTable,
}

impl preopens::Host for PreopensView<'_> {
    fn get_directories(&mut self) -> wasmtime::Result<Vec<(Resource<Descriptor>, String)>> {
        self.mounts
            .entries()
            .iter()
            .map(|mount| {
                let descriptor = self.table.push(Descriptor::Dir(mount.preopen.clone()))?;
                Ok((descriptor, mount.name.clone()))
            })
            .collect()
    }
}
