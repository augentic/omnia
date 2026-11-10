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
- `Document::{json, decode}` and `Filter::depth` in
  `omnia_wasi_docstore::document_store`.
- `omnia_test::guest::{send, deliver}`, which drive a router natively, and
  `omnia_test::guest::ScriptedVcs`, a scripted `Vcs`.

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

### Removed

- `omnia_sdk::api::http::{get, post, put, patch, delete, handle_with}`,
  `RawRequest`, and the `MethodFilter` re-export. A route is an axum
  `MethodRouter` bound with `Handlers::route`.
- `omnia_sdk::api::messaging::{consume, consume_with, Consume, Routes}`. A
  topic is bound with `Handlers::topic` or `Handlers::topic_with`.
- `Client::owner`.
- `BlobStoreExt`.

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
