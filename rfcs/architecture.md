# Specify on Omnia — The Effect-Oriented Architecture

> Status: This is the standing architecture — the agreed direction being worked toward. It sequences the work into independently valuable stages and lives alongside [roadmap.md](roadmap.md).

## The core idea

The architecture boils down to a single concept:

> The "Specify" CLI is Omnia compiled with Specify-specific Wasm guests.

Everything that follows is a consequence of this idea. The runtime hosts Wasm guests and satisfies a fixed vocabulary of typed effects; it holds no domain, workflow, or model knowledge. Specify's behaviour — orchestrating the workflow, extracting from sources, building for targets, and development tooling — lives in the guests and in the backends bound behind Omnia's host interfaces.

Context comes from artifacts, not conversational history. Each model evaluation is self-contained: a guest hands the model one **whole brief** and a typed tool surface scoped to concrete artifacts (like `spec.md` or a build request), never an accumulated chat transcript. This avoids the overloaded context windows that cause failures.

This approach provides three major benefits:

1. **Cloud-native portability**: Specify scales from a desktop CLI to a cloud service. Because the execution environment is abstracted behind Omnia's host interfaces, moving to the cloud swaps backends, not guest code.
2. **Scalability and auditability**: An operation runs identically whether triggered from an editor or a CI pipeline.
3. **Cost efficiency**: Because model evaluation is a typed call over specific inputs, a task can be routed to a frontier LLM, a small local model, or deterministic replay by swapping the model backend.



## The shape of the system

The shape of the system

The system is two roles communicating over one contract: a generic runtime and the guests that run on it.

- **Omnia is the foundation**: a command-line executable that instantiates a guest and satisfies its typed effect imports from the backends bound for this deployment. It knows nothing about adapters, workflows, or models.
- **Everything else is a guest**: the workflow (`plan`, `execute`), the adapters (source and target), and the development tooling all run as peers on the runtime.
- **Capabilities are host interfaces**: a guest reaches the outside world only by importing a host interface (`wasi:filesystem`, `wasi:keyvalue`, `wasi-model`, …) and calling it. Each interface is satisfied by a swappable backend.
- **The boundary is typed**: only typed records and handles cross it — never untyped text.

### Guest instantiation

A guest instance is created to serve exactly one trigger, then discarded. There are four triggers:

- an **HTTP request**,
- a **message on a topic** (NATS, Kafka),
- a **WebSocket call**, or
- a **CLI command** (`omnia <guest>.wasm <args…>`).

