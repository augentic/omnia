//! # The guest loader
//!
//! The `omnia:plugins/loader` capability crate: a guest names code (a
//! location — a registry package, a mount-relative path, or a name the
//! deployment declares — and an optional sha256 pin) and the host acquires,
//! verifies, and admits it through the runtime's admission seam, handing
//! back a typed [`Plugin`] handle. Component bytes never cross the interface
//! in either direction, and the requester receives no lifecycle authority —
//! validation, compilation, and publication stay host-side.
//!
//! Everything loader lives here: the [`WasiPlugins`] host binding, the
//! [`Plugins`] load path, and the acquisition seam. Acquisition policy
//! (registries, cache, path reads) is the two slots [`Plugins::install`]
//! takes — one per acquiring [`Origin`] kind. Assembly installs the declared
//! policy through [`Plugins::install_declared`]: the deployment's mounts
//! become the roots path loads resolve against, and its `registries`
//! configuration (the `runtime!` macro's `registries:`, a manifest's
//! `[registries]`) is the default routing of a package load that names no
//! registry of its own. The built-in acquirers are [`PathMounts`] and
//! [`RegistryClient`]; a store behind `RegistryClient` implements
//! [`ContentStore`] and [`ReleaseStore`]. The runtime core keeps zero storage
//! and network dependencies.
//!
//! Embedders — deployments and store implementors alike — reach all of this
//! through the `omnia` facade's re-exports, never by depending on this crate
//! or on `omnia-core` directly; those are dependencies for building another
//! capability crate.

#![cfg(not(target_arch = "wasm32"))]

mod admission;
mod declared;
mod error;
mod host;
mod loader;
mod path;
mod registry;
mod source;
mod store;

pub use self::error::LoadError;
pub use self::host::{WasiPlugins, WasiPluginsCtxView};
pub use self::loader::{Plugin, PluginLoader, Plugins};
pub use self::path::PathMounts;
pub use self::registry::RegistryClient;
pub use self::source::{Origin, PathSource, RegistrySource};
pub use self::store::{ContentStore, NoStore, ReleaseStore};
