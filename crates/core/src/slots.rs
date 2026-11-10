//! The backend bundle keyed by capability.
//!
//! [`Slots`] holds one slot per capability a deployment's backends serve.
//! [`Fill`] and [`Link`] are what a tuple of hosts does with it.

use std::any::{Any, TypeId};
use std::fmt;

use anyhow::{Result, bail};
use futures::future::BoxFuture;
use wasmtime::component::Linker;

use crate::host::{Backends, Host, Provides, SlotHost};
use crate::runtime::Runtime;
use crate::store::StoreCtx;
use crate::{Backend, Capability, Serve};

/// A backend bundle holding one slot per capability, keyed by host type.
///
/// A backend fills the slot of every host it serves, and the bundle
/// [`Provides`] each [`SlotHost`] from its slot. Every store gets its own
/// clone.
///
/// # Panics
///
/// A host call panics when its host is linked but no backend fills its slot.
///
/// # Examples
///
/// ```
/// use std::fmt::Debug;
/// use std::sync::Arc;
///
/// use omnia_core::{Backend, Connect, HostCtx, NoOptions, Slots};
///
/// pub trait FooClient: Debug + Send + Sync + 'static {}
///
/// /// Host-side service for `omnia:foo`.
/// #[derive(Debug, HostCtx)]
/// #[host_ctx(client = FooClient)]
/// pub struct WasiFoo;
/// omnia_core::capability!(WasiFoo, "omnia:foo", FooClient);
///
/// #[derive(Clone, Debug)]
/// struct Memory;
///
/// impl FooClient for Memory {}
///
/// impl Connect for Memory {
///     type ConnectOptions = NoOptions;
///
///     async fn connect_with(_: NoOptions) -> anyhow::Result<Self> {
///         Ok(Self)
///     }
/// }
///
/// impl Backend for Memory {
///     type Hosts = (WasiFoo,);
/// }
///
/// let mut slots = Slots::new();
/// slots.add(&Memory)?;
/// let client: &Arc<dyn FooClient> = slots.get::<WasiFoo>().expect("filled");
/// # anyhow::Ok(())
/// ```
#[derive(Default)]
pub struct Slots {
    entries: Vec<Entry>,
}

impl Slots {
    /// Creates an empty bundle.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fills the slot of every host `backend` names in [`Backend::Hosts`].
    ///
    /// # Errors
    ///
    /// Returns an error naming the capability when another backend already
    /// serves one of the hosts.
    pub fn add<T: Backend>(&mut self, backend: &T) -> Result<&mut Self>
    where
        T::Hosts: Fill<T>,
    {
        self.add_as::<T, T::Hosts>(backend)
    }

    /// Fills only the slots of the host tuple `Hs` from `backend`.
    ///
    /// # Errors
    ///
    /// Returns an error naming the capability when another backend already
    /// serves one of the hosts.
    pub fn add_as<T, Hs: Fill<T>>(&mut self, backend: &T) -> Result<&mut Self> {
        Hs::fill(backend, self)?;
        Ok(self)
    }

    /// Returns the slot a backend filled for host `H`, if any.
    #[must_use]
    pub fn get<H: Capability>(&self) -> Option<&H::Slot> {
        let host = TypeId::of::<H>();
        let entry = self.entries.iter().find(|entry| entry.host == host)?;
        (*entry.slot).as_any().downcast_ref()
    }

    fn insert<H: Capability>(&mut self, slot: H::Slot) -> Result<()> {
        let host = TypeId::of::<H>();
        if self.entries.iter().any(|entry| entry.host == host) {
            bail!("two backends serve `{}`", H::NAME);
        }
        self.entries.push(Entry {
            host,
            name: H::NAME,
            slot: Box::new(slot),
        });
        Ok(())
    }
}

impl Clone for Slots {
    fn clone(&self) -> Self {
        let entries = self.entries.iter().map(|entry| Entry {
            host: entry.host,
            name: entry.name,
            // through the deref, or the blanket impl would clone the `Box`
            slot: (*entry.slot).boxed(),
        });
        Self {
            entries: entries.collect(),
        }
    }
}

impl fmt::Debug for Slots {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.entries.iter().map(|entry| entry.name)).finish()
    }
}

impl<H: SlotHost> Provides<H> for Slots {
    fn borrow(&mut self) -> H::Borrow<'_> {
        let host = TypeId::of::<H>();
        let slot = self
            .entries
            .iter_mut()
            .find(|entry| entry.host == host)
            .and_then(|entry| (*entry.slot).as_any_mut().downcast_mut::<H::Slot>())
            .unwrap_or_else(|| panic!("`{}` is linked but no backend serves it", H::NAME));
        H::lend(slot)
    }
}

