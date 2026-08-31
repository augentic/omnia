# omnia-plugin

Plugin acquisition for the [omnia](https://github.com/augentic/omnia) runtime: the `Acquire` seam behind the `omnia:plugins/loader` capability, plus the built-in acquisition policies.

This crate is omnia-internal. Consumers depend on `omnia`, which re-exports this crate's surface under its own paths (`omnia::Acquire`, `omnia::PathAcquire`, …) — never name `omnia-plugin` directly.

## License

MIT OR Apache-2.0
