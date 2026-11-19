# Omnia WASI Types

This crate provides `omnia:types`, the WIT types every Omnia interface
shares.

## Interface

- `error` is the failure every `omnia:*` call reports. Each package's
  `types` interface uses it, and says what each case means for that
  capability.
- `timestamp` is an instant, in milliseconds since the Unix epoch.
- `json` is a JSON document, as UTF-8 text.

## Bindings

- **Guest**: every `omnia-wasi-*` guest binding maps `omnia:types/types` to
  `omnia_wasi_types::omnia::types`. A guest matches one `Error` whichever
  capability returned it.
- **Host**: each host crate binds its own copy of `omnia:types/types`, so
  its `Error` stays its own. Every host defines the same instance on the
  linker, and a later host reopens it.

## License

MIT OR Apache-2.0
