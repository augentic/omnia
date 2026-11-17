## 0.38.0

Unreleased

### Added

- `omnia_sdk::WasiProvider`: one zero-sized provider that implements every
  capability trait over the WASI bindings on `wasm32`. It replaces a guest's
  provider struct with one empty impl per capability, and the `WasiModel`,
  `WasiVcs`, and `WasiPlugins` handles.
- `omnia_sdk::wasi::{identity, keyvalue, sql, vault}::Handle`, on `wasm32`:
  the handle `named(name)` answers on `WasiProvider`.
- `#[derive(Provider)]`, `#[derive(Settings)]`, and `#[derive(FromRow)]`,
  re-exported by `omnia_sdk`:
  - `Provider` composes a provider from one field per capability. A field
    names an `omnia_sdk` capability bare, as in `#[provider(Config)]`, and
    any other by path, as in `#[provider(crate::geo::Geocoder)]`.
  - `Settings` loads a part's required configuration in one call.
  - `FromRow` decodes a SQL row into a record type.
- `#[omnia_sdk::capability]`, which makes a trait declared outside
  `omnia_sdk` a capability a `Provider` field delegates.
- `omnia_sdk::api::{Handlers, Router}`: a module registers its HTTP routes
  (`route`), messaging topics (`topic`, `topic_with`), and CLI subcommands
  (`command`, `command_with`, behind the `command` feature) on one
  `Handlers<P>`, and `with_state(provider)` closes them into the one
  `Router<P>`. `http::export!`, `messaging::export!`, and
  `command::export!` bind the WASI exports. `command::export!` stamps the
  guest crate's version and description as `--version` and `--help`.
- `SecretStore`, the secrets capability over `omnia:vault`. A handle
  addresses one locker, `default` until `named(name)` selects another, and
  `keys(after)` lists it in key order.
- One `Context` accessor per capability: `blobs`, `config`, `docs`, `http`,
  `identity`, `model`, `plugins`, `publisher`, `secrets`, `sql`, `state`,
  `vcs`, and `websockets`. Each borrows the provider as that one trait, so
  a handler bounded on `Config + StateStore` calls `get` without UFCS.
- `omnia:keyvalue`, the Omnia-owned interface `StateStore` calls.
  `WasiKeyValue` serves it over one `Bucket`:
  - `store.open` answers a `bucket` handle, and every call goes through
    it.
  - A write names its `lifetime`: `forever`, a TTL in milliseconds, or one
    of two that leave an existing entry the time it has left.
  - A write returns the entry's `revision`. `set` takes an `absent` or
    `matches` precondition, and one that fails answers a `conflict`
    carrying the entry it found.
  - `get-many`, `set-many`, and `delete-many` take several keys at once.
  - The optional `keys` interface pages the keys under a prefix.
  - `WasiKeyValue` answers `unsupported` for a `delete` with a revision,
    and for a `matches` write or an `increment` that replaces the
    lifetime.
- `omnia:sql`, the Omnia-owned interface `SqlDb` calls. `WasiSql` serves it
  beside the `wasi:sql` draft over one `Connection`. Each call names its
  database, and `batch` runs its statements in one transaction.
- `omnia:blobstore`, the Omnia-owned interface `BlobStore` calls.
  `WasiBlobStore` serves it over one `BlobStoreClient`:
  - `store.open` answers a `container` handle, and every object call goes
    through it.
  - `get` streams an object, or a `byte-range` of it: a half-open span, an
    offset to the end, or a suffix.
  - `put` streams an object in, and stores it once the writer commits.
  - `list` pages the names under a prefix, flat or folded at the next `/`.
  - The optional `admin`, `uploads`, and `presign` interfaces manage
    containers, assemble an object from parts, and presign requests.
  - `WasiBlobStore` answers `unsupported` for a precondition, for object
    attributes, and for `list-containers`, `uploads`, and `presign`.
- The codes of a failed capability call. Its `omnia_sdk::Error` names the
  class of failure in its code:
  - `not-found`, `denied`, `invalid`, `unsupported`, or `other` on a
    `ServerError`, a `500`.
  - `unavailable` on a `BadGateway`, a `502`, when the upstream did not
    answer or the call exceeded its time bound.
- `omnia_sdk::Page`, one page of a listing and the cursor that resumes it.
  `BlobStore::list`, `DocumentDb::query`, and `SecretStore::keys` answer
  one.
