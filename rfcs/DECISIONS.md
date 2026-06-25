# DECISIONS

The shared, durable log of settled design choices the RFCs reference. Each entry
is a decision that is *closed* — plans and code rely on it and do not re-litigate
it. New phases append their settled choices here as they land.

## Host-mediated dynamic linking (`guest-registry.md`)

- **Resources never cross the link seam.** Plain records cross by value; a live
  resource handle is rejected (`dispatch.rs::contains_resource`). §4.5.
- **wRPC is the carrier on every leg**, pinned to a single reviewed git rev
  (`wrpc-transport` / `wrpc-wasmtime`) until the wasmtime-46 line is on crates.io.
- **The floor stays generic (Law 2).** `link` and the `GuestSelector` operate on
  opaque interface strings and opaque `GuestId`s; Omnia never parses a consumer
  scheme.
- **Per-guest `link` allow-list, unioned at startup.** The shared linker wires
  each named host-mediated interface once across all guests.
- **Instance-per-call.** A dispatched call always lands in a freshly-instantiated
  target on a new store, so it can never recursively re-enter its caller. The
  dispatch-depth bound (`DispatchHandle::enter` / `DepthGuard`) is a process-wide
  safety bound on synchronous nesting.

## `wasi-model` boundary + backends (`wasi-model.md`)

### Layer 1 (landed)

- **Mechanism/judgment split.** Omnia owns the `complete` boundary, the
  prompt/answer/error envelope, the `WasiModelCtx` backend trait, the `ToolHost`
  host callbacks, validation in the `complete` binding, and the composable
  record/replay `WasiModelCtx` wrappers. The backend owns the model id, the
  provider SDK or spawned process, message assembly, and the loop.
- **JSON-Schema-over-strings at the floor.** The generic envelope validates the
  JSON `answer` per `response-format.kind`. Phase 1 implements the structural
  gates (`json-object`, `text`) with `serde_json` only; the `json-schema` gate is
  parse-only behind a `TODO` (the validator crate is a Phase 3 follow-up).
- **`grants.working-tree` is a typed `borrow<descriptor>`, never a raw handle.**
  The owned `Prompt` reduces it to a stable `working_tree_lent` boolean marker for
  replay keying; the descriptor is resolved against the table by the floor.
- **`complete-stream` binding is deferred to Phase 3.** The WIT is final in 0.1.0
  and the binding is generated (confirming `bindgen!` compiles the native
  `stream<>` type), but the host impl returns `error::backend("streaming
  unsupported")` until Phase 3.
- **Replay key = canonical JSON of the reduced prompt.** Drop `metadata`
  (tracing only); the working-tree handle is already the `working_tree_lent`
  boolean. Everything that shapes the answer stays in the key. Phase 1 keys by
  canonical-string equality (no new dependency); the content-addressed `sha256`
  form is a Phase 3 follow-up. Recorder and replayer share `replay.rs` so they
  cannot drift.

### Phase 2a (this work) — the genai backend + host→guest `resolve`

- **`resolve` reuses the landed dispatch machinery via a new public host→guest
  entry point — not a parallel mechanism.** `omnia::dispatch_to_guest` (in
  `dispatch.rs`) reuses the same `enter`/`DepthGuard` depth bound and
  `contains_resource` resource rejection as guest→guest dispatch, so a
  `complete`→`resolve`→adapter chain is depth-counted exactly like a guest→guest
  hop.
- **The host→guest leg instantiates the target fresh and calls its export
  directly** (the `wasi-http` `server.rs` pattern), rather than round-tripping
  the wRPC carrier. The entry point owns the whole store lifecycle (build →
  instantiate → call → drop), so the target's `references` export needs no
  `link` declaration and the callee is a fresh instance that cannot re-enter its
  caller (instance-per-call). Because `resolve` is invoked from *within* the
  caller guest's concurrent event loop (the backend's loop awaits it inside the
  `complete` host call) and wasmtime forbids a recursive
  `StoreContextMut::run_concurrent` on the same thread, the callee runs on its
  own spawned task: when the caller's loop parks awaiting it, the ambient store
  clears and the callee's call runs unnested.
- **The floor discovers the `resolve` export by convention, not by package name.**
  The entry point finds the exported interface that contains a `resolve` function
  on the target component and invokes it; no consumer package/interface name is
  baked into the floor (Law 2). The `resolve` func name mirrors the `ToolHost`
  method and the RFC tool name, both floor concepts.
- **The dispatcher is threaded into the store context as `Arc<dyn
  HostDispatch>`** (a blanket `impl<R: Runtime> HostDispatch for R`), injected by
  the `runtime!` macro exactly like the per-store `WrpcState`. It is inert unless
  a host uses it; `wasi-model` reaches it through `WasiModelCtxView`.
- **`verify` is routing-only.** The floor checks the requested check against
  `grants.verify` and acknowledges routing; profile definitions, sandboxing, and
  execution are owned by RFC-60 and are not implemented here.
- **`read` / `list` / `write` stay loud stubs in Phase 2a.** They consume the
  RFC-55 wasi-filesystem working tree, which is not yet built; they (and the
  `wasi:keyvalue` cross-turn session state that backs `write`/`read` visibility)
  land in Phase 2b. The genai loop's per-call working state lives in the
  `complete` future for now.
- **Vendor SDK + keys stay below the boundary.** `genai = "0.6"` is a pinned,
  swappable dependency confined to the `omnia-genai` backend crate (backends
  repo). API keys are read in `connect` and never logged or recorded into
  fixtures.
- **Cross-repo dependency.** The backends repo consumes the unreleased omnia
  Phase 2a API via a temporary `[patch.crates-io]` path override during
  development; final merge is gated on a published omnia release that includes
  `dispatch_to_guest` / `HostDispatch` and `omnia-wasi-model`. Because the local
  omnia line (0.35.0) is a minor ahead of the published one the existing
  backends pin (0.34.0), the override is kept surgical: only `omnia-genai`
  requests omnia/`omnia-wasi-model` at `0.35.0` (satisfied solely by the patched
  local path), so the two omnia versions coexist and every other backend keeps
  resolving the published 0.34.0 crates untouched. The patch (and the genai
  `0.35.0` pin) collapse to a normal published dependency at the release gate.
