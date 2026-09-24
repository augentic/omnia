# omnia-plugin

The `omnia:plugins/loader` capability for the [omnia](https://github.com/augentic/omnia) runtime — the guest loader: a guest names a location — a registry package (with the registry to fetch from, or the deployment's routing), a mount-relative path, or a guest the deployment declared — plus an optional sha256 pin, and the host acquires, verifies, and admits it — component bytes never cross the interface, and every trust decision stays host-side. A declared guest is attested, not fetched; a path load registers under its file's stem.

Everything loader lives here:

- the loader WIT and the `WasiPlugins` host binding,
- the `Plugins` load path (pin policy, idempotency, acquisition routing) over `omnia-core`'s privileged `Runtime::admit` seam, reachable host-side through `PluginLoader` on `Runtime`,
- the acquisition policy — one slot per origin kind — with the built-in `PathMounts` and `RegistryClient` policies. `Deployment::assemble` installs the declared policy through `Plugins::install_declared`: the deployment's mounts are the roots path loads resolve against, and its `registries` configuration (the `runtime!` macro's `registries:`, a manifest's `[registries]`) is the routing a package load naming no registry falls back on. `Plugins::install` takes custom slots, selected before assembly with `Deployment::loader`.

Depend on the `omnia` facade, not on this crate: the whole surface re-exports there behind `omnia`'s non-default `loader` feature (`omnia::WasiPlugins`, `omnia::PathMounts`, `omnia::Plugins`, …), including the traits a store-backed `RegistryClient::cached` acquirer implements — `omnia::ContentStore` and `omnia::ReleaseStore` from here, `omnia::Backend` and `omnia::NoOptions` from the runtime core. A direct dependency on `omnia-plugin` (or on `omnia-core`) is only for building another capability crate of your own.

## License

MIT OR Apache-2.0
