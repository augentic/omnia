# RFC-58: Model Backends — frontier, spawned agent, SLM, and routing

> Status: Draft · Order 9 of 10 · Parallel after [RFC-53](rfc-53-wasi-model.md) · Depends: [RFC-53](rfc-53-wasi-model.md), [RFC-59](rfc-59-model-tool-loop.md), [RFC-55](rfc-55-working-tree.md) · Enables: [RFC-18](future/rfc-18-slm.md) · Owns: backend variety and routing behind `wasi-model`

## Abstract

The `wasi-model` host ([RFC-53](rfc-53-wasi-model.md)) dispatches `eval` to a backend. This RFC owns the backend catalogue and router: frontier / hosted, spawned-agent, replay expansion, local SLM, and the decision key that selects one per call. It builds on the core replay seam in [RFC-53](rfc-53-wasi-model.md) and the tool-loop semantics in [RFC-59](rfc-59-model-tool-loop.md); it does not redefine either.

## Backend catalogue

The backend is the single seam the model is reached through. The fleet lives inside it, and the model id never crosses `eval`.

- **Frontier / hosted** — hard synthesis and review through a hosted API via [`genai`](https://github.com/jeremychone/rust-genai), one API over OpenAI / Anthropic / Gemini / Ollama / other providers. Switching frontier, hosted, and local providers is backend configuration.
- **Spawned agent** — the native layer spawns a fresh, context-free agent session, hands it the brief, and parses the validated answer. It may own its own tool loop and read / write the working tree through the `local-path` it is lent ([RFC-55](rfc-55-working-tree.md)). It still returns through the [RFC-53](rfc-53-wasi-model.md) typed boundary and must remain recordable.
- **Replay expansion** — [RFC-53](rfc-53-wasi-model.md) defines the minimal replay seam. This RFC expands replay into a production backend with fixture management, matching policy, and diagnostics across backend families.
- **Local SLM** — narrow, high-volume transformations via a local model and constrained decoding, carried by [RFC-18](future/rfc-18-slm.md).
- **Router** — selects a backend per call by brief path, difficulty, deployment mode, or an abstract cost / quality hint. It never routes on a vendor model id supplied by a guest.

## Deployment modes

- **Interactive** — frontier API or spawned agent against concrete artifacts.
- **Headless** — hosted API or local SLM at fleet scale, no editor in the loop.
- **CI / testing** — replay fixtures served through the replay backend.

## Scope

- Frontier / hosted backend configuration.
- Spawned-agent backend protocol and process management.
- Replay fixture management beyond the minimal [RFC-53](rfc-53-wasi-model.md) seam.
- Router decision keys and deployment-mode selection.
- Local SLM integration via [RFC-18](future/rfc-18-slm.md).

## Out of scope

- The `eval` host boundary and backend trait; see [RFC-53](rfc-53-wasi-model.md).
- The standard model tool loop; see [RFC-59](rfc-59-model-tool-loop.md).
- Verify profile definitions; see [RFC-60](rfc-60-verify-profiles.md).

## Open questions

- The routing key: brief path, difficulty, deployment mode, or a combination.
- The spawned-agent protocol: how a session is spawned, handed the brief, returns a schema-valid answer, and consumes the prose shelf.
- The record/replay capture point for spawned-agent runs that own their own loop.
- The constrained-decoding hook a non-agent SLM backend uses to keep typed reports schema-valid ([RFC-18](future/rfc-18-slm.md)).

## Acceptance criteria

1. At least two real backends, such as frontier API and spawned agent, sit behind the one `wasi-model` boundary and are selected by config.
2. Interactive and headless modes both run a real operation.
3. CI replays through the replay backend without a live model.
4. The router keys on abstract operation information, never a vendor model id exposed to guests.
5. Every backend's run is recordable and replayable through the [RFC-53](rfc-53-wasi-model.md) boundary.
6. `make lint` and `cargo make ci` stay green.

## Risks and invariants

- **Vendor coupling stays behind the boundary.** Any one model is one backend detail, never part of the contract or runtime floor.
- **Router stays abstract.** Its key is difficulty, deployment mode, or operation identity, not a vendor id.
- **Spawned process management.** Sessions stay robust and context-free; a leaked transcript reintroduces the dependency the architecture sheds.
- **The embedded topology is a non-goal.** Judgment never runs inside the operator's live editor session.
