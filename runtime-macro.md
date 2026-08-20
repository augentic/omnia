### `link: ["emery:adapter/source@0.1.0"]`

**Fallback.** `emery:adapter/source@0.1.0` is no longer host-mediated.
Guests importing it fail at instantiation (unsatisfied import) unless they
carry their own per-guest `link:`. Re-suppliable at the CLI via `run --link`.

**Recommendation.** Move the interface from the deployment-wide key to a
per-guest `link:` on the emery guest entry — the importer.
`Manifest::link_interfaces()` unions deployment-wide and per-guest lists
onto the same shared linker (`crates/omnia/src/deployment/manifest.rs`), so
behavior is identical while the declaration sits next to the guest that
needs it:

```toml
[[guest]]
id = "emery"
source.path = "./emery.wasm"
link = ["emery:adapter/source@0.1.0"]
```

Ad-hoc runs against a bare wasm path can still supply `run --link emery:adapter/source@0.1.0` at the CLI.

## Cross-key constraint

None remains: the `program:` and `command_guest:` keys are gone from the
macro, and command mode with a compiled-in deployment (`config:` or inline
keys) is a direct command — raw argv passthrough — by default (`plan` in
`crates/omnia/src/runtime/entry.rs`). The recommended end state below is
that shape, so `emery <args>` keeps working with no key at all.

## Recommended end state

```rust
omnia::runtime!({
    mode: command,
    config: concat!(env!("CARGO_MANIFEST_DIR"), "/omnia.toml"),
    hosts: { /* unchanged */ },
});
```

with `omnia.toml` carrying the guests (emery named first, so the telemetry
name and command-mode `argv[0]` stay `emery`), the per-guest `link`, and the
`[[route.http]]` MCP prefixes. Because the manifest is compiled in, this
command-mode binary is a direct command: invokers keep `emery <args>`
untouched (with `--debug`/`--quiet` reserved as host log flags). Migration
checklist:

- Declare emery and every adapter it links to as static guests; verify
exactly one exports `wasi:cli/run`.
- Move `emery:adapter/source@0.1.0` to emery's per-guest `link`.
- Encode the MCP path mapping as `[[route.http]]` prefixes.

The result is a plain, static omnia CLI runtime with no launcher coupling.
