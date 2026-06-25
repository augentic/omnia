//! # Guest registry
//!
//! One [`Engine`] and one `Linker` hold many pre-instantiated guests at once,
//! each selectable by an opaque [`GuestId`]. A registry entry is instantiated
//! fresh per call and discarded (instance-per-call). This is pure wasmtime
//! infrastructure: it is what lets one process route an HTTP request, a CLI
//! command, and a topic message to *different* guests.
//!
//! The floor treats identities as opaque keys; consumers project their own
//! scheme onto them. Omnia never parses a [`GuestId`].

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use anyhow::{Result, bail};
use wasmtime::Engine;
use wasmtime::component::InstancePre;

use crate::RuntimeOptions;

/// Opaque guest identity.
///
/// The floor treats it as an ordered string key; consumers (e.g. Specify)
/// project their own scheme onto it (`source:typescript`, ...). Omnia never
/// parses it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GuestId(pub Arc<str>);

impl GuestId {
    /// Returns the identity as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for GuestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for GuestId {
    fn from(value: &str) -> Self {
        Self(Arc::from(value))
    }
}

impl From<String> for GuestId {
    fn from(value: String) -> Self {
        Self(Arc::from(value))
    }
}

/// A registry entry's resolution target.
///
/// Phase 1 only populates [`Target::Local`]. `Remote` (a bound wRPC endpoint)
/// arrives with the cluster transports and is what makes the desktop->cloud
/// swap a config change rather than a code change.
enum Target<T: 'static> {
    /// A locally pre-instantiated component.
    Local(InstancePre<T>),
}

/// A registered guest: an opaque identity bound to a resolution target.
pub struct Guest<T: 'static> {
    id: GuestId,
    target: Target<T>,
}

impl<T: 'static> Guest<T> {
    /// Create a guest backed by a local pre-instantiated component.
    #[must_use]
    pub const fn local(id: GuestId, instance_pre: InstancePre<T>) -> Self {
        Self {
            id,
            target: Target::Local(instance_pre),
        }
    }

    /// Returns the guest's identity.
    #[must_use]
    pub const fn id(&self) -> &GuestId {
        &self.id
    }

    /// Returns the guest's pre-instantiated component, ready to instantiate
    /// fresh on a new [`wasmtime::Store`] per call.
    #[must_use]
    pub const fn instance_pre(&self) -> &InstancePre<T> {
        match &self.target {
            Target::Local(pre) => pre,
        }
    }
}

/// One [`Engine`] + one `Linker`; many pre-instantiated guests keyed by
/// identity.
///
/// Every guest is pre-instantiated against the *same* linker, so they share one
/// set of host interfaces and one pooling pool — load-bearing for the
/// instance-per-call cost story. Pre-instantiation happens once, at
/// registration; per call only a fresh instantiate on a new store remains.
///
/// The registry is cheap to share behind an `Arc`, matching how the runtime
/// context is cloned into each connection handler.
pub struct Registry<T: 'static> {
    engine: Engine,
    options: RuntimeOptions,
    guests: HashMap<GuestId, Guest<T>>,
    default: GuestId,
}

impl<T: 'static> Registry<T> {
    /// Assemble a registry from pre-instantiated guests, with `default` naming
    /// the entry selected when a trigger carries no identity (the CLI / single
    /// `omnia run <wasm>` entry).
    ///
    /// # Errors
    ///
    /// Returns an error if `guests` is empty, or if `default` is not among the
    /// registered guests.
    pub fn new(
        engine: Engine, options: RuntimeOptions, guests: HashMap<GuestId, Guest<T>>,
        default: GuestId,
    ) -> Result<Self> {
        if guests.is_empty() {
            bail!("cannot build a guest registry with no guests");
        }
        if !guests.contains_key(&default) {
            bail!("default guest `{default}` is not registered");
        }
        Ok(Self {
            engine,
            options,
            guests,
            default,
        })
    }

    /// Returns the shared engine every guest is instantiated against.
    #[must_use]
    pub const fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Returns the runtime options.
    #[must_use]
    pub const fn options(&self) -> &RuntimeOptions {
        &self.options
    }

    /// Look up a guest by identity.
    #[must_use]
    pub fn get(&self, id: &GuestId) -> Option<&Guest<T>> {
        self.guests.get(id)
    }

    /// Returns the default guest — the entry a trigger resolves to when it
    /// carries no identity of its own (e.g. the CLI / single-file shorthand).
    ///
    /// # Panics
    ///
    /// Never in practice: [`Registry::new`] validates the default key is
    /// present and the map is not mutated after construction.
    #[must_use]
    pub fn default_guest(&self) -> &Guest<T> {
        self.guests.get(&self.default).expect("default guest is validated to exist at construction")
    }

    /// Returns the number of registered guests.
    #[must_use]
    pub fn len(&self) -> usize {
        self.guests.len()
    }

    /// Returns `true` if the registry has no guests (never, post-construction).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.guests.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use wasmtime::{Config, Engine};

    use super::*;

    #[test]
    fn guest_id() {
        let id = GuestId::from("source:typescript");
        assert_eq!(id.as_str(), "source:typescript");
        assert_eq!(id.to_string(), "source:typescript");
        assert_eq!(GuestId::from(String::from("workflow")), GuestId::from("workflow"));
        assert!(GuestId::from("a") < GuestId::from("b"));
    }

    #[test]
    fn no_guests() {
        let options = RuntimeOptions::load().expect("options should load");
        let engine = Engine::new(&Config::from(&options)).expect("engine should build");
        // An empty map never constructs a `Guest`, so `T` is unconstrained here.
        let guests: HashMap<GuestId, Guest<()>> = HashMap::new();

        let result = Registry::new(engine, options, guests, GuestId::from("default"));
        assert!(result.is_err(), "an empty registry must be rejected");
    }
}
