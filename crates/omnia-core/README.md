# omnia-core

The runtime core of [omnia](https://github.com/augentic/omnia): deployment
assembly, the multi-guest registry, host-mediated dispatch, per-store
contexts, and telemetry.

Embedders depend on the `omnia` facade crate, which re-exports this crate's
surface together with the capability crates built on it. Depend on
`omnia-core` directly only when building a capability crate — one that links a
host into the runtime and installs its state through the [`Wiring::extend`]
hook and the runtime's [`Extensions`]. `omnia-plugin` (the
`omnia:plugins/loader` capability) is the exemplar; `omnia-cli` (the `run`
command-line grammar) is the other crate built on this one.

[`Wiring::extend`]: https://docs.rs/omnia-core/latest/omnia_core/trait.Wiring.html
[`Extensions`]: https://docs.rs/omnia-core/latest/omnia_core/struct.Extensions.html
