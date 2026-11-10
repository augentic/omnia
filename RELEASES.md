## 0.38.0

Unreleased

### Added

- `omnia_sdk::api::{Handlers, Router}`: a module registers its HTTP routes
  (`route`), messaging topics (`topic`, `topic_with`), and CLI subcommands
  (`command`, `command_with`, behind the `command` feature) on one
  `Handlers<P>`, `From<[..; N]>` (or `merge`) merges them, and
  `with_state(client)` closes the whole into the one `Router<P>` that
  serves requests, handles deliveries, and `run`s argv. `with_state` is
  the router's only constructor: `Router` registers nothing itself, and
  keeps `layer`, the program methods, `handle`, and `run`.
  `command::export!` binds a command router fn (with `WasiProvider` or a
  custom provider) alongside a zero-argument `async fn` entry, stamping
  the guest crate's `CARGO_PKG_VERSION` and `CARGO_PKG_DESCRIPTION` as
  the program's `--version` and `--help` defaults.
- `omnia_sdk::api::command::Render`: the text form of a handler output,
  which `Format::Text` encodes (`()` and `String` implement it); `Failure`
  renders its envelope through it.
- `omnia_sdk::Capabilities`: every capability trait at once, with the
  `Send + Sync + 'static` a provider needs, for a fn generic over the
  whole provider (`handlers<P: Capabilities>() -> Handlers<P>`).
  `DocumentStore` and `TableStore` join it under the `docstore` and `sql`
  features.
- `Error::{bad_request, not_found, server_error, bad_gateway}(code,
  description)`: constructors that name the wire `error` code, for a
  specification's own reason codes. The macros of the same names build
  the variant under its default code (`bad_request`, ...).
- `omnia_core::HostView<'a, C>`: the linker-facing view of a host over one
  client, `{ client: &'a C, table: &'a mut ResourceTable }`, with
  `HasTable`. `#[derive(HostCtx)]` names the client trait in
  `#[host_ctx(client = XClient)]` and makes `HostView<'a, dyn XClient>`
  the host's `HasData::Data`; the bindgen `Host*` impls go on
  `HostView<'_, dyn XClient>`.
- `omnia_core::Backends`: the shape of a deployment's backend bundle,
  `Clone + Send + Sync + 'static`, implemented for every such type. It is
  the one bound `Runtime<B>`, `Host<B>`, `Wiring<B>`, and `Deployment<B>`
  put on the bundle; a bundle that connects itself is `omnia::ConnectAll`.
- `StoreCtx::{view, mounts, dispatcher, extensions, chain}`: the store's
  own accessors. `view::<H>()` assembles host `H`'s view for a bundle
  `B: Provides<H>`, and is the getter every `add_to_linker` passes.
- `#[derive(thiserror::Error)]` on `GuestError`, `AcquireError`,
  `LoadError`, `HandleError`, and `McpError`, with the same messages.

### Changed

- `Host<B>` is over the backend bundle, no longer the store type, and
  carries `run`: `add_to_linker(&mut Linker<StoreCtx<B>>)` plus a
  `run(&Runtime<B>)` whose default resolves at once and which a trigger
  host (`WasiHttp`, `WasiMessaging`, `WasiWebSocket`) overrides with its
  serve loop. A host implements it once, `impl<B: Backends +
  Provides<Self>> Host<B>`; a hand-built bundle needs no store trait.
- `omnia::Backends`, the trait the generated bundle implements to connect
  every backend, is `omnia::ConnectAll`; `Backends` now names the bundle
  shape (`omnia_core::Backends`), and `ConnectAll: Backends`. `Host<B>`,
  `Wiring<B>`, `Deployment<B>`, and the generic fns over a bundle bound it
  `B: Backends` in place of `Clone + Send + Sync + 'static`.
- `Deployment<B>`, `DeploymentBuilder::build::<B>()`,
  `Deployment::host::<H>()`, and `Wiring::link(&mut Deployment<B>)` take
  the bundle alone: `build::<StoreCtx<Backends>>()` is
  `build::<Backends>()`, and `host::<WasiOtel, Backends>()` is
  `host::<WasiOtel>()`. `omnia_test::host::Deployment::{boot, run}` hand
  their closure a `&mut Deployment<B>`.
- `Registry<B>`, `RegistryParts<B>`, `Guest<B>`, `LinkSeam<B>`,
  `StoreFactory<B>`, and `FreshCall<B>` are over the bundle, each fixing
  the store to `StoreCtx<B>`: `Registry<StoreCtx<Backends>>` is
  `Registry<Backends>`, and `Runtime::registry` and `Runtime::guest`
  return `&Registry<B>` and `Arc<Guest<B>>`.
- `runtime!` names each backend field by its row's position, so two
  backends whose paths end in the same name (`keyvalue::Client`,
  `model::Client`) no longer share a field.

