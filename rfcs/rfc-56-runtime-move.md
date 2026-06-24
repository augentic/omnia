# RFC-56: The Runtime Move — the generic Omnia binary and the multi-guest registry

> Status: Draft · Order 8 of 10 · Stage S4 · Depends: [RFC-52](rfc-52-effect.md), [RFC-54](rfc-54-orchestration.md) · Binds: [RFC-55](rfc-55-working-tree.md) · Enables: [RFC-57](rfc-57-specify-guests.md) · Owns: the runtime and guest selection

## Abstract

This is the keystone: the move that makes "Specify is Omnia compiled with Specify-specific backends" literally true. The generic `omnia <guest>.wasm <args…>` binary replaces the bespoke `specify` host. It instantiates guests per call, satisfies their typed effects ([RFC-52](rfc-52-effect.md)) from bound backends — `wasi:filesystem` (the working-tree backend, [RFC-55](rfc-55-working-tree.md)), `wasi:keyvalue`, lifecycle, and the `wasi-model` host ([RFC-53](rfc-53-wasi-model.md)) with configured model backends ([RFC-58](rfc-58-model-backends.md)) — and holds **many guests at once** in a registry, selecting among them by identity through host-mediated dynamic linking.

## The model

- **Four instantiation triggers.** A guest instance serves one trigger then is discarded: an HTTP request, a topic message (NATS / Kafka), a WebSocket call, or a **CLI command** (`omnia <guest>.wasm <args…>`).
- **Instance-per-call.** Every trigger and every host->guest callback gets a fresh instance on a new `Store`. Guests hold no state between calls — it lives in host services — which is what makes the runtime horizontally scalable; the per-callback fresh instance also prevents *recursive* reentrance (reentering an instance already on the stack), the one kind of reentrance the component model still traps. *Sibling* reentrance is allowed under component-model async, so instance-per-call is a statelessness / isolation choice, not a reentrancy workaround.
- **The multi-guest registry.** One `wasmtime::Engine` and one `Linker` provide every host interface once. A registry maps guest identity -> a pre-instantiated component (`InstancePre`): `workflow`, `source:<id>`, `target:<id>`. Each call selects an `InstancePre` by identity, instantiates fresh, calls the typed export, and discards it.
- **Host-mediated dynamic linking.** A caller reaches an adapter through the host-satisfied per-axis imports (`source` / `target`), naming a plan-bound `adapter-id` as each call's first argument (`build(id, …)`, `survey(id)`, …); the host looks the identity up in the registry, instantiates a fresh instance, carries the typed records to it over wRPC, invokes the adapter's matching `source` / `target` export, and returns the typed result. **Identity is data at the call site** — a plan binding the caller carries (a slice's bound source / target), or, for the `eval` `resolve` callback, the adapter whose brief is being evaluated (fixed for that `eval`). Because the id is a call argument, one caller instance fans out across many same-axis adapters in a loop (e.g. `survey` over every bound source) without re-instantiation — the deterministic for-each stays in guest code. There is no ahead-of-time composition; two same-world adapters are distinct registry entries, so they cannot collide.
- **Inbound trigger routing.** The CLI trigger names its guest directly (`omnia <guest>.wasm`); the other three triggers carry no `adapter-id`, so the host derives the identity from the request and resolves it against the same registry. For HTTP the starting point is a **declarative route table keyed by path prefix** — the [Spin](https://spinframework.dev/v4/http-trigger) `spin.toml` model, longest-prefix wins — projecting a prefix onto a registry key (`/target/omnia/…` → `target:omnia`). Only guests that **export** `wasi:http/incoming-handler` are routable: the host instantiates the matched entry fresh and invokes `handle`; a guest without that export is reachable only through the CLI trigger and host-mediated dynamic linking. Because Specify owns the `wasi:http` host, the table is the floor, not the ceiling — the host may route **programmatically** instead, computing the identity from path / host / headers (mapping in `wasi:keyvalue`) the way Cloudflare's [Workers for Platforms](https://developers.cloudflare.com/cloudflare-for-platforms/workers-for-platforms/configuration/dynamic-dispatch/) dispatch worker selects a script by name. Either way the dispatch is unchanged: select an `InstancePre` by identity, instantiate on a fresh `Store`, invoke the typed export, discard.
- **Guest acquisition.** Core guests (the workflow) embed in the binary (`include_bytes!`) for offline, zero-skew startup; adapters resolve lazily by digest from an OCI store into a local cache — only the identities a plan binds are instantiated.

**Transport is pluggable behind the seam.** The per-axis imports (`source` / `target`) are a typed contract, not a wire protocol — every host-mediated call rides [wRPC](https://github.com/bytecodealliance/wrpc), a WIT-native, transport-agnostic RPC backend that owns the encode -> request -> await -> decode round-trip (including async `stream` / `future` values). The deployment binds the transport: an in-process or Unix-domain-socket transport co-located on one node, NATS or QUIC across a cluster — so moving desktop to cloud is a transport swap, not a code change. Plain records cross by value, but a live resource (the `working-tree` `descriptor`) never does, so `build` / `merge` always ship the content-addressed `revision` / `changeset` and the serving node re-materializes its own tree ([RFC-55](rfc-55-working-tree.md)) — uniformly, local or remote. wRPC is a pinned, swappable backend dependency that never enters the `augentic:specify` contract, so the guest's view stays purely typed and the seam keeps a native in-process fast-path available if profiling ever demands it.

## The component mandate

Standing the guests on the generic runtime is the point at which shipping a WASM component stops being optional: the runtime instantiates a component per call, so an adapter with no component cannot be a guest. Both axes ship a component implementing their world (and the `references` shelf), including the agent-only source adapters (`intent`, `documentation`, `typescript`, `screenshots`, `captures`), whose `survey` / `extract` may still be satisfied through `eval` even though the world exports the interface.

## The cross-repo boundary

- **Omnia** provides the generic floor: the Wasmtime interpreter, the pluggable host-service framework, and the general-purpose host interfaces — `wasi:filesystem`, `wasi:keyvalue`, `wasi:blobstore`, `wasi-model` (`eval`). It carries zero Specify domain and zero model knowledge.
- **Specify** provides the backends (working tree [RFC-55](rfc-55-working-tree.md), model [RFC-58](rfc-58-model-backends.md), kv / lifecycle, the wRPC guest-to-guest transport), the guests, the registry, and the operator CLI.

## Scope

- The generic `omnia <guest>.wasm <args…>` surface and the CLI trigger.
- Instance-per-call execution and the multi-guest `InstancePre` registry keyed by identity.
- Host-mediated dynamic linking through the per-axis `source` / `target` imports — per-call `adapter-id` (from plan / session context) resolved against the registry.
- Binding real backends for the deterministic effects and the `wasi-model` host.
- The component-on-both-axes mandate; retiring the bespoke `specify` host.

## Acceptance criteria

1. Workflow and adapter guests run via `omnia <guest>.wasm <args…>`; the bespoke `specify` host is gone.
2. Multiple guests are co-resident on one Engine + Linker, selected by identity; two same-world adapters resolve without collision and without composition.
3. The deterministic effects and `wasi-model` are satisfied by real backends; execution is instance-per-call with no durable in-guest state.
4. The runtime floor holds zero adapter names, zero workflow knowledge, and zero model knowledge.
5. Every source and target adapter ships a WASM component implementing its world; there are no prose-only adapters.
6. `make lint` and `cargo make ci` stay green.

## Risks and invariants

- **Cross-repo sequencing.** The Omnia framework capability and the Specify backends land in order; the WIT seams version independently.
- **Statelessness is load-bearing.** Instance-per-call concurrency requires the kv and data backends to be correct; what persists lives in host services.
- **Instance-per-call must stay cheap.** Per-call instantiation is only affordable because the heavy work is amortized: `InstancePre` resolves imports and type-checks ahead of time, and the Wasmtime pooling allocator (with copy-on-write image reuse) pre-allocates instance memories. Without them, fresh-instance-per-call would dominate latency.
- **Law 2 preserved.** Everything Specify-specific lives in backends, guests, and native orchestration — never in the generic floor.
- **wRPC is a pre-1.0 dependency on the hot path.** All guest-to-guest dispatch rides wRPC, which is pre-1.0; because it is the universal carrier (not just the cross-node leg), its churn is confined behind the host-dispatch seam (the `source` / `target` imports) so it never reaches the `augentic:specify` contract or the guests, and the seam keeps a native in-process fast-path in reserve. The decision and its constraints (resources do not cross; ship `revision` / `changeset` and re-materialize on the serving node) are recorded in the engine workspace's `DECISIONS.md`.
- **Toolchain cost.** Components + `wit-bindgen` add a build step for every adapter author, including the agent-only source adapters; this is the principal adoption cost, borne here because the generic runtime is what makes a component mandatory.

