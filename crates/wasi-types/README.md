# Omnia WASI Types

This crate provides `omnia:types`, the WIT types every Omnia interface
shares.

## Interface

- `error` is the failure every `omnia:*` call reports. Each package's
  `types` interface uses it, and says what each case means for that
  interface.
- `timestamp` is an instant, in milliseconds since the Unix epoch.
- `json` is a JSON document, as UTF-8 text.

## Bindings

- **Guest**: `omnia-sdk` binds every `omnia:*` package in one world, so a
  guest matches one `Error` whichever interface returned it.
- **Host**: this crate binds `omnia:types` once, and every host crate maps
  its vendored copy onto that binding with `bindgen!`'s `with`. So every
  host returns this crate's `Error`. Every host still defines the `types`
  instance on the linker, and a later host reopens it.
- An `anyhow::Error` that holds an `Error` converts to that `Error`, so a
  backend picks the kind a guest sees. Any other failure converts to
  `Error::Other`, with its full context chain.

## License

MIT OR Apache-2.0