- `command_with`'s decoder returns `DecodeError`, as `topic_with`'s does,
  and `Command::call(handler, input)` takes the decoded input.
  `Format::encode(body)` renders through `Render`, and `Failure` renders
  its envelope through `Render` in place of `Failure::text`.
- `api::Format` and `api::Encoded` are `api::command::Format` and
  `api::command::Encoded`, beside `Render`; nothing outside the command
  façade reads them.
- A `serde_json::Error` converts into a `BadRequest` under the code
  `bad_request`, no longer `serde_json`.
- An HTTP handler bounds its `P` on the capability traits it reaches
  alone: the `'static` axum needs is the router fn's, which `Handlers<P>`
  carries, so `async fn handler<P: Config + StateStore>(context:
  Context<P>, ..)` registers on either transport.

### Removed

- `omnia_sdk::api::command::{Commands, Router}`: subcommands are
  `Handlers::command(name, handler)` beside a module's paths and topics,
  the output's `Render` impl replaces the per-verb render fn, and the
  router's `run`/`run_with` replace `handle`/`handle_with`.
  `Commands::map` has no successor.
- `Router::{new, route, topic, topic_with, command, command_with, merge}`:
  every handler is registered on a `Handlers<P>` and the router is built
  by `Handlers::with_state` alone, so there is one way to register and
  one to close. `Router::new(client).topic(..)` is
  `Handlers::new().topic(..).with_state(client)`.
- `omnia_sdk::api::http::Router`, the alias of `axum::Router`.
- `Client::call`: a handler runs through the router's dispatch, inside its
  `handler` span, and a unit test calls the fn directly over
  `Context::new(provider, metadata)`.
- `omnia_core::Server`: folded into `Host::run`.
- `omnia_core::{StoreView, HasMounts, HasDispatcher, HasExtensions,
  HasChain, HasLimits}` and `omnia::LinkStore`: `StoreCtx<B>` is the one
  store type, so its inherent methods replace them; the generated
  `Wasi*View` types are the one `omnia_core::HostView`.
- `omnia_core::serve_links`: `Runtime::serve_links` is the one entry.
- The 13 client-trait re-exports at `omnia_backend`'s root
  (`omnia_backend::KeyValueClient`, ...): each lives in its module
  (`omnia_backend::keyvalue::KeyValueClient`), as the `omnia-wasi-*`
  crates re-export them.

---

Release notes for previous releases can be found on the respective release branches of the repository.

<!-- ARCHIVE_START -->
* [0.37.x](https://github.com/augentic/omnia/blob/release-0.37.0/RELEASES.md)
* [0.36.x](https://github.com/augentic/omnia/blob/release-0.36.0/RELEASES.md)
* [0.35.x](https://github.com/augentic/omnia/blob/release-0.35.0/RELEASES.md)
* [0.34.x](https://github.com/augentic/omnia/blob/release-0.34.0/RELEASES.md)
* [0.33.x](https://github.com/augentic/omnia/blob/release-0.33.0/RELEASES.md)
* [0.32.x](https://github.com/augentic/omnia/blob/release-0.32.0/RELEASES.md)
* [0.31.x](https://github.com/augentic/omnia/blob/release-0.31.0/RELEASES.md)
* [0.30.x](https://github.com/augentic/omnia/blob/release-0.30.0/RELEASES.md)
* [0.29.x](https://github.com/augentic/omnia/blob/release-0.29.0/RELEASES.md)
* [0.28.x](https://github.com/augentic/omnia/blob/release-0.28.0/RELEASES.md)
* [0.27.x](https://github.com/augentic/omnia/blob/release-0.27.0/RELEASES.md)
* [0.25.x](https://github.com/augentic/omnia/blob/release-0.25.0/RELEASES.md)
* [0.23.x](https://github.com/augentic/omnia/blob/release-0.23.0/RELEASES.md)
* [0.22.x](https://github.com/augentic/omnia/blob/release-0.22.0/RELEASES.md)
* [0.21.x](https://github.com/augentic/omnia/blob/release-0.21.0/RELEASES.md)
* [0.20.x](https://github.com/augentic/omnia/blob/release-0.20.0/RELEASES.md)
* [0.19.x](https://github.com/augentic/omnia/blob/release-0.19.0/RELEASES.md)
* [0.18.x](https://github.com/augentic/omnia/blob/release-0.18.0/RELEASES.md)
* [0.17.x](https://github.com/augentic/omnia/blob/release-0.17.0/RELEASES.md)
* [0.16.x](https://github.com/augentic/omnia/blob/release-0.16.0/RELEASES.md)
* [0.15.x](https://github.com/augentic/omnia/blob/release-0.15.0/RELEASES.md)
