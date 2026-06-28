# Design: Host-Mediated Dynamic Linking — Remaining Transports & Hardening

> Status: Implementation plan — **remaining work only**. 
>
> The guest registry, inbound
> trigger routing, and host-mediated dynamic linking over the **in-process wRPC
> transport** have landed (`crates/omnia/src/{registry,routing,dispatch,transport,selector}.rs`,
> `examples/linking`, `examples/routing`). This document now tracks only the work that is
> **not yet built**: the additional cluster transports (Phase 3) and hardening (Phase 4).

## 1. What remains

The mechanism is in place: one `Engine` + one `Linker`, a `Registry` of identity →
`InstancePre`, capability-based trigger routing from `omnia.toml`, the `LinkTransport`
seam, the in-process wRPC carrier, the `GuestSelector`, instance-per-call dispatch,
resource-crossing rejection, and the dispatch-depth bound. What is missing is the
"desktop → cloud" half — carrying the *same* dispatch over the wire — and the
production hardening around it.

Two invariants from the original design still bind every change below:

- **wRPC is the carrier on every leg.** New transports are bound-transport swaps behind
the existing `LinkTransport` seam; the dispatch interceptor on the linker does not
change.
- **Resources never cross the seam** (§"Resources"). Plain records cross by value; a
live `descriptor` is rejected with a typed error. Cross-node transports must preserve
this — the serving node re-materialises its own tree from a content-addressed
`revision` / `changeset`.



## 2. Phase 3 — Additional transports

The landed `LinkTransport` trait abstracts a bound wRPC client (and, where the node
serves guests, a server handle). Only the in-process pipe is wired today. Phase 3 adds
the remaining transports and the `Target::Remote` resolution that makes a guest's
location a config decision rather than a code one.

- **Unix-domain socket (next).** `wrpc-transport`'s UDS `Client` / `UnixListener` —
same node, separate processes. This is the natural first proof that the dispatch path
is unchanged across a real transport boundary: bind UDS in `omnia.toml`
(`[transport] default = "unix"`), run `examples/linking` with the two guests in
separate processes, and confirm the echo round-trips with no guest or dispatch-code
change.
- **NATS / QUIC (cluster).** The distributed legs; wRPC ships both. A registry entry
resolves to `Target::Remote(<bound wRPC endpoint>)` instead of `Target::Local`, and
the caller forwards the invocation to that endpoint. Per-target overrides in the
manifest select the transport per identity:

```toml
[transport]
default = "in-process"
[transport.target."target:omnia"]
kind    = "nats"
address = "nats://…"
```

- `Target::Remote` **population.** The registry type already carries the
`Local | Remote` split; the resolver, the forward path, and manifest wiring for
remote endpoints are the work. Inbound routing is untouched — only inter-guest
dispatch gains a remote arm.

**Acceptance:** demonstrate the desktop → cloud transport swap — the same
`examples/linking` (and a representative two-guest deployment) running co-located, then
over UDS, then across two processes/nodes over NATS — driven entirely by `omnia.toml`,
with no guest or dispatch-code change. `cargo make ci` stays green.

## 3. Phase 4 — Hardening

- **Optional native in-process fast-path.** A direct `Instance::get_func` +
`Func::call_async` behind the same `LinkTransport` seam, bypassing wRPC
encode/decode for the co-located case. **Only if profiling demands it** — the
in-process wRPC pipe is the baseline and stays the default.
- **Richer dispatch metrics.** Per-dispatch target identity, latency, and transport
emitted alongside the existing pool/instantiation metrics, so nested instantiation
cost is visible for pool sizing across transports.
- **Fault injection / failure-mode tests.** Transport failures, slow peers, depth-bound
exhaustion, and resource-rejection paths exercised deliberately, especially for the
remote transports where partial failure is new.
- **Docs.** The deployment manifest reference (population, routing, transport), the
transport-swap runbook, and the operator-facing description of the registry.



## 4. References

- [architecture.md](architecture.md) — the standing direction the registry serves.
- [wRPC](https://github.com/bytecodealliance/wrpc) — the carrier. Relevant crates on
`main`: `wrpc-transport` (UDS / NATS / QUIC), `wrpc-wasmtime` (the wasmtime
polyfill/serve integration); pinned to a reviewed git rev until a wasmtime-46 line is
published.

