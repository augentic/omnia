# RFC-58: Model Backends Extension

> Status: Draft · Order 9 of 10 · **remaining work only** · Depends: the landed `wasi-model` boundary, [RFC-55](rfc-55-working-tree.md) · Enables: [RFC-18](future/rfc-18-slm.md) · Owns: the rest of the backend catalogue and routing behind `wasi-model`

## Abstract

Two backends already sit behind the one `wasi-model` boundary and are selected by config: **frontier / hosted** (`omnia-genai`, with its in-process tool loop per [RFC-59](rfc-59-working-tree-tools.md)) and the **spawned agent** (`omnia-cursor`), plus the in-tree **replay** backend (`ModelDefault`). This RFC now owns only what is **not yet built**: the per-call **router**, the **local SLM** backend, and the **replay expansion** beyond the minimal seam.

## Remaining backends

- **Router** — selects a backend per call by brief path, difficulty, deployment mode, or an abstract cost / quality hint. It **never** routes on a vendor model id supplied by a guest. This is the one selection mechanism still missing; today a deployment binds a single backend in `runtime!`.
- **Local SLM** — narrow, high-volume transformations via a local model and constrained decoding, carried by [RFC-18](future/rfc-18-slm.md). It is a further in-process-loop variant behind the same `WasiModelCtx` trait.
- **Replay expansion** — the minimal replay seam (a directory of canonical-JSON-keyed fixtures) is live. This RFC expands it into a production backend: content-addressed `sha256` keying, fixture management, matching policy, and cross-backend diagnostics.



## Scope

- Router decision keys and deployment-mode selection.
- Local SLM integration via [RFC-18](future/rfc-18-slm.md), including the constrained-decoding hook that keeps typed reports schema-valid.
- Replay fixture management beyond the minimal seam (matching policy, diagnostics, `stream-json` transcript capture).



## Out of scope

- The `complete` host boundary and backend trait — landed in `crates/wasi-model`.
- The genai (`omnia-genai`) and spawned-agent (`omnia-cursor`) backends — landed in the `backends` repo.
- The genai in-process tool loop's remaining `read` / `list` / `write` work; see [RFC-59](rfc-59-working-tree-tools.md).
- Verify profile definitions; see [RFC-60](rfc-60-verify-profiles.md).



## Open questions

- The routing key: brief path, difficulty, deployment mode, or a combination.
- The constrained-decoding hook a non-agent SLM backend uses to keep typed reports schema-valid ([RFC-18](future/rfc-18-slm.md)).
- The matching policy for replay expansion (exact hash vs. tolerant matching) and its diagnostics across backend families.



## Acceptance criteria

1. The router keys on abstract operation information, never a vendor model id exposed to guests, and selects among the bound backends per call.
2. A local SLM backend runs a narrow transformation behind the same boundary with schema-valid output.
3. CI replays through the expanded replay backend with content-addressed keying and useful diagnostics on a miss.
4. Every backend's run remains recordable and replayable through the `wasi-model` boundary.
5. `make lint` and `cargo make ci` stay green.



## Risks and invariants

- **Vendor coupling stays behind the boundary.** Any one model is one backend detail, never part of the contract or runtime floor.
- **Router stays abstract.** Its key is difficulty, deployment mode, or operation identity, not a vendor id.
- **The embedded topology is a non-goal.** Judgment never runs inside the operator's live editor session.
