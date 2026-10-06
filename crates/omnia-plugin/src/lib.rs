//! # The guest loader
//!
//! The `omnia:plugins/loader` capability crate. A caller names where a
//! guest's bytes come from — a [`Location`]: a guest the deployment declares,
//! a component beneath one of its read-only mounts, or a package its store
//! holds or one of its registries serves — and the host acquires the bytes inside the
//! deployment's grant and admits them through the runtime's one admission
//! body, which verifies them against what the grant declares (the entry's
//! digest pin or the call's, raw wasm alone for anything the caller named,
//! never under a name the deployment declares), handing back a typed
//! [`Plugin`] handle. Component bytes never cross the interface, and
//! nothing a caller passes widens the grant: a declared name must be in the
//! guest list, a path must lie beneath a read-only mount, and a package is
//! served from the deployment's store or fetched from the registry its
//! `registries` routes it to — the load may name a registry only for a
//! namespace that routing leaves open. The requester receives no lifecycle authority: validation,
//! compilation, and publication stay host-side.
//!
//! Everything loader lives here: the [`WasiPlugins`] host binding, the
//! [`Plugins`] load path over the runtime's first-use seam (a declared name),
//! the deployment's mounts (a path), and the runtime's acquisition (a
//! package), the registry client, and the package store. A
//! [`SourceSpec::Package`] source — the deployment's, at first use, or one a
//! load names — is acquired by the [`RegistrySource`] the deployment installs
//! — by default a [`RegistryClient`] over the deployment's [`PackageStore`]:
//! the store first (an [`FsStore`] over the directory the `runtime!` macro's
//! `plugins: { store: .. }` or a manifest's `[plugins] store` names, where a
//! release is filed by its [`Reference`]), then the registry its
//! `plugins.registries` configuration routes the package to, what it fetches
//! written to the store once. The runtime core keeps zero storage and network
//! dependencies.
//!
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
pub use self::source::{Location, RegistrySource};
pub use self::store::{FsStore, NoStore, PackageStore, Reference};
