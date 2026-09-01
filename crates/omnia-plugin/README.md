# omnia-plugin

The `omnia:plugins/loader` capability for the [omnia](https://github.com/augentic/omnia) runtime: a guest names code (package, location, optional sha256 pin) and the host acquires, verifies, and admits it — component bytes never cross the interface, and every trust decision stays host-side.

Everything plugin lives here:

- the loader WIT and the `WasiPlugins` host binding,
- the `Plugins` load path (pin policy, idempotency, acquisition routing) over `omnia-core`'s privileged `Runtime::admit` seam, reachable host-side through `PluginLoader` on `Runtime`,
- the acquisition policy — one slot per location kind — with the built-in `PathMounts` and `RegistryClient` policies, installed by `Plugins::install` from the deployment's `Wiring::extend` hook (the `runtime!` macro's `plugins: { locations: [...] }` list lowers into it).

The surface reaches embedders re-exported from the `omnia` facade (`omnia::WasiPlugins`, `omnia::PathMounts`, `omnia::Plugins`, …). Store implementors depend on this crate directly for `ContentStore` and `ReleaseStore`.

## License

MIT OR Apache-2.0