- `omnia_test::contract::state` (behind the `contract` feature, natively
  and on `wasm32`), the `StateStore` behaviour rows: one generic `async fn`
  per invariant, run natively over `omnia_test::guest::Memory` and in a
  guest over `WasiProvider`.
- `omnia_test::contract::{sql, document}`, the `SqlDb` and `DocumentDb`
  behaviour rows. Both run in a guest over `WasiProvider`, and the document
  rows run natively over `omnia_test::guest::MemoryDocs` too.
- `omnia_test::contract::blob`, the `BlobStore` behaviour rows, run natively
  over `omnia_test::guest::Memory` and in a guest over `WasiProvider`. Each
  row takes a `scope` that prefixes its container names, so a rerun against
  a live store uses fresh names.
- `BlobStore` documents the portable container and object names every
  provider accepts.
- `omnia_wasi_keyvalue::ConnectOptions`. `KeyValueDefault` serves the
  buckets `KEYVALUE_BUCKETS` lists, comma-separated, and `default` when it
  is unset.
- `omnia_wasi_sql::ConnectOptions::databases`. `SqlDefault` serves one
  `SQLite` database per declared name:
  - `SQL_DATABASE` stays the URI of `default`.
  - `SQL_DATABASES` lists the further names, comma-separated.
  - `SQL_DATABASE__<NAME>` is the URI of the listed name, which is a
    private in-memory database without one.
- `omnia_wasi_identity::StubOptions`. `IdentityStub` serves the identities
  `IDENTITY_NAMES` lists, comma-separated, and `default` when it is unset.
  Each identity's token is distinct.
- Extension traits, blanket-implemented over each capability:
  `ConfigExt::{require, parse}`, `DocumentDbExt::get_as`,
  `HttpClientExt::fetch_json`, `IdentityExt::access_token`,
  `StateStoreExt::{get_json, set_json}`, and `SqlDbExt::query_as`.
- `Error::{bad_request, not_found, server_error, bad_gateway}(code,
  description)`, which name the wire `error` code.
- `Document::{new, json, decode}`, `Filter::{depth, check, matches}`, and
  `SortField::compare` in `omnia_sdk::docstore`. `Filter::matches` and
  `SortField::compare` are the reference query semantics, and
  `omnia_test::guest::MemoryDocs` evaluates its queries with them.
  `Filter::check` holds a filter to `Filter::MAX_DEPTH` and
  `Filter::MAX_LIST_LEN`.
- `omnia_sdk::messaging::Message::with_key` and
  `omnia_sdk::api::messaging::Delivery::key`. A message's key crosses the
  wire as its `key` metadata entry.
- `omnia_test::guest::{send, deliver}`, which drive a router natively, and
  `omnia_test::guest::ScriptedVcs`, a scripted `Vcs`.
- `Metadata::deadline` carries the instant the guest timeout expires. Each
  carrier holds it as decimal milliseconds since the Unix epoch:
  - The HTTP host stamps the `x-deadline` header.
  - The messaging host stamps the `deadline` metadata entry.
  - `Metadata::from_env` reads `<PREFIX>_DEADLINE`.

  A host's stamp replaces any inbound value. A command runs uncapped and
  carries none.
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
- `omnia_plugin::loader`, on `wasm32`: the guest bindings for
  `omnia:plugins/loader`. `omnia-sdk` serves `Plugins` through them and
  carries no WIT of its own.

### Changed

- omnia runs WASI 0.3 guests alone. Guests build for `wasm32-wasip3`, and
  the MSRV is Rust 1.100. The linker no longer serves WASI 0.2, so a guest
  that imports a WASI 0.2 `wasi:cli`, `wasi:clocks`, `wasi:filesystem`,
  `wasi:http`, `wasi:io`, `wasi:random`, or `wasi:sockets` interface is
  refused by name, with the rebuild it needs, at startup or at its first
  use. The `wasi:config`, `wasi:messaging`, and `wasi:sql` drafts are
  served as before.
- `omnia_test::build` compiles a fixture crate's programs for
  `wasm32-wasip3`.
- A raw export that may wait is an `async func`. On `wasm32-wasip3`,
  `std::thread::sleep` and every async-typed import suspend the task, and
  a sync-typed export that suspends traps with `cannot block a synchronous
  task before returning`. The exports behind the `export!` macros are
  already async-typed.
