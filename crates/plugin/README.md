# omnia-plugin

The `omnia:plugins` capability for the [omnia](https://github.com/augentic/omnia) runtime: the guest loader.

## Interface

Implements `omnia:plugins`, the interface the SDK's `plugins` module binds. A caller names a `source`, and the host acquires the bytes inside the deployment's grant, verifies them, and admits them through the runtime's admission seam under the name the source derives. Component bytes never cross the interface, and nothing a caller passes widens the grant.

- `declared(name)` names one of the deployment's `[[guest]]` entries. A name outside the guest list is `not-found`. One not yet loaded goes through the runtime's own first-use seam (`Runtime::guest`, the same one a route or a link call loads it through), and an active one is attested, not fetched. The entry carries the pin, so the source carries none.
- `path(file)` names a component beneath one of the deployment's read-only mounts, with an optional `pin`, and registers as its file stem. It loads through the mount it lies beneath alone:
  - a path that is not plain and relative is `invalid`;
  - one beneath no mount, or with no file, is `not-found`;
  - one beneath a writable mount, or a mount the loading guest's grants do not reach, is `denied` before the file is read, as is a link that leads out of its mount;
  - a writable mount overlapping a read-only mount's directory, by identity rather than path, is refused at assembly.
- `release(reference, registry?, pin?)` names an exact `namespace:name@version`, and registers as the reference without its version. The deployment's package store serves a release it holds. Otherwise the routing the deployment's `registries` declares fetches it, and it is written to the store once. The load's `registry` serves only a namespace that routing leaves open, and what it fetches is never stored. A `registry` other than the routed one is `denied`, and a release nothing routes or holds is `not-found`.
- A path or release that would register under a declared name is `denied` whether or not that guest has loaded, the name being its entry's alone.
- A guest loaded by path or release runs under the deployment's `[plugins.grants]` within the loading guest's grants, so it never reaches more than the guest that loaded it. A component importing code from a package those narrowed grants do not reach is `denied`. A `declared` load is not held to the loading guest's grants, since that guest runs under the grants its entry declares. The registry a `release` names is held to the deployment's routing and to the loading guest's `hosts` grant. A malformed one is `invalid`, and one whose host that grant does not reach is `denied` before any fetch. Each fetch builds a fresh client, and the redirects it follows are not checked.
- A path or release the caller names admits raw wasm alone. A declared `source.path` admits a pre-compiled artifact only when its entry pins the `digest`, since an unpinned file is read while guests run, and a declared `source.package` admits raw wasm alone. Bytes that break these rules, miss the entry's pin, or do not load as a component are `invalid`.
- A `pin` is a `sha256:<hex>` digest, and any other is `invalid`. A caller-named load whose bytes miss it, or whose name is active under other bytes, loads nothing and answers a `conflict`: the digest found, and the plugin active under the name.
- A mount, store, or registry that did not produce the bytes is `unavailable`, and a registration that failed is `other`.
- `WasiPlugins` answers `unsupported` for a runtime with no loader installed and for a release with no registry client.

## Crate map

Everything loader lives here:

- the `omnia:plugins` WIT and the `WasiPlugins` host binding,
- the `Plugins` load path (source resolution, idempotency): a `declared` name through `Runtime::guest`; a caller-named path read through the mount it lies beneath, a caller-named release fetched through `Runtime::acquire`, each admitted through `omnia-core`'s one admission body, `Runtime::admit_bytes` under `Policy::CallerNamed` with the deployment's `[plugins.grants]` narrowed by `Grants::within` to the loading guest's — the pin check, `Verified::wasm`, and the rule that a declared name is its entry's alone live there, for the loader and `Runtime::register` alike — reachable host-side through `PluginLoader` on `Runtime`,
- the grant `Plugins::install` fixes: the runtime's mounts as path roots; the grants a guest loaded by path or release runs under, within its loader's (the deployment's `[plugins.grants]`, else `Grants::all()`), checked against the runtime's mounts and inherited variables by `Runtime::validate_grants`; and the `RegistryClient` the runtime acquires every package through — the default `RegistrySource` (a trait of `omnia-core`, through which the runtime also acquires a `source.package` guest at first use), store first: a `PackageStore` keyed by exact `Reference` (`FsStore`, a flat directory of `namespace_name@version.wasm` files, over the deployment's `plugins.store`; `NoStore` without one) answers before any registry, and a release it lacks is fetched by the deployment's `registries` configuration (the `runtime!` macro's `plugins: { store, registries, grants }`, a manifest's `[plugins]`), verified, and written to the store once — one fetched from the `registry` a load names, for a namespace the configuration routes nowhere, is served to that load and never stored; `Deployment::assemble` installs both, and `Deployment::registry_source` selects a custom acquirer (a `RegistryClient` over a store of your own, say) before assembly.

Depend on the `omnia` facade, not on this crate: the whole surface re-exports there behind `omnia`'s non-default `loader` feature (`omnia::WasiPlugins`, `omnia::Plugins`, `omnia::RegistryClient`, …), including the trait a store of your own implements — `omnia::PackageStore` from here, with `omnia::FsStore`, `omnia::NoStore`, and `omnia::Reference` beside it — and `omnia::Connect` and `omnia::NoOptions` from the runtime core. A direct dependency on `omnia-plugin` (or on `omnia-core`) is only for building another capability crate of your own.

## License

MIT OR Apache-2.0
