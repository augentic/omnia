# omnia-link

In-process guest→guest linking for the [omnia](https://github.com/augentic/omnia)
runtime: [`InProcessLinks`] implements the `LinkSeam` the registry drives,
polyfilling every import a guest makes outside the runtime's own namespaces
(`wasi:`, `omnia:`), serving every export outside them by in-memory routing to
a fresh callee task, and dispatching each call to the guest its leading string
argument names. Nothing declares the seam: a component says what it imports
and what it exports.

Depend on the `omnia` facade, not on this crate: `omnia`'s non-default `link`
feature installs the seam, and `omnia::is_host` re-exports from here.
A direct dependency on `omnia-link` (or on `omnia-core`) is never needed by a
deployment.

[`InProcessLinks`]: https://docs.rs/omnia-link/latest/omnia_link/struct.InProcessLinks.html

## License

MIT OR Apache-2.0
