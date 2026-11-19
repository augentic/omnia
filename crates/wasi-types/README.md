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
- **Host**: each host crate binds its own copy of `omnia:types/types`, so
  its `Error` stays its own. Every host defines the same instance on the
  linker, and a later host reopens it.

## License

MIT OR Apache-2.0
