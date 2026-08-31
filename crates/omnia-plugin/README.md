# omnia-plugin

Plugin acquisition for the [omnia](https://github.com/augentic/omnia) runtime: the `Acquire` seam behind the `omnia:plugins/loader` capability, plus the built-in acquisition policies.

The `Acquire` surface reaches embedders re-exported from `omnia` (`omnia::Acquire`, `omnia::PathAcquire`, …). Store implementors depend on this crate for `ContentStore` and `ReleaseStore`.

## License

MIT OR Apache-2.0