struct Entry {
    host: TypeId,
    name: &'static str,
    slot: Box<dyn AnySlot>,
}

// A slot with its type erased: cloned for every store, downcast for its host.
trait AnySlot: Any + Send + Sync {
    fn boxed(&self) -> Box<dyn AnySlot>;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<S: Clone + Send + Sync + 'static> AnySlot for S {
    fn boxed(&self) -> Box<dyn AnySlot> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// A tuple of hosts whose slots a backend of type `T` fills.
///
/// Implemented for tuples of one to four hosts that [`Serve<T>`].
pub trait Fill<T> {
    /// Fills the slot of every host in the tuple from `backend`.
    ///
    /// # Errors
    ///
    /// Returns an error naming the capability when another backend already
    /// serves one of the hosts.
    fn fill(backend: &T, slots: &mut Slots) -> Result<()>;
}

/// A tuple of hosts linked and run over the backend bundle `B`.
///
/// Implemented for tuples of one to four [`Host<B>`] capabilities.
pub trait Link<B: Backends> {
    /// The [`Capability::NAME`] of every host in the tuple.
    const NAMES: &'static [&'static str];

    /// Links every host in the tuple into the deployment linker.
    ///
    /// # Errors
    ///
    /// Returns the linking error from a host's generated bindings.
    fn link(linker: &mut Linker<StoreCtx<B>>) -> Result<()>;

    /// Pushes every host's [`Host::run`] future onto `servers`.
    fn run<'a>(runtime: &'a Runtime<B>, servers: &mut Vec<BoxFuture<'a, Result<()>>>);
}

macro_rules! host_tuple {
    ($($host:ident),+) => {
        impl<T, $($host: Serve<T>),+> Fill<T> for ($($host,)+) {
            fn fill(backend: &T, slots: &mut Slots) -> Result<()> {
                $(slots.insert::<$host>($host::slot(backend))?;)+
                Ok(())
            }
        }

        impl<B: Backends, $($host: Host<B> + Capability),+> Link<B> for ($($host,)+) {
            const NAMES: &'static [&'static str] = &[$($host::NAME),+];

            fn link(linker: &mut Linker<StoreCtx<B>>) -> Result<()> {
                $($host::add_to_linker(linker)?;)+
                Ok(())
            }

            fn run<'a>(runtime: &'a Runtime<B>, servers: &mut Vec<BoxFuture<'a, Result<()>>>) {
                $(servers.push(Box::pin($host::run(runtime)));)+
            }
        }
    };
}

host_tuple!(H1);
host_tuple!(H1, H2);
host_tuple!(H1, H2, H3);
host_tuple!(H1, H2, H3, H4);

// The guest-driven suites cover a filled slot serving its host. These cover
// the two refusals no guest run reaches.
#[cfg(test)]
mod tests {
    use std::fmt::Debug;
    use std::sync::Arc;

    use wasmtime::component::{HasData, ResourceTable};

    use super::*;
    use crate::host::{HostCtx, HostView};

    trait FooClient: Debug + Send + Sync + 'static {}

    #[derive(Debug)]
    struct WasiFoo;

    crate::capability!(WasiFoo, "omnia:foo", FooClient);

    impl HasData for WasiFoo {
        type Data<'a> = HostView<'a, dyn FooClient>;
    }

    impl HostCtx for WasiFoo {
        type Borrow<'a> = &'a dyn FooClient;

        fn view<'a>(client: Self::Borrow<'a>, table: &'a mut ResourceTable) -> Self::Data<'a> {
            HostView { client, table }
        }
    }

    impl SlotHost for WasiFoo {
        fn lend(slot: &mut Arc<dyn FooClient>) -> &dyn FooClient {
            &**slot
        }
    }

    #[derive(Clone, Debug)]
    struct Memory;

    impl FooClient for Memory {}

    #[derive(Clone, Debug)]
    struct Remote;

    impl FooClient for Remote {}

    #[test]
    fn two_backends_one_host() {
        let mut slots = Slots::new();
        slots.add_as::<_, (WasiFoo,)>(&Memory).expect("should fill an empty slot");

        let error = slots.add_as::<_, (WasiFoo,)>(&Remote).expect_err("should refuse");

        assert_eq!(error.to_string(), "two backends serve `omnia:foo`");
    }

    #[test]
    #[should_panic(expected = "`omnia:foo` is linked but no backend serves it")]
    fn linked_without_slot() {
        let mut slots = Slots::new();

        let _ = <Slots as Provides<WasiFoo>>::borrow(&mut slots);
    }
}