- A handler takes its `Context<P>` first and its input second. An HTTP
  handler is a plain axum handler that takes `Context<P>` as an extractor
  over the `Arc<P>` in the router state. A handler that takes `Json<T>`
  answers `415` to a request without `content-type: application/json`.
- `omnia_sdk::api::Client` is gone, and the provider is the router's state:
  - `with_state(Client::new(provider))` becomes `with_state(provider)`.
  - A raw axum router closes with `with_state(Arc::new(provider))`.
  - `Command::new` takes the `&Arc<P>` where it took a `&Client<P>`.
- `omnia_sdk::Error` implements `IntoResponse`, so a handler answers
  `omnia_sdk::Result<T>`. `HttpResult` and `HttpError` are gone.
- `omnia_sdk::command!(main)` is `omnia_sdk::api::command::export!(main)`.
- The fns a hand-written WASI export calls live in `omnia_sdk::wasi`, on
  `wasm32`:
  - `wasi::http::serve`, which was `api::http::serve`. It takes any router
    that converts into an `axum::Router`, opens the guest telemetry scope,
    and layers `api::http::trace`, so the export does neither itself.
  - `wasi::messaging::handle`, which was `api::messaging::handle`. It opens
    the guest telemetry scope.
  - `wasi::command::execute`, which was `api::command::execute_wasi`.
- The value types live in the SDK's own modules:
  - `omnia_sdk::docstore`, which was `omnia_sdk::document_store`.
  - `omnia_sdk::sql` (`Value`, `Row`, `Field`, `Statement`, `FromRow`).
    `omnia_wasi_sql` exported `Row` and `Field`, and `DataType` is `Value`.
  - `omnia_sdk::messaging::Message`, which was `omnia_sdk::Message`.
  - `omnia_sdk::blobstore::{ObjectMetadata, ContainerMetadata}` and
    `omnia_sdk::identity::AccessToken`.
  - A `DEFAULT` binding name in `omnia_sdk::{identity, keyvalue, sql,
    vault}`, and `omnia_sdk::keyvalue::Conflict`.
- Every capability trait but `Vcs`, `Plugins`, and `Model` answers
  `omnia_sdk::Result`. Those three answer their module's typed `Error`,
  which converts into an `omnia_sdk::Error` under its `code()`. A trait
  over named bindings (`Identity`, `SecretStore`, `SqlDb`, and
  `StateStore`) addresses `default` until `named(name)` selects another
  name the deployment declares. Migrating a 0.37 guest:
  - `Config::get` answers `Option<String>`, so an unset key is `Ok(None)`.
    `ConfigExt::require` fails one with the code `not-found`, and
    `ConfigExt::parse` answers it as `None`.
  - `HttpRequest` is `HttpClient`, and `fetch` takes a `Request<Bytes>`.
    `fetch_json` takes the output type alone.
  - `Identity::access_token(identity)` is `named(identity)` and then
    `token(scopes)`, which answers an `AccessToken`. Its `expires_in` is a
    `Duration`, and its `Debug` redacts the token.
    `IdentityExt::access_token()` answers the token alone.
  - `Publish::send` is `Publisher::publish`. A `Message` payload is
    `Bytes`, and `Message::new` takes `&[u8]`.
  - `Broadcast::send(name, data, sockets)` is `WebSockets::broadcast(data)`
    to every socket, or `WebSockets::send_to(sockets, data)`.
  - `TableStore` is `SqlDb`. `query` and `execute`, which was `exec`, take
    the SQL and its `&[Value]` on the handle, not a connection name.
    `execute` answers a `u64`, and `batch` runs `Statement`s.
  - `DocumentStore` is `DocumentDb`. `query` takes `&Query`, which was
    `QueryOptions` with `after` for `continuation`, and answers a
    `Page<Document>`. `insert` answers `Ok(Err(Exists))` for an id already
    stored, and a `Document` payload is `Bytes`.
  - `BlobStore` answers `None` for a missing object. A call on an object
    in a container that does not exist fails with the code `not-found`,
    and `delete_container` succeeds when the container is absent.
    `get_range` takes a half-open `Range<u64>` clamped to the object, and
    `list(container, prefix, after)` answers a `Page<ObjectMetadata>`.
    `object_info` is `stat`. The `BlobStoreExt` methods `has`,
    `delete_objects`, `copy_object`, `move_object`, and `clear` are the
    provided `exists`, `delete_many`, `copy`, `rename`, and `clear`.
  - `ObjectMetadata`, `ContainerMetadata`, `AccessToken`, `Document`,
    `Message`, `Delivery`, and `Statement` are `#[non_exhaustive]`, so
    code outside the SDK builds them with `new`. A metadata `created_at`
    is a `SystemTime`.
