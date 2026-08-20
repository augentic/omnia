# Removing keys from the `runtime!` invocation

## Context

The emery runtime currently declares:

```rust
program: "emery"
command_guest: "emery"
link: ["emery:adapter/source@0.1.0"]
resolver: launcher::resolver()
http_paths: launcher::mcp_route
http_listener: launcher::http_listener()
```

Each key maps to one `MainOptions` call in the macro-generated `main`
(`crates/host-macros/src/runtime/codegen.rs`); omitting a key omits that
call, so removal means falling back to the runtime's default behavior.

## Resolution strategy

The six keys exist to serve one deployment style: a launcher process that
wires everything at runtime. The elegant removal is not six point fixes but
one move — express the deployment statically in the manifest (inline
`guests:` / per-guest `link` / `routes:` keys, or an `omnia.toml` via
`config:`) and let the runtime defaults carry the rest. Most keys then need
no replacement at all; the sections below give the per-key fallback and the
recommended resolution.

## Key by key



### `program: "emery"`

**Fallback.** The host CLI grammar returns: `emery run [wasm] [--config] -- args…` replaces raw argv passthrough. `--config`/`OMNIA_CONFIG`/positional
wasm can override the compiled-in manifest. `--debug`/`--quiet` peeling and
the `"emery"` telemetry name / `argv[0]` are gone. Requires the `cli`
feature (a default feature, so no Cargo change).

**Recommendation.** Migrate invokers to `emery run -- <args>`; if the old
surface must survive a transition window, a one-line wrapper
(`exec emery run -- "$@"`) is cheaper than keeping the key. The telemetry
name and command-mode `argv[0]` need no replacement: `DeploymentBuilder`
defaults `program_name` to the manifest name — the first `[[guest]]` id
(`crates/omnia/src/deployment.rs`) — so naming the static guest entry
`"emery"` preserves both for free. Logging reverts to the standard
env-driven configuration; document the env var for invokers that used
`--debug`/`--quiet`. The one genuine loss is that the compiled-in manifest
becomes overridable at the CLI — a feature in development, and acceptable in
production wherever the binary is not the trust boundary.

### `command_guest: "emery"`

**Fallback.** Command mode routes to the *sole static* `wasi:cli/run`
exporter. Zero static exporters: inert, exits `0`. Two or more: ambiguous,
run fails. A resolver-supplied command guest no longer works.

**Recommendation.** Lean on the structural invariant instead of the name.
With the guest set static, emery is the only command-capable guest — the
adapter guests export `emery:adapter/source`, not `wasi:cli/run` — so the
sole-exporter catch-all lands on emery with no configuration, and any future
violation fails fast at boot rather than misrouting. Nothing to do beyond
keeping exactly one `wasi:cli/run` exporter in the manifest; if adapters
could plausibly grow that export, guard the invariant with a seam test.
(Explicit command routing returns via `[[route.cli]]` once the runtime
parses it; today that table is deliberately empty.)

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

### `resolver: launcher::resolver()`

**Fallback.** The deployment is static: no resolve-on-miss. Every guest must
be declared up front; any identity nothing statically registers (command
guest, `http_paths` target, link target) fails or 404s.

**Recommendation.** Enumerate the resolve set — emery plus every adapter it
resolves today — as static `guests:`/`[[guest]]` entries. This is strictly
clearer where the set is closed: boot fails fast on a missing artifact
instead of surfacing a miss at first use. Per-environment variation moves to
per-environment `omnia.toml` files selected with `--config`/`OMNIA_CONFIG`
rather than resolve-on-miss logic. The resolver is only load-bearing if the
target set is genuinely open-ended at runtime (identities not knowable at
deploy time) — in that case it should stay, and this removal exercise stops
here.

### `http_paths: launcher::mcp_route`

**Fallback.** HTTP routing reverts from `TableOnly` to `CapabilityDefault`:
the MCP path mapping disappears; with no static routes a sole HTTP exporter
becomes the catch-all for *every* path (including ones the hook 404'd); two
or more exporters with no routes fail as ambiguous.

**Recommendation.** Express the mapping as static HTTP routes:

```toml
[[route.http]]
prefix = "/mcp"
guest = "emery"
```

The catch-all regression does not materialise: once any route is
configured, the trigger is fully route-driven even under
`CapabilityDefault` — an unmatched path is a miss (404) and a sole exporter
never becomes the catch-all (`Router::build` in
`crates/omnia/src/registry/routing.rs`). A static prefix table therefore
reproduces both the mapping and the hook's 404 discipline. The caveat: this
works exactly when `mcp_route` is a fixed path→guest table. A hook that
derives identities dynamically from the path only made sense alongside the
resolver, and goes with it.

### `http_listener: launcher::http_listener()`

**Fallback.** The HTTP trigger binds from the `HTTP_ADDR` env var at boot
(default `0.0.0.0:8080`) instead of adopting the pre-bound socket; guests
see the environment's `HTTP_ADDR`, not the adopted listener's actual
address.

**Recommendation.** Invert the handshake: instead of binding a socket and
handing it in, the launcher (or any invoker) sets `HTTP_ADDR` in the child
environment before exec. With a fixed per-environment port the env value and
the actual bind address coincide, so guests see the right address. Avoid
port `0`: env-driven binding has a pick-then-bind race that socket adoption
existed to close. Race-free ephemeral-port adoption is the one capability
only this key provides — if the launcher genuinely needs it (parallel test
harnesses, port brokering), keep this key and remove the other five.

## Cross-key constraint

`program:` requires a compiled-in manifest (`config:` or inline keys) **or**
`resolver:` + `command_guest:` (macro validation in
`crates/host-macros/src/runtime/parse.rs`). Removing all six together always
parses, and the recommended end state below dissolves the constraint
entirely: no `program:`, and the deployment is the compiled-in manifest.

## Recommended end state

```rust
omnia::runtime!({
    mode: command,
    config: concat!(env!("CARGO_MANIFEST_DIR"), "/omnia.toml"),
    hosts: { /* unchanged */ },
});
```

with `omnia.toml` carrying the guests (emery named first, so the telemetry
name stays `emery`), the per-guest `link`, and the `[[route.http]]` MCP
prefixes. Migration checklist:

- Declare emery and every adapter it links to as static guests; verify
exactly one exports `wasi:cli/run`.
- Move `emery:adapter/source@0.1.0` to emery's per-guest `link`.
- Encode the MCP path mapping as `[[route.http]]` prefixes.
- Set `HTTP_ADDR` in the runtime's environment; drop the socket handoff.
- Migrate all invokers from `emery <args>` to `emery run -- <args>`.

The result is a plain, static omnia CLI runtime with no launcher coupling.