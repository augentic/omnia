# omnia-link

In-process guest→guest linking for the [omnia](https://github.com/augentic/omnia)
runtime: [`InProcessLinks`] implements the `LinkSeam` the registry drives,
polyfilling every import a guest makes outside the runtime's own namespaces
(`wasi:`, `omnia:`), serving every export outside them by in-memory routing to
a fresh callee task, and selecting the target guest per call. Nothing declares
the seam: a component says what it imports and what it exports.

Depend on the `omnia` facade, not on this crate: the whole surface re-exports
there behind `omnia`'s non-default `link` feature (`omnia::InProcessLinks`,
`omnia::GuestSelector`, `omnia::FirstArgSelector`).
A direct dependency on `omnia-link` (or on `omnia-core`) is never needed by a
deployment.

[`InProcessLinks`]: https://docs.rs/omnia/latest/omnia/struct.InProcessLinks.html

## License

MIT OR Apache-2.0
