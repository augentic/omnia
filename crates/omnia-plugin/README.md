# omnia-plugin

The `omnia:plugins/loader` capability for the [omnia](https://github.com/augentic/omnia) runtime — the guest loader. The deployment's `[[guest]]` list is the allow-list of everything that may run; an entry marked `on_demand` is admitted not at boot but when a caller first names it through `load(name)`. The host acquires the bytes from the entry's declared source (a file path, embedded bytes, or a registry package), checks them against the entry's declared `digest`, and admits them through the runtime's admission seam. Nothing a caller passes chooses code — no bytes, no paths, no registry endpoints cross the interface — and every trust decision stays host-side. A name that is already active is attested, not fetched; an undeclared name is refused.

Everything loader lives here:

- the loader WIT and the `WasiPlugins` host binding,
- the `Plugins` load path (declared-name lookup, digest pin, idempotency) over `omnia-core`'s privileged `Runtime::admit` seam, reachable host-side through `PluginLoader` on `Runtime`,
- the on-demand `Source` table `Plugins::install` takes (`omnia::Source`, the same type a boot guest loads through — one pin check and one `wasm_only` policy for both), and the `RegistrySource` seam that fetches a `SourceSpec::Package` — by default a `RegistryClient` routed by the deployment's `registries` configuration (the `runtime!` macro's `registries:`, a manifest's `[registries]`); `Deployment::assemble` installs the manifest's on-demand entries, and `Deployment::registry_source` selects a custom registry (a caching `RegistryClient::cached`, say) before assembly.

Depend on the `omnia` facade, not on this crate: the whole surface re-exports there behind `omnia`'s non-default `loader` feature (`omnia::WasiPlugins`, `omnia::Plugins`, `omnia::RegistryClient`, …), including the traits a store-backed `RegistryClient::cached` acquirer implements — `omnia::ContentStore` and `omnia::ReleaseStore` from here, `omnia::Backend` and `omnia::NoOptions` from the runtime core. A direct dependency on `omnia-plugin` (or on `omnia-core`) is only for building another capability crate of your own.

## License

MIT OR Apache-2.0
