## 0.38.0

Unreleased

### Added

- `omnia_sdk::WasiProvider`: one zero-sized provider that implements every
  capability trait over the WASI bindings on `wasm32`. It replaces a guest's
  provider struct with one empty impl per capability, and the `WasiModel`,
  `WasiVcs`, and `WasiPlugins` handles.
- `#[derive(Provider)]`, `#[derive(Settings)]`, and `#[derive(FromRow)]`,
  re-exported by `omnia_sdk`:
  - `Provider` composes a provider from one field per capability.
  - `Settings` loads a part's required configuration in one call.
  - `FromRow` decodes a SQL row into a record type.
- `omnia_sdk::api::{Handlers, Router}`: a module registers its HTTP routes
  (`route`), messaging topics (`topic`, `topic_with`), and CLI subcommands
  (`command`, `command_with`, behind the `command` feature) on one
  `Handlers<P>`, and `with_state(Client::new(provider))` closes them into
  the one `Router<P>`. `http::export!`, `messaging::export!`, and
  `command::export!` bind the WASI exports. `command::export!` stamps the
  guest crate's version and description as `--version` and `--help`.
- `omnia_sdk::Capabilities`, every capability trait at once, and the
  `Vault` capability.
- Extension traits, blanket-implemented over each capability:
  `ConfigExt::get_parsed`, `DocumentStoreExt::get_as`,
  `HttpRequestExt::fetch_json`, `IdentityExt::access_token`,
  `StateStoreExt::{get_json, set_json}`, and `TableStoreExt::query_as`.
- `Error::{bad_request, not_found, server_error, bad_gateway}(code,
  description)`, which name the wire `error` code.
- `Document::{json, decode}`, `Filter::{depth, matches}`, and
  `SortField::compare` in `omnia_sdk::docstore`. `Filter::matches` and
  `SortField::compare` are the reference query semantics, and
  `omnia_test::guest::MemoryDocs` evaluates its queries with them.
- `omnia_test::guest::{send, deliver}`, which drive a router natively, and
  `omnia_test::guest::ScriptedVcs`, a scripted `Vcs`.
- `omnia_core::Backends`, the shape of a deployment's backend bundle:
  `Clone + Send + Sync + 'static`, implemented for every such type. It is
  the one bound `Runtime<B>`, `Host<B>`, `Wiring<B>`, and `Deployment<B>`
  put on the bundle.
- `StoreCtx::{view, mounts, dispatcher, extensions, chain}`, the store's
  own accessors. `view::<H>()` assembles host `H`'s view for a bundle
  `B: Provides<H>`, and is the getter every `add_to_linker` passes.
- `omnia_core::HostView<'a, C>`, the linker-facing view of a host over one
  client: `{ client: &'a C, table: &'a mut ResourceTable }`, with
  `HasTable`. `#[derive(HostCtx)]` names the client trait in
  `#[host_ctx(client = XClient)]` and makes `HostView<'a, dyn XClient>`
  the host's `HasData::Data`. The bindgen `Host*` impls go on
  `HostView<'_, dyn XClient>`, and reach the backend through `client`.
  A crate that depends on `omnia` alone derives `omnia::HostCtx` with
  `#[host_ctx(client = XClient, crate = omnia)]`.
- `Runtime::stop` and `Runtime::stopped`. A trigger server waits on
  `stopped` to stop accepting work and drain. `Runtime::shutdown` stops
  the runtime too.

### Changed

- A handler takes its `Context<P>` first and its input second. An HTTP
  handler is a plain axum handler that takes `Context<P>` as an extractor
  over the `Client` in the router state. `Client::new(provider)` takes no
  owner. A handler that takes `Json<T>` answers `415` to a request without
  `content-type: application/json`.
- `omnia_sdk::Error` implements `IntoResponse`, so a handler answers
  `omnia_sdk::Result<T>`. `HttpResult` and `HttpError` are gone.
- `omnia_sdk::command!(main)` is `omnia_sdk::api::command::export!(main)`.
- The value types live in the SDK's own modules:
  - `omnia_sdk::docstore`, which was `omnia_sdk::document_store`.
  - `omnia_sdk::sql` (`DataType`, `Row`, `Field`, `FromRow`,
    `ColumnError`), which `omnia_wasi_sql` exported.
  - `omnia_sdk::blobstore`, `omnia_sdk::identity::AccessToken`,
    `omnia_sdk::keyvalue::BUCKET`, and `omnia_sdk::vault::LOCKER`.
- `TableStore::{query, exec}` take `&str` and `&[DataType]`,
  `StateStore::set` answers `()`, and `Identity::token(identity, scopes)`
  answers an `AccessToken`.
- `StateStore` opens the `state` bucket, which was `cache`. A value set
  with a TTL is still stored in the guest-side `Cacheable` envelope, and
  `omnia_test::host::Backends::state` reads through it.
- `omnia_wasi_keyvalue::Cacheable` is at the crate root on every target,
  and its `is_expired` is public.
- `omnia_test::guest::Scripted` is `ScriptedModel`, and
  `omnia_test::guest::Memory` implements `Vault`.
