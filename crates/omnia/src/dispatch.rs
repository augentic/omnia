//! # Host-mediated dynamic linking
//!
//! A caller guest imports an interface (say `omnia:link/echo`) whose
//! implementation the host satisfies at runtime. The host polyfills that import
//! on the shared `Linker` so invoking it:
//!
//! 1. extracts a target identity from the call via a [`GuestSelector`],
//! 2. rejects any resource handle attempting to cross the seam,
//! 3. enforces a dispatch-depth bound,
//! 4. instantiates the target *fresh* on a new store and invokes the matching
//!    export over the bound wRPC transport, and
//! 5. returns the typed result, discarding the callee instance.
//!
//! Because step 4 is always a fresh instance, a dispatched call cannot
//! recursively re-enter its caller. The runtime core stays generic: it links whatever
//! interfaces the manifest names, by opaque string, and resolves opaque
//! [`GuestId`]s — it never parses a consumer scheme. See
//! `rfcs/guest-registry.md` for the full design.
//!
//! ## Where the selector runs
//!
//! The selector must see the *typed* parameters, so the polyfill is a
//! `func_new_async` closure that runs the selector *before* encoding the call
//! onto wRPC — then reuses wRPC's own value codec ([`ValEncoder`]/[`read_value`])
//! and instance-per-call serve integration ([`ServeExt::serve_function`]) for
//! the carrier round-trip.

mod handle;
mod host;
mod link;
mod selector;
mod serve;
mod transport;

pub use handle::DispatchHandle;
pub use host::HostDispatch;
pub use link::link;
pub use selector::{FirstArgSelector, GuestSelector};
pub use serve::serve_links;
pub use transport::{LinkClient, WrpcState};
