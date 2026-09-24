//! # The guest loader
//!
//! The `omnia:plugins/loader` capability crate. The deployment's guest list
//! is the allow-list: every component that may ever run is a `[[guest]]`
//! entry, and an entry marked `on_demand` is admitted not at boot but when a
//! caller first names it through `load(name)`. The host acquires the bytes
//! from the entry's declared [`Source`], verifies them against what the
//! source declares — its digest pin, whether it admits raw wasm alone — and
//! admits them through the runtime's admission seam, handing back a typed
//! [`Plugin`] handle. Nothing a caller passes chooses code — no bytes, no
//! paths, no registry endpoints — and the requester receives no lifecycle
//! authority: validation, compilation, and publication stay host-side.
//!
//! Everything loader lives here: the [`WasiPlugins`] host binding, the
//! [`Plugins`] load path over the deployment's on-demand [`Source`] table,
//! and the registry seam. A [`SourceSpec::Package`] source is fetched by the
//! [`RegistrySource`] the deployment installs — by default a
//! [`RegistryClient`] routed by the deployment's `registries` configuration
//! (the `runtime!` macro's `registries:`, a manifest's `[registries]`); a
//! store behind it implements [`ContentStore`] and [`ReleaseStore`]. The
//! runtime core keeps zero storage and network dependencies.
//!
//! [`Source`]: omnia_core::Source
//! [`SourceSpec::Package`]: omnia_core::SourceSpec::Package
//!
//! Embedders — deployments and store implementors alike — reach all of this
//! through the `omnia` facade's re-exports, never by depending on this crate
//! or on `omnia-core` directly; those are dependencies for building another
//! capability crate.

#![cfg(not(target_arch = "wasm32"))]

mod admission;
mod error;
mod host;
mod loader;
mod registry;
mod source;
mod store;

pub use self::error::LoadError;
pub use self::host::{WasiPlugins, WasiPluginsCtxView};
pub use self::loader::{Plugin, PluginLoader, Plugins};
pub use self::registry::RegistryClient;
pub use self::source::RegistrySource;
pub use self::store::{ContentStore, NoStore, ReleaseStore};
