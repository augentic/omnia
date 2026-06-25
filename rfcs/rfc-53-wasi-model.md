# RFC-53: The `wasi-model` Host Core

> Status: Draft · Order 3 of 10 · Stage S2 · Depends: [RFC-51](rfc-51-adapter-wit.md), [RFC-52](rfc-52-effect.md) · Enables: [RFC-58](rfc-58-model-backends.md) · Owns: judgment as a host effect

## Abstract

Judgment is a host effect. Omnia exposes a `wasi-model` host whose `eval` export a guest calls to have a prompt evaluated:

```wit
eval: func(prompt: prompt) -> result<answer, error>;
```

This RFC owns the boundary only: prompt / answer records, the backend trait behind `eval`, schema validation, error mapping, and the minimal replay-capable backend needed for deterministic tests. The genai backend's in-process tool loop is [RFC-59](rfc-59-model-tool-loop.md). Closed verify profiles are [RFC-60](rfc-60-verify-profiles.md). Backend variety and routing are [RFC-58](rfc-58-model-backends.md).

## The boundary

A guest treats `eval` like any other typed host effect. It supplies a complete prompt request, including the brief identity, operation type, expected answer shape, and any handles the backend is allowed to expose as tools. The host returns either a validated typed answer or a typed error. No vendor model id, SDK type, transcript, or free-form tool contract crosses the boundary.

Behind `eval` sits a backend trait. The trait is responsible for evaluating the prompt, producing an answer candidate, and returning enough transcript metadata for record/replay. The host wrapper validates the answer against the operation's expected schema before returning it to the guest.

## Minimal replay

Replay belongs at the `wasi-model` boundary because it is the test substitute for judgment itself. The core RFC therefore includes a minimal recording / replay backend:

- The recording backend logs `(prompt request + tool transcript) -> validated answer`.
- The replay backend serves the recorded answer for an equivalent prompt request.
- Replay fixtures are deterministic enough to test one vertical operation without a live model.

[RFC-58](rfc-58-model-backends.md) expands this into the full backend catalogue and router, but it does not invent the replay seam.

## Scope

- The `wasi-model` host interface (`eval`).
- Prompt, answer, and error records.
- The backend trait used by the host.
- Answer validation before returning to the guest.
- Minimal record/replay at the backend boundary.

## Out of scope

- Tool-call dispatch (`resolve`, `read`, `list`, `write`), repair-loop semantics, and session state inside `GenaiBackend`; see [RFC-59](rfc-59-model-tool-loop.md).
- Closed verification profiles, sandboxing, and severity mapping; see [RFC-60](rfc-60-verify-profiles.md).
- Frontier, spawned-agent, SLM, and router backends; see [RFC-58](rfc-58-model-backends.md).

## Acceptance criteria

1. A guest can call `eval` and receive either a validated typed answer or a typed error.
2. The backend trait carries no vendor-specific type above the backend boundary.
3. The host validates answers before returning them to guests.
4. One recorded prompt replays deterministically without a live model.
5. `make lint` and `cargo make ci` stay green.

## Risks and invariants

- **Law 2 at the floor.** The model id and vendor SDK live in the `wasi-model` backend, never in Omnia core or the typed contract.
- **Validated answers only.** A model response that does not validate is not an answer; it is a backend failure. Backends that run a repair loop (genai; [RFC-59](rfc-59-model-tool-loop.md)) consume invalid candidates internally.
- **Replay is boundary-level.** Recording and replay happen around the typed prompt / answer boundary, so CI does not depend on any one backend implementation.
- **No transcript leakage.** The operator's live editor conversation is never reused as a model session.