- `SqlDb` calls `omnia:sql`, one host call per method, and `batch` runs its
  statements in one transaction. A failing statement rolls back the whole
  batch.
- `BlobStore` calls `omnia:blobstore`, opening a `container` handle for
  each object call. Its codes come from the error's case, where the SDK
  read a missing container or object from the message.
- `SqlDefault` opens only the names it declares, and refuses any other.
  A guest that opened a connection by another name, such as `db`, opens
  `default` or lists the name in `SQL_DATABASES`.
- `SqlClient::open` fails a name the deployment does not declare with
  `omnia_wasi_sql::Error::NotFound`.
- `omnia_wasi_sql::Connection` serves `omnia:sql` and the draft alike:
  - `query` and `execute`, which was `exec`, take a `Vec<Value>`, and
    `execute` answers a `u64`.
  - `batch` runs a `Vec<Statement>` in one transaction. A `Statement` is
    its `sql` and `params`.
  - A `Row` has no `index`.
  - A backend picks a failure's code by returning an
    `omnia_wasi_sql::Error`, whose six cases are the six capability error
    codes.
- `SqlDefault` stores a `date` or `time` as the text it is given. It stored
  one as `NULL` before.
- `Row::get` reads a `bool` from an integer `0` or `1` too, which is how
  SQLite returns a `BOOLEAN`.
- The `omnia:docstore` `error` has one case per capability error code, and
  `insert` answers `already-exists` for an id already stored. The SDK
  reads both without matching message text.
- `DocStoreClient::insert` answers `false` for an id already stored, and a
  backend picks a failure's code by returning an
  `omnia_wasi_docstore::Error`.
- `DocumentDb::query` fails a filter past its limits with the code
  `invalid` before the query reaches the host, where the guest
  trapped before. A filter nests at most 5 deep, a list holds 1 to 100
  values, and an `and` or `or` holds a child. `MemoryDocs` refuses the
  same filters, and the host's `in-list` and `not-in-list` constructors
  refuse an empty list.
- `IdentityStub` refuses an identity `IDENTITY_NAMES` does not list, and
  `IdentityDefault` serves the identity `default` alone. A refused name is
  `no-such-identity`, which `Identity` reports with the code `not-found`.
  A backend picks a failure's code by returning an
  `omnia_wasi_identity::Error`.
- `BlobStoreClient` states the container rules every backend keeps:
  - `get_container` fails a container that does not exist with
    `omnia_wasi_blobstore::Error::missing_container`.
  - `create_container` opens a container that already exists.
  - `delete_container` succeeds when the container is absent.

  A backend picks a failure's code by returning an
  `omnia_wasi_blobstore::Error`, whose six cases are the six capability
  error codes.
- `omnia_wasi_blobstore::Container` changes shape:
  - `get_data` takes a half-open `Range<u64>`, clamped to the object. An
    inverted range, which a backend refused, reads as empty.
  - `object_info` answers `None` for an absent object.
  - `list_objects(prefix, after)` answers one name-ordered `Page` of
    `ObjectMetadata`.

  `ContainerMetadata`, `ObjectMetadata`, and `Page` are the crate's own
  structs, which the host maps onto the `omnia:blobstore` records. The
  host resolves a `byte-range` to a `Range<u64>`, and folds and limits a
  listing itself.
- `StateStore` calls `omnia:keyvalue`, opening a `bucket` handle for each
  call:
  - `get` answers `Option<Bytes>`.
  - `set` answers `()`, and a TTL is an `Option<Duration>`. A zero TTL
    fails with the code `invalid` and writes nothing.
  - `cas` is `compare_and_swap`. It writes under the `absent`
    precondition, or under the `revision` it read the expected value at.
    A mismatch answers `Ok(Err(Conflict))` with the value it found.
  - `increment` takes a TTL. Like `compare_and_swap`, it applies the TTL
    only to an entry it creates.
  - `named(name)` selects another bucket the deployment declares.
  - A portable key works on every backend: it holds only ASCII letters,
    digits, `.`, `_`, and `-`, and neither starts nor ends with `.`. A
    backend may reject any other key with the code `invalid`.
