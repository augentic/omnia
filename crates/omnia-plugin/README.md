# omnia-plugin

Plugin acquisition for the [omnia](https://github.com/augentic/omnia) runtime: the `Acquirer` seam behind the `omnia:plugins/loader` capability — one slot per location kind — plus the built-in acquisition policies.

The `Acquirer` surface reaches embedders re-exported from `omnia` (`omnia::Acquirer`, `omnia::PathAcquire`, …). Store implementors depend on this crate for `ContentStore` and `ReleaseStore`.

## License

MIT OR Apache-2.0
