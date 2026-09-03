//! Compile-only proof that the `omnia` facade is a deployment's whole
//! dependency surface.
//!
//! The manifest declares `omnia` and one host crate — nothing else — and this
//! file exercises every `runtime!` shape that reaches past the `hosts:` block:
//! a `plugins:` block with both location kinds and a `cache:` store, whose
//! type implements the store traits and `Backend` by hand. Every path the
//! macro emits and every trait a store must name has to resolve through
//! `omnia::…`, or this crate fails to build. Nothing here runs; the macro's
//! snapshot suite pins the expansion's shape, this crate pins its
//! resolvability from an embedder's position.

#![cfg(not(target_arch = "wasm32"))]

use std::future::Future;

use omnia::anyhow::Result;
use omnia::futures::future::BoxFuture;
use omnia::{Backend, ContentStore, NoOptions, NoStore, ReleaseStore};
use omnia_wasi_otel::{OtelDefault, WasiOtel};

// Clone without Copy: the generated hook clones the store out of the backend
// bundle, and a Copy type would trip `clippy::clone_on_copy` there.
#[derive(Clone)]
struct Cache;

impl Backend for Cache {
    type ConnectOptions = NoOptions;

    fn connect_with(_options: NoOptions) -> impl Future<Output = Result<Self>> {
        std::future::ready(Ok(Self))
    }
}

impl ContentStore for Cache {
    fn content<'a>(&'a self, digest: &'a str) -> BoxFuture<'a, Result<Option<Vec<u8>>>> {
        ContentStore::content(&NoStore, digest)
    }

    fn put_content<'a>(&'a self, digest: &'a str, bytes: &'a [u8]) -> BoxFuture<'a, Result<()>> {
        ContentStore::put_content(&NoStore, digest, bytes)
    }
}

impl ReleaseStore for Cache {
    fn release<'a>(
        &'a self, registry: &'a str, package: &'a str, version: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>>> {
        ReleaseStore::release(&NoStore, registry, package, version)
    }

    fn put_release<'a>(
        &'a self, registry: &'a str, package: &'a str, version: &'a str, digest: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        ReleaseStore::put_release(&NoStore, registry, package, version, digest)
    }
}

omnia::runtime!({
    plugins: {
        interfaces: ["omnia:link/echo"],
        locations: [
            { name: "adapters", path: "adapters" },
            { registry: "ghcr.io" },
        ],
        cache: Cache,
    },
    guests: [
        { id: "engine", source: "engine.wasm" },
    ],
    hosts: {
        WasiOtel: OtelDefault,
    },
});
