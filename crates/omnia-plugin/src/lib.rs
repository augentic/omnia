//! # The guest loader
//!
//! The `omnia:plugins/loader` capability crate. A caller names where a
//! guest's bytes come from — a [`Location`]: a guest the deployment declares,
//! a component beneath one of its read-only mounts, or a package one of its
//! registries serves — and the host acquires the bytes inside the
//! deployment's grant, verifies them against what that grant declares (the
//! entry's digest pin or the call's, raw wasm alone for anything the caller
//! named), and admits them through the runtime's admission seam, handing back
//! a typed [`Plugin`] handle. Component bytes never cross the interface, and
//! nothing a caller passes widens the grant: a declared name must be in the
//! guest list, a path must lie beneath a read-only mount, and a package is
//! fetched from the registry the deployment's `registries` routes it to —
//! the load may name a registry only for a namespace that routing leaves
//! open. The requester receives no lifecycle authority: validation,
//! compilation, and publication stay host-side.
//!
//! Everything loader lives here: the [`WasiPlugins`] host binding, the
//! [`Plugins`] load path over the deployment's on-demand [`Source`] table and
//! its mounts, and the registry seam. A [`SourceSpec::Package`] source is
//! fetched by the [`RegistrySource`] the deployment installs — by default a
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
pub use self::source::{Location, RegistrySource};
pub use self::store::{ContentStore, NoStore, ReleaseStore};