- `WasiHttp` is its own `HostCtx` carrier, like every other host. A bundle
  implements `Provides<WasiHttp>`, and `HttpBorrow` is
  `omnia_wasi_http::HttpBorrow`. `runtime!` no longer matches a `hosts:`
  row by the name `WasiHttp`, so an aliased row links.
- `Host<B>` is over the backend bundle, no longer the store type, and
  carries `run`. `add_to_linker` takes a `&mut Linker<StoreCtx<B>>`, and
  `run(&Runtime<B>)` resolves at once unless a trigger host (`WasiHttp`,
  `WasiMessaging`, `WasiWebSocket`) overrides it with its serve loop. A
  host implements it once, as `impl<B: Provides<Self>> Host<B>`.
- `omnia::Backends`, the trait the generated bundle implements to connect
  every backend, is `omnia::ConnectAll`, and `ConnectAll: Backends`.
- `Deployment<B>`, `DeploymentBuilder::build::<B>()`,
  `Deployment::host::<H>()`, and `Wiring::link(&mut Deployment<B>)` take
  the bundle alone. `build::<StoreCtx<Backends>>()` is
  `build::<Backends>()`, and `host::<WasiOtel, Backends>()` is
  `host::<WasiOtel>()`.
- `Registry<B>`, `RegistryParts<B>`, `Guest<B>`, `LinkSeam<B>`,
  `StoreFactory<B>`, and `FreshCall<B>` are over the bundle, each fixing
  the store to `StoreCtx<B>`. `Runtime::registry` and `Runtime::guest`
  return `&Registry<B>` and `Arc<Guest<B>>`.
- `runtime!` names each backend field by its row's position, so two
  backends whose paths end in the same name (`keyvalue::Client`,
  `model::Client`) no longer share a field.
- The HTTP trigger serves HTTP/1.1 and HTTP/2 on one listener. On stop it
  accepts no new connection and lets each open one finish its requests.
- A server-mode deployment drains on the first `SIGINT` or `SIGTERM`, and
  exits on the second.
- Without a `Forwarded` header, the HTTP trigger takes a request's scheme
  and authority from its URI before the `Host` header, so an HTTP/2
  request needs no `Host`.
- Guests link `wasip3` 0.10 and `wit-bindgen` 0.63. `omnia:model` and
  `omnia:vcs` use `wasi:filesystem` and `wasi:clocks` at 0.3.2.
- `omnia::Backend` is `omnia::Connect`.
- Each host's client trait is named for its interface: `Wasi<Name>Ctx` is
  `<Name>Client`, as in `KeyValueClient`, `BlobStoreClient`, and
  `ModelClient`.
- `WasiBlobstore` and `BlobstoreDefault` are `WasiBlobStore` and
  `BlobStoreDefault`. `omnia::WasiPluginsCtxView` is
  `omnia::WasiPluginsView`.
- The messaging and websocket resource trait `Client` is `Endpoint`, and
  `MessagingClient::endpoint` and `WebSocketClient::endpoint` replace
  `connect`. `ClientProxy` keeps its name.
- The document store's WIT package is `omnia:docstore`, which was
  `wasi:docstore`, so a guest built against `wasi:docstore` is rebuilt.
- Native builds of `omnia-sdk`, and of `omnia-test` with the `guest`
  feature, link no `wasmtime`. `omnia-wasi-docstore` is a `wasm32`-only
  dependency of the SDK, and `MemoryDocs` keeps its own in-memory store.
- `Filter::on_date` accepts only a zero-padded `YYYY-MM-DD` date, so
  `2024-1-5` is an error.

### Removed

- `omnia_sdk::api::http::{get, post, put, patch, delete, handle_with}`,
  `RawRequest`, and the `MethodFilter` re-export. A route is an axum
  `MethodRouter` bound with `Handlers::route`.
- `omnia_sdk::api::messaging::{consume, consume_with, Consume, Routes}`. A
  topic is bound with `Handlers::topic` or `Handlers::topic_with`.
- `Client::owner`.
- `BlobStoreExt`.
- `omnia::HttpCtx`, and the `WasiHttpView` re-export from `omnia_wasi_http`.
- `omnia::Server`, folded into `Host::run`.
- `omnia::{StoreView, HasMounts, HasDispatcher, HasExtensions, HasChain,
  HasLimits, LinkStore}`. `StoreCtx<B>` is the one store type, and its
  own methods replace them.
- `omnia::serve_links`. `Runtime::serve_links` is the one entry.
- `omnia::wasi_view!`, the `Wasi*CtxView` types it generated, and the
  `omnia::pastey` re-export. A host derives `HostCtx`, and its view is
  `HostView`.
- `omnia_wasi_docstore::document_store` and the SDK-typed
  `omnia_wasi_docstore::store` functions. The value types are
  `omnia_sdk::docstore`, a guest reaches the store through
  `omnia_sdk::DocumentStore`, and `omnia_wasi_docstore::{store, types}` are
  the raw `omnia:docstore` bindings.
- `omnia_test::guest::MemoryDocs::backend`.

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