Guests hold no state between calls, so every trigger gets a **fresh instance**; the same holds for every host→guest callback. This is first a statelessness and isolation choice — it is what makes the runtime horizontally scalable and free of a whole class of aliasing complexity. It also sidesteps the one kind of reentrance the component model still traps: *recursive* reentrance, re-entering an instance already on the stack. (*Sibling* reentrance — a fresh task into a component whose other tasks are suspended — is business-as-usual under the component model's async ABI.)

### Calls in both directions

The runtime and guests interact both ways:

- **Guest → host**: the guest imports a host interface and calls it whenever it needs something impure — read a file (`wasi:filesystem`), cache a value (`wasi:keyvalue`), record a lifecycle event (`journal`), or evaluate a prompt (`wasi-model`).
- **Host → guest**: while servicing a guest call, a host may need content only a guest can produce. It instantiates a fresh guest and calls one of its exported functions. The `wasi-model` host does exactly this when it resolves a brief's references (below).

> **A quick note on naming:** "Omnia" refers to two things — the runtime itself, and the `omnia` target guest (the adapter that generates code for the Omnia runtime). This document is explicit when the context isn't obvious.

## The runtime: Omnia

Omnia is built on Wasmtime. Its design centers on pluggable host services behind typed interfaces, so a backend can be swapped without changing guest code. Three properties make this architecture possible:

- **One binary, guest-selected behaviour**: `omnia <guest>.wasm <args…>` runs, and the guest decides what to do. There is no bespoke `specify` host.
- **Instance-per-call execution**: a fresh instance spins up every time a guest is called, so a host→guest callback can never *recursively* re-enter an instance already on the stack — the one kind of reentrance the component model still traps (*sibling* reentrance, into a component whose other tasks are suspended, is allowed under the async ABI) — avoiding a class of aliasing complexity by construction.
- **Stateless guests, host-held state**: guests cannot hold state in memory between calls. Persistent data lives in a host service — filesystem-backed locally, or Redis / S3 in the cloud. This decoupling is what lets Specify move from a desktop tool to a horizontally scalable service unchanged.

Specify extends this surface in exactly one sanctioned way: **custom backends behind Omnia's host interfaces** — a git-aware `wasi:filesystem` backend that materializes the [working tree](#the-working-tree), and the **model backend** behind `wasi-model`. The model id and any vendor SDK live in that backend, never in the runtime floor.

## Judgment: the `wasi-model` host

Model evaluation is a host capability like any other. Omnia exposes a `wasi-model` host whose `complete` export a guest calls to have a prompt completed (see [wasi-model.md](wasi-model.md) for the full `augentic:model/completion` WIT):

```wit
// wasi-model host — judgment as a typed effect a guest imports
complete: func(prompt: prompt) -> result<answer, error>;
```

Behind the host sits a **swappable model backend**. The backend runs an LLM tool-use loop: it drives a model through its API, advertises a typed tool surface, dispatches the model's tool calls, runs the verify-repair cycle, and returns a validated, typed answer to the calling guest. The guest treats `complete` exactly like `wasi:keyvalue.get` — a typed call whose backend it never sees.

### Resolving references — the host calls back into a guest

A brief points at internal references (e.g. `../references/business-logic.md`). The model emits a `resolve` tool call for each; the backend follows it by selecting the relevant **adapter guest**, instantiating it, and calling its exported reference shelf:

```wit
// adapter reference shelf — the model backend calls this back into the guest
resolve: func(id: adapter-id, reference: reference) -> result<list<u8>, error>;
```

Because recursively re-entering a live instance would trap, this resolution lands in a **fresh adapter instance** every time — isolated from whatever guest called `complete`. The adapter's prose (briefs and references) is **embedded in its module at build time**, so `resolve` is an in-module lookup, not a host filesystem read. The shelf is the adapter's, not the runtime's: a *computed* reference is served by a fresh instance, and the runtime floor stays free of any reference-injection machinery.

Logical sequence: extract

The model reads and mutates a working tree through the same tool surface — `read` / `list` to scan existing code, `write` to accumulate an edit, `verify` to check itself — so it never holds a descriptor or an OS path. A filesystem-capable spawned-agent backend instead reads and writes the working tree directly through the `local-path` it is lent.

### The model backend is swappable

Swapping the model backend is how deployment modes are chosen; Omnia core never learns which model is bound:

- **Frontier / hosted** — a hosted inference API (via `[genai](https://github.com/jeremychone/rust-genai)`) for hard synthesis and review.
- **Spawned agent** — a fresh, context-free agent session for the filesystem-capable path.
- **Small local model** — a local SLM for narrow, high-volume transformations, with constrained decoding for valid typed reports.
- **Replay** — serves recorded `(prompt + tool transcript) → answer` fixtures, turning model evaluations into deterministic regression tests in CI.

Record/replay is a property of the backend boundary: a recording backend logs request→response around the live model; the replay backend serves them.

## Guest-to-guest interaction: host-mediated dynamic linking

A single operation spans several guests: the workflow guest plus the source and target adapter guests it drives. Guests reach each other through **host-mediated dynamic linking** — never by composing them into one module ahead of time.

- **How it works**: the caller imports the per-axis host interfaces (`source` / `target`) and names a plan-bound `adapter-id` as the first argument of each call (`build(id, …)`, `survey(id)`, …) — the very interfaces the adapters export, so there is no separate dispatch facade to keep in sync. The Omnia host intercepts these imports through the Wasmtime `Linker` and issues a wRPC invocation to the named adapter's matching export (`augentic:specify/source` / `target`) over the bound transport.
- **The host's role**: the host selects the adapter **by identity**, instantiates a fresh, stateless instance, carries the typed WIT records to it over wRPC, invokes the exported function, and returns the typed result.
- **Why it fits**: it preserves strict WIT typing with no manual byte serialization, supports dynamic (config-driven, OCI-resolved) adapter selection, and enforces instance-per-call — so a dispatched call cannot recursively re-enter its caller. The `wasi-model` `complete → resolve` callback is this same mechanism applied by the model backend.

Because the interfaces (`target` / `source` / `references`) are statically known and only the adapter *instances* are dynamic, the host serves them with `wit-bindgen-wrpc`**-generated typed bindings** rather than wRPC's dynamic value-introspection path; the dynamic path remains available if an interface is ever unknown at host-compile time.

The seam is a contract, not a wire protocol: every selected call rides [wRPC](https://github.com/bytecodealliance/wrpc) — a WIT-native, transport-agnostic RPC backend that encodes the typed records (and their async `stream` / `future` values) — over whatever transport the deployment binds: an in-process or Unix-domain-socket transport on a single node, NATS or QUIC across a cluster. Moving from desktop to cloud is therefore a transport swap, not a code change. Plain records (`revision`, `changeset`, `input`, `report`, `lead`, `evidence`) cross by value; a live resource such as the [working tree](#the-working-tree)'s `descriptor` never crosses, so `build` / `merge` always ship the content-addressed `revision` / `changeset` and the serving node re-materializes its own tree ([RFC-55](rfc-55-working-tree.md)) — uniformly, local or remote. wRPC stays behind the backend boundary — pinned and swappable, never in the `augentic:specify` contract — so the guest's view stays purely typed and the seam keeps a native in-process fast-path available if it is ever needed.

### Many guests, selected by identity

The binary holds every guest on **one runtime** and picks among them in native code:

```text
Registry  (one wasmtime::Engine + one Linker<StoreCtx>)
  "workflow"             -> InstancePre     (embedded in the binary via include_bytes!)
  "source:typescript"    -> InstancePre   ┐  resolved by digest from an OCI store into a
  "source:documentation" -> InstancePre   │  local cache — only the identities a plan
  "target:omnia"         -> InstancePre   ┘  binds are instantiated
```

Each call selects an `InstancePre` by identity, instantiates a fresh instance on a new `Store`, calls the typed export, and discards it. **Identity is data, resolved by the host — not topology**: it arrives as an `adapter-id` call argument on the host-satisfied `source` / `target` imports, so one caller instance can drive many same-axis adapters in a loop. Two same-world adapters (two sources, two targets) are distinct registry entries, so there is no collision and no ahead-of-time composition. Which adapter a call targets comes from the operation's context:

- the `wasi-model` callback resolves against the adapter in scope for `prompt.grants.references` — its identity is fixed for the duration of that `complete`;
- a workflow→target call (`build`, `merge`, `guidance`) targets the slice's bound target; a workflow→source call (`survey`, `extract`) targets a bound source. Both bindings come from the plan.

The same select-by-identity resolves an **inbound trigger**, not only a guest-to-guest call:

- A CLI command names its guest directly (`omnia <guest>.wasm`)
- An HTTP request carries no `adapter-id`, so the host derives the identity from the request and looks it up in the registry above. The starting point is a **declarative route table keyed by path prefix** — the model Fermyon [Spin](https://spinframework.dev/v4/http-trigger)'s `spin.toml` popularised, longest-prefix wins — projecting a prefix onto a registry key (`/target/omnia/…` → `target:omnia`). Only guests that **export** `wasi:http/incoming-handler` are routable: the host instantiates the matched entry fresh and invokes its handler, so a guest without that export stays reachable solely through the CLI trigger and host-mediated dynamic linking. Because Specify owns the `wasi:http` host implementation, the static table is the floor, not the ceiling — the host may instead route **programmatically**, computing the identity from the request's path, host, or headers (the mapping held in `wasi:keyvalue`) the way Cloudflare's [Workers for Platforms](https://developers.cloudflare.com/cloudflare-for-platforms/workers-for-platforms/configuration/dynamic-dispatch/) dispatch worker resolves a script by name. Either way the dispatch is the same one every other trigger uses: select an `InstancePre` by identity, instantiate on a fresh `Store`, call the typed export, and discard.

## Lifecycle of an operation

Logical sequence: build

A `build` flows like this:

1. The workflow guest resolves the slice to a base revision and asks the host to materialize the slice's [working tree](#the-working-tree); the slice and its inputs stay pure, node-independent data, while the mutable tree is the one capability.
2. It runs any deterministic setup (a `tool` adapter export, reached by host-mediated dynamic linking).
3. For the judgment leg it calls `wasi-model.complete` with a structured `prompt` (see [wasi-model.md](wasi-model.md)).
4. The model backend drives the model. `resolve` follows the adapter's reference shelf (the adapter's `references` export); `read` / `list` scan existing code through the working tree; `write` accumulates an edit.
5. When the prompt calls for it the model emits `verify(<check>)`; the backend runs that vetted, sandboxed profile and feeds the severity-tiered `report` back; the model repairs and re-verifies.
6. `complete` returns the validated, typed answer to the guest.
7. The report carries only judgment (status and findings); the host extracts the resulting mutations as a content-addressed `change-set` (a `git diff` against the base revision), and the guest requests the lifecycle `transition` effect.

In short: deterministic control lives in guest code, judgment is a typed `complete` call, references load lazily through the shelf, and what crosses out is a typed report plus a content-addressed change-set.

## The working tree

A `build` generates a slice *into a pre-existing project*, reading existing code and conventions and writing changes back in place. Modeling that tree as a bare `project-path` string is the one thing that pins an operation to a single machine, so the contract models it as a host-materialized **working tree** capability instead.

The host materializes the tree from a content-addressed **base revision** (a git commit, in the git backend) onto whichever node runs the operation — a local clone on a desktop, a fresh checkout on a cluster node. The capability exposes two faces:

- a `wasi:filesystem` **descriptor**, for deterministic guest code that reads or validates the tree through capability-scoped handles;
- a host-reported node-local `local-path`, for the one consumer that cannot hold a descriptor: the filesystem-capable **spawned-agent** model backend, which reads and writes through real OS paths. An absent path means no real local tree exists on this node — a clean capability signal that an agent-driven build is unavailable there.

The agent's read-modify-write loop is irreducibly node-local, so it is not abstracted away — it is *quarantined* between two portable boundaries: a host-materialized tree on the way in, and a content-addressed **change-set** (adds, modifies, deletes against the base revision) on the way out. Neither `build` nor `merge` returns the delta; the report carries only judgment, and the host extracts the change-set from the tree. `build` is lent the slice's tree and the caller extracts its delta; `merge` is lent the *baseline* tree and folds a change-set into it in place. What crosses the contract is the change-set, never a shared mount — which is what lets `build` and `merge` run on different nodes. Git provides exactly this content-addressing, so it is the natural first backend, carried as a **custom git-aware** `wasi:filesystem` **backend** (native code, so git stays native and there is no in-guest VCS). The mechanism is specified in [RFC-55](rfc-55-working-tree.md).

## Host services and state

Guests are stateless and instance-per-call, so anything that must outlive a call lives in a host service behind a swappable backend:

- `wasi:filesystem` — inputs, assets, and the project tree; the working tree is a custom git-aware backend.
- `wasi:keyvalue` (`state`) — host-held scratch and memoization (a computed reference, a model session's accumulating edits); filesystem locally, Redis / NATS for fleet-shared state.
- `journal` — the durable lifecycle log and its legal transitions; a JSON store over a filesystem backend.
- `wasi-model` — model evaluation, backed by a frontier API, a spawned agent, a local SLM, or replay.

Because guests interact only with typed interfaces, the deployment topology is dictated entirely by the bound backends. A local CLI wires these to the local filesystem and a model API; a cloud deployment wires the same interfaces to S3, Redis, and a fleet model backend. The guests do not change.

## The four laws

When evaluating a design decision — prose or code, what a function takes, where it runs — keep this principle in mind:

> Run every adapter and workflow as a guest on a runtime that understands only typed effects. Keep structure in deterministic guest code, reach the model through the `wasi-model` host behind a swappable backend, and pass handles instead of raw text across boundaries.

1. **Typed boundaries**: WIT records for data, WIT interfaces for effects. Untyped text is not passed across boundaries.
2. **The runtime floor only knows effects**: Omnia core doesn't know about workflows, adapters, or models — only how to host guests and satisfy typed effects. Which backend satisfies an interface — including which model backs `wasi-model` — is deployment configuration the floor never sees.
3. **Determinism by default, judgment by exception**: control flow lives in deterministic guest code. `wasi-model.complete` returns a typed, validated decision that steers the next deterministic step. Models do not guess control flow.
4. **Laziness is key**: handles (file paths, reference ids) cross boundaries instead of corpora, keeping context windows small and operations scalable.

## Deployment modes

Because each evaluation is self-contained, the bound model backend is chosen per deployment (or per call, by a routing backend):

- **Interactive**: a frontier API or a spawned headless agent, run against concrete artifacts.
- **Headless**: a hosted API or a local SLM at fleet scale, no editor in the loop.
- **CI / testing**: the replay backend serves recorded fixtures, turning evaluations into reliable regression tests.

This enables progressive optimisation: as a transformation becomes well-understood, move it from a frontier model to a local SLM, or to deterministic code, by changing the bound backend — no guest rewrite.

## CLI bootstrapping

Because "Specify is Omnia compiled with Specify-specific backends," there is no separate runtime to download — the binary *is* the runtime, linked with its backends. The runtime acquires its guests two ways:

- **Embedded core**: first-party guests (the workflow) compile into the binary via `include_bytes!`, ensuring offline startup and zero version skew.
- **Config-driven resolution**: adapters and third-party guests resolve dynamically via digest-pinned references from an OCI store into a local cache.

## The incremental path

The architecture is approached in stages. Each is independently valuable and forward-compatible on the same typed contract:

- **S1 · Typed contract** ([RFC-51](rfc-51-adapter-wit.md)) — the versioned `augentic:specify` WIT package: records, per-axis `source` / `target` interfaces, the `references` shelf, and the worlds, with host bindings.
- **S2 · Effect map** ([RFC-52](rfc-52-effect.md)) — the typed effects are named and assigned owners: `wasi:filesystem`, `wasi:keyvalue`, lifecycle, `references`, and `wasi-model`.
- **S2 · Judgment boundary** ([RFC-53](rfc-53-wasi-model.md)) — the `wasi-model` `complete` host, prompt / answer records, backend trait, answer validation, and minimal replay seam.
- **S3 · Vertical operation proof** ([RFC-54](rfc-54-orchestration.md)) — one deterministic `tool` operation through generated bindings and one judgment operation through `wasi-model.complete`, proving the path before the workflow moves.
- **S4 · Working-tree backend** ([RFC-55](rfc-55-working-tree.md)) — the git-aware `wasi:filesystem` backend that materializes a content-addressed tree per operation, so `build` and `merge` can run on different nodes and out of order.
- **S4 · Verify profiles** ([RFC-60](rfc-60-verify-profiles.md)) — closed verification profiles, sandboxing, report mapping, and capability signaling for nodes that cannot verify.
- **S4 · Runtime move** ([RFC-56](rfc-56-runtime-move.md)) — the generic Omnia binary plus Specify backends, instance-per-call, the multi-guest registry, the CLI trigger, and the mandatory component on both axes; the working-tree backend is bound behind `wasi:filesystem`.
- **S4 · Workflow (and development) as guests** ([RFC-57](rfc-57-specify-guests.md)) — the workflow and the development tooling run on the runtime like every adapter.
- **Parallel · Model backends** ([RFC-58](rfc-58-model-backends.md)) — the frontier (genai, including its in-process tool loop per [RFC-59](rfc-59-model-tool-loop.md)), spawned-agent, SLM, replay expansion, and routing backends behind `wasi-model`.

## Key trade-offs

- **Omnia is the sole runtime**: the *same* binary and guests run from desktop to cloud with only backends swapping (filesystem → S3, kv → Redis, model → fleet) — at the cost of a hard dependency on Omnia's host surface.
- **Model evaluation needs egress (or a local model) at** `complete` **time**: the binary carries a model-backend dependency; the replay backend covers CI, and a local SLM backend covers air-gapped runs.