- `StateStoreExt::{get_json, set_json}` fail a value that does not encode
  or decode with the code `invalid`.
- `StateStore` opens the `default` bucket, which was `cache`. The host
  stores a guest's bytes as they are and expires them itself, so a value a
  0.37 guest set with a TTL reads back as its JSON `Cacheable` envelope.
  `omnia_test::host::Backends::state` reads the raw bytes.
- Bucket names are strict. `KeyValueDefault` opens only the buckets it
  declares, and any other fails with `not-found`. A deployment whose guests
  open a bucket other than `default` lists it in `KEYVALUE_BUCKETS`.
- `Bucket::{set, increment}` take a TTL, and `Bucket::compare_and_swap`
  replaces `swap`. A backend picks a failure's code by returning an
  `omnia_wasi_keyvalue::Error`.
- `KeyValueDefault` and `omnia_test::guest::Memory` store a counter as
  decimal text, as `omnia:keyvalue` specifies, rather than as eight
  big-endian bytes. `increment` fails any other value with the code
  `invalid`.
- Each `Has<Trait>` names the type it delegates to as `Inner`, and its
  accessor returns `&Self::Inner`, so a capability's associated types
  forward to the field's. A `pub` provider that derives `Provider` needs
  `pub` field types.
- `omnia_test::guest::Scripted` is `ScriptedModel`, and
  `omnia_test::guest::Memory` implements `SecretStore`.
- `omnia_test::guest::Memory` keeps state per bucket and secrets per
  locker. `Memory::bucket` declares a bucket beside `default`,
  `Memory::named` addresses another bucket and locker of the same store,
  and `Memory::snapshot` answers state by bucket.
- `omnia_test::guest::Memory` fails a call on an object in a container
  nothing created with the code `not-found`, as the runtime does. A test
  creates the container first, or seeds it with `insert_container` or
  `insert_object`.
- The `omnia_test::guest` doubles follow the traits:
  - `Provider`'s `publish`, `broadcast`, and `tables` fields and builders
    are `publisher`, `websockets`, and `sql`.
  - `ScriptedTables::statements` answers `Executed`, which was
    `Statement`, and its `binding` was `connection`. `on_exec` takes a
    `u64`.
  - `Broadcasted` has no channel `name`.
  - `MapConfig` answers an unknown key as unset.
  - `FixedIdentity` answers a named identity's token as `<token>:<name>`.
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
  `2024-1-5` is an error. The error is an `omnia_sdk::Error` with the code
  `invalid`, which was an `anyhow::Error`.
- An `omnia_sdk::vcs::Error`, `plugins::Error`, or `model::Error` under
  `anyhow` context converts into `omnia_sdk::Error` with the code and
  status it has unwrapped, which was a `server_error`.
- A `model::Error` converts into `omnia_sdk::Error` under the code its
  `code()` names, as a `vcs::Error` or `plugins::Error` does:
  - `invalid-request` and `budget-exhausted` on a `BadRequest`, which were
    `bad_request`.
  - `tool-failed` and `backend` on a `BadGateway`, which were
    `bad_gateway`.
- An `omnia_sdk::Error` under `anyhow` context adds its description, not
  its display, to the context. `bad_request!("no name")` under the
  context `saving` describes itself as `saving: no name`, which was
  `saving: code: bad_request, description: no name`.
- `omnia_sdk::Error` implements `PartialEq` and `Eq`.
- `omnia_sdk::Error::code()` and `description()` answer `&str`, which was
  a cloned `String`.
- A messaging failure is an `omnia_sdk::Error`, so `Router::handle`
  answers `omnia_sdk::Result<()>`. Its code names the failure:
  - `missing-topic`, a `BadRequest`, is a delivery with no topic, which
    was `DeliveryError::MissingTopic`.
  - `unhandled-topic`, a `NotFound`, is a topic no route takes, which was
    `DeliveryError::UnhandledTopic`.
  - `undecodable`, a `BadRequest`, is a payload that did not decode,
    whatever error a decoder refused it with. A broker drops it.
  - A handler's error takes the variant and code of the first
    `omnia_sdk::Error`, `vcs::Error`, `plugins::Error`, or `model::Error`
    in its source chain, and is otherwise a `ServerError`. Its description
    is the whole chain. A broker redelivers it when it is a `BadGateway`.

  Over the draft `wasi:messaging`, every failure still crosses as
  `error.other` carrying the error's display. A match on a `DeliveryError`
  variant becomes a match on the code.
