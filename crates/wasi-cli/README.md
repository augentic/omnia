# Omnia WASI CLI

This crate provides the `wasi:cli` (one-shot command) trigger for the Omnia
runtime.

## Interface

Drives a guest's `wasi:cli/run@0.3.0` export (WASI Preview 3) via
`wasmtime_wasi::p3::bindings::Command`.

## Behavior

`WasiCli` is a [`Server`] that instantiates the sole command-capable guest,
invokes `wasi:cli/run` exactly once through the shared `TriggerRouter`, and
records the guest's [`ExitStatus`] in a shared cell. The generated `main` reads
that cell at the process boundary and exits with the corresponding code. A
guest `wasi:cli/exit` surfaces host-side as `I32Exit` so specific non-zero exit
codes are preserved.

## Usage

`WasiCli` is wired in like any other host, but produces a one-shot `main` that
returns the guest's exit code:

```rust,ignore
omnia::runtime!({ main: true, hosts: { WasiCli } });
```

## License

MIT OR Apache-2.0