- A subcommand reads its handler's error by the messaging rule above, so
  `Command::call` takes any handler, where it took one whose error
  converted into `Failure`. `Command::hints` and `Router::hints` consult
  their fn for every failure.
- `omnia_sdk::api::Metadata` is `#[non_exhaustive]`. Code outside the SDK
  builds one with `Metadata::default()` and assigns its fields, or with
  `from_lookup`, `from_headers`, `from_env`, or `minted`.
- A request id minted natively draws 128 bits from the operating system's
  CSPRNG, as one minted in a guest draws from `wasi:random`.
- `SqlDefault` binds each `$n` placeholder to the `n`th parameter wherever
  it appears, so `SET name = $2 WHERE id = $1` no longer binds its values
  swapped. A `?` or `?n` placeholder binds as before, and any other named
  placeholder fails.
- `omnia:otel` is `0.2.0`. Its timestamps are the WASI 0.3
  `wasi:clocks/system-clock@0.3.2` `instant`, where `0.1.0` carried the
  WASI 0.2 `wall-clock` `datetime`. The host reads an instant before the
  epoch as the epoch.

### Removed

- `omnia_sdk::api::http::{get, post, put, patch, delete, handle_with}`,
  `RawRequest`, and the `MethodFilter` re-export. A route is an axum
  `MethodRouter` bound with `Handlers::route`.
- `omnia_sdk::api::messaging::{consume, consume_with, Consume, Routes}`. A
  topic is bound with `Handlers::topic` or `Handlers::topic_with`.
- `Client::owner`.
- The hidden `omnia_sdk` re-exports of `axum`, `http`, `anyhow`, `bytes`,
  `http_body`, `tracing`, `wit_bindgen`, and the `omnia_wasi_*` crates
  other than `omnia_wasi_messaging`. A guest depends on the crates it
  names. `omnia_sdk::wasip3` stays.
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
  `omnia_sdk::DocumentDb`, and `omnia_wasi_docstore::{store, types}` are
  the raw `omnia:docstore` bindings.
- The `sql` and `docstore` features of `omnia-sdk`. `SqlDb` and
  `DocumentDb` are always built.
- `omnia_test::guest::Namespaced`. A test addresses another bucket or
  locker with `Memory::named`.
- `omnia_test::guest::MemoryDocs::backend`.
- `omnia_wasi_keyvalue::Cacheable` and the guest-side `cache` module, the
  envelope a value set with a TTL was stored in.
- `omnia_sdk::CasError`. A conflict is `omnia_sdk::keyvalue::Conflict`.
- `omnia_sdk::api::DecodeError` and its `api::http` re-export. A
  `topic_with` or `command_with` decoder answers `omnia_sdk::Result<I>`, so
  `DecodeError::new(message)` becomes `bad_request!("{message}")`.
- The host's `omnia_wasi_sql::DataType`, which is `omnia_wasi_sql::Value`,
  and `omnia_wasi_sql::Error::trace`. The draft's `trace` is the message
  of the `Error` case a backend returns.
- The `wasi:blobstore@0.2.0-draft` imports, whose values are `wasi:io@0.2`
  streams a WASI 0.3 runtime does not serve. With them go
  `omnia_wasi_blobstore::{blobstore, container, types}` on `wasm32`, and
  the host's `ContainerProxy`, `IncomingValue`, `OutgoingValue`, and
  `StreamObjectNames`. A guest reaches a blob store through
  `omnia_sdk::BlobStore`, and `omnia_wasi_blobstore::omnia` holds the raw
  `omnia:blobstore` bindings.
- The `wasi:keyvalue@0.2.0-draft2` imports. With them go
  `omnia_wasi_keyvalue::{store, atomics, batch}` on `wasm32`, and the
  host's `BucketProxy` and `Cas`. A guest reaches a key-value store
  through `omnia_sdk::StateStore`, and `omnia_wasi_keyvalue::omnia` holds
  the raw `omnia:keyvalue` bindings.

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
