# Portable storage and the human seam

Status: **Draft** — working design, executed as the discrete steps in [§7](#7-execution-steps); the generation-set shrink prerequisite has landed. Each step lands independently with the journey test green.

## 1. Motivation

The engine's persistence today is the host filesystem, wired through a static, CWD-rooted deployment policy in `src/main.rs`: the invocation directory mounts writable as `.`, and a CWD-relative `.emery-cache` backs the cache preopen. That policy hard-codes three constraints we want to remove:

- **One project per deployment.** `crates/engine/src/handler/locations.rs` notes explicitly that no project-id keying is needed *because* the cache is CWD-relative. Multi-project, multi-tenant, and serverless deployments are impossible without re-keying storage.
- **A writable `.` mount.** The guest holds write authority over the operator's entire project tree in order to maintain one subtree (`.emery/`). Least-authority (the C3 posture) wants the guest holding exactly the capabilities it uses.
- **Hand-rolled durability.** `crates/artifacts/src/atomic.rs` (temp file, `sync_all`, atomic rename) and the crash-litter pruning in `Home::prune` exist only because the filesystem offers no better primitive.

The move: engine state goes behind `wasi:keyvalue` and `wasi:blobstore` capability imports, with the host binding deciding what backs them (local directory, sqlite, object store, …). Deployment becomes host policy, not engine code.

This is tractable because the output home is already store-shaped: `crates/engine/src/home.rs` writes immutable, digest-named generation sets, swaps one small `current` pointer, and prunes what the pointer no longer names. That is exactly the idiom a keyvalue-pointer-over-immutable-blobs store requires — the port changes the backing, not the semantics.

## 2. Goals and non-goals

Goals:

1. Engine state (generation store, pointer, component cache, global store, locks) reachable only through storage capabilities; no `std::fs` against engine-owned state in guest code.
2. The host binding chooses the backing. The shipped local binary defaults to a durable filesystem store under the invocation directory (generations survive restart the way `.emery/spec/` does today). That is store durability, not an unchanged working tree — `specify` no longer writing `spec.md` into the tree ([step 6](#7-execution-steps)) is an intentional operator-visible change.
3. The human seam — review of `spec.md` / `design.md` — survives as a **verifiable, non-authoritative projection** of the store, via the `specify` envelope and `emery show` ([§4](#4-the-human-seam)). Generation is not repo-anchored (§4 standing fact 3); git enters at delivery, not here.
4. Net deletion where the backing permits it: `atomic.rs`, the litter half of `prune`, the `cache_dir()` mkdir in `main.rs`, the writable `.` mount, and `.emery/project.yaml` ([§5](#5-deleting-projectyaml)) all go.

Non-goals:

- Moving the **workspace lend**. The WIT `workspace` record (`wit/emery.wit`) lends the operator's live source tree read-only to the model; it is inherently a filesystem view and stays one. (A content-addressed workspace snapshot for reproducible extraction is a separate, larger design.)
- Dynamic adapter resolution or any download path.
- A migration framework. Pre-1.0, crossing this boundary is a re-init.
- A human-edited replacement config discovered implicitly (`emery.yaml` at the repo root, etc.). That reverses the rule that the CLI is the only mutation path and recreates the same noun. Distinct from the explicit `--sources sources.toml` argv carrier ([§5.2](#52-the-replacement-authority)), which the engine never writes or discovers.
- `emery diff`, `emery materialize`, and exposing extract receipts or bindings on `show`. The re-mine diff stays on the `specify` envelope (exists). Review is `show spec|design` to stdout. A working-tree copy of the spec is a delivery-loop concern ([layer 3](#layer-3--git-projection)), not a generate-time verb. The generation carries no argv or receipt snapshot to expose: the adapter list is argv / `sources.toml` ([§5.2](#52-the-replacement-authority)); requirement-level provenance stays `Sources:` in `spec.md`. The generation is the two reviewable documents.

## 3. Target architecture

### 3.1 Storage inventory and destination

| Surface | Today | Destination |
| --- | --- | --- |
| Generation documents (`spec.md`, `design.md`) | `.emery/spec/generations/<digest>/` | blobstore container, objects keyed by generation digest |
| `current` pointer | `.emery/spec/current` file | keyvalue entry; swap is CAS, conflict fails closed ([§3.2](#32-authority-model)) |
| Component cache | `.emery-cache/components/<name>.wasm` + `.meta.yaml` sidecar | blobstore container keyed by adapter name; `ComponentMeta` is **deleted**, not ported — write-only provenance whose one reader (`init`'s `bind`) dies in step 5 |
| Global adapter store + `.meta` sidecars | host-side `<store>/<name>@<version>.wasm` + sidecar | blobstore (immutable entries) + keyvalue (verify-on-read digest + OCI provenance) |
| Locks / PID stamps | file stamps via `bytes_write` | keyvalue atomics (`StateStore::cas` / `increment`). No lock stamps on the live surface; the capability is there if they return |
| `project.yaml` | `.emery/project.yaml` | **deleted** — the file is a sticky copy of `init`'s argv, not engine state ([§5](#5-deleting-projectyaml)) |
| Workspace lend | read-only view of the source tree | **stays on disk** (non-goal) |

### 3.2 Authority model

- The **keyvalue pointer is the single authority** for "what is the current generation". A read failure on the pointer is an error, never an empty result — exactly the posture `Home::current` already takes with `spec-home-corrupt`.
- Generation blobs are **immutable and self-verifying**: `SpecSet::id()` is the digest of the documents' bytes, so any copy anywhere can be checked against the pointer. Projections ([§4](#4-the-human-seam)) are therefore never trusted, only verified.
- Multi-document commit needs no transaction: write the immutable blobs first, then CAS the one pointer key (absent → first id, or the outgoing id this run observed → incoming). A CAS conflict is a typed failure — two concurrent `specify` runs do not last-write-wins. Same ordering `Home::commit` already implements over the filesystem; the primitive changes from rename-of-`current` to `StateStore::cas`.

### 3.3 The capability seam

Follow the existing provider pattern (`omnia_guest::Model`, `emery_adapter::Source`; see the bare `Provider` in `src/lib.rs`): one engine-side storage capability trait pair, with wasm32 defaults over the `wasi:keyvalue` / `wasi:blobstore` imports and bare native impls so tests script storage in memory exactly as they script the model and the source seam today.

omnia-guest already ships this exact shape for storage: `omnia_guest::StateStore` and `omnia_guest::BlobStore` are pre-existing capability traits whose wasm32 defaults delegate to the `omnia-wasi-keyvalue` / `omnia-wasi-blobstore` imports. The engine seam consumes or mirrors those traits rather than inventing a new pair.

`StateStore` today is get / set / delete only; [§3.5](#35-omnia-work-this-design-requires) adds the `wasi:keyvalue/atomics` surface the pointer swap needs. Blind `set` is not the pointer primitive.

`emery_engine::home` remains the one module owning spec-set reads/writes; `Locations` stops being path math and becomes key/container-name math. Kernels keep consuming values and never touch the environment.

### 3.4 Host bindings

The `omnia::runtime!` invocation in `src/main.rs` grows `WasiKeyValue` / `WasiBlobstore` host entries beside `WasiHttp` / `WasiModel`, and the `mounts:` table shrinks (cache mount deleted; `.` drops to read-only in step 6, since `specify` stops writing the working tree). The shipped binary's default binding is a durable filesystem store under the invocation directory — `omnia-filesystem` for blobstore, plus the filesystem keyvalue backend this work adds ([§3.5](#35-omnia-work-this-design-requires)) — so a local restart still has `current` (goal 2). Stock `KeyValueDefault` is in-memory and is not the local default. Alternative bindings (project-id-keyed, remote) are deployment profiles, not engine changes.

> [!NOTE]
> `wasi-keyvalue` and `wasi-blobstore` are early-phase WASI proposals — WITs are unstable and stock wasmtime does not ship host implementations — but omnia already mitigates both halves. It vendors its own fork of each WIT (`omnia/crates/wasi-keyvalue/wit`, `omnia/crates/wasi-blobstore/wit`) and ships the host implementations (`WasiKeyValue` / `WasiBlobstore`, in-memory defaults) plus the guest capabilities (§3.3). `omnia-backends` already carries `omnia-filesystem` for wasi-blobstore (durable, network-free) as a one-line host swap; this work adds the matching filesystem keyvalue backend there. Upstream churn lands against the omnia fork as a versioned seam change, like the adapter WIT.

### 3.5 Omnia work this design requires

Two omnia-side changes. They land in `augentic/omnia` / `augentic/omnia-backends`, in parallel with emery steps that do not need them, and they **block emery step 2**.

1. **Filesystem `wasi:keyvalue` backend** in `omnia-backends`: a `keyvalue` module in the existing `omnia-filesystem` crate beside the blobstore, mirroring the shape of the `nats` / `redis` keyvalue modules, so one local-first package covers both worlds. Durable, network-free, buckets as directories, keys as files (reject `..` / absolute / empty segments, as blobstore does). Writes are temp-file + atomic rename. CAS is **native** under a per-key lock — not the host's racy read-modify-write fallback in `atomics_impl.rs`. Increment uses the same lock. Configuration mirrors `BLOBSTORE_ROOT` (`KEYVALUE_ROOT`, plus `Client::open(root)` for deployments that anchor themselves).

2. **`cas` and `increment` on `omnia_guest::StateStore`.** The trait today cannot express the pointer swap or the locks row in §3.1. Add a one-shot `cas(key, expected: Option<&[u8]>, value)` that maps to `wasi:keyvalue/atomics` on wasm32 (`cas.new` + `swap`; `expected: None` is "key absent") and `increment` over `atomics.increment`. Native tests script both the same way they script `get` / `set`. Do not key the pointer into the blobstore as a workaround; `current` stays a keyvalue entry.

## 4. The human seam

The spec is a derived document. The loop this work preserves:

1. **Mine.** `emery specify <adapter>...` or `emery specify --sources sources.toml`. Extract, reconcile, synthesise, commit a generation (immutable blobs, then the `current` pointer).
2. **Review.** The success envelope (generation id, re-mine diff); then `emery show spec|design`. Later the MCP resource and a delivery PR. Pipe `show` to a pager if you want an editor.
3. **Change.** Edit a *source* — intent `--value` or the `value` field in `sources.toml`, the workspace those adapters extract, or the adapter list — and re-run `specify`. There is no load/patch/save of `spec.md`. `show` is stdout; there is no working-tree copy to edit.

Durability is three scopes, not one:

- **Same deployment, next process.** The store. The local binary's filesystem binding keeps `current` across restart the way `.emery/spec/` does today. No file in the working tree does not mean ephemeral.
- **Fresh environment (new machine, CI).** Re-run `specify` from the operator's own invocation (a `sources.toml` passed explicitly, or adapters in the Makefile/skill). The spec is regenerated, not checked out.
- **Spec as a git deliverable.** [Layer 3](#layer-3--git-projection) — the delivery loop copies the specs into a checkout it owns — not generation time.

Three standing facts shrink the rest:

1. **The filesystem is already read-only to humans.** The CLI contract forbids hand-editing anything under `.emery/`; every mutation routes through the CLI. The seam to preserve is *review* — read, never edit of the generated documents. Mutation is the loop above: sources in, `specify` again.
2. **Every generation is self-verifying** (§3.2), so we can hand out any number of non-authoritative views without forking authority.
3. **Generation is not repo-anchored.** When code generation returns (the `v1`-tagged loop), delivery creates a temporary checkout, generates code, adds the specs, commits, and raises a PR: the spec reaches a repo at delivery time, in a checkout the pipeline owns — never at generation time in whatever directory `specify` happened to run. Between generation and delivery, the store is the spec's only home, and the store backing carries the durability weight a git-tracked copy used to.

The seam is delivered in layers over one authority. Layer 1 is part of this work's definition of done; layers 2–3 are follow-on deployment profiles.

**Decision: `init` dies in the `show` PR (step 5), not earlier.** Deleting `init` ([§5](#5-deleting-projectyaml)) is the named deletion for `show`. Net live verbs: `specify` and `show`; `completions` stays auto-derived. That verb swap is one policy change in the same PR. The storage port (steps 1–4) does not need the project record gone — `specify` keeps loading `project.yaml` until the new input authority lands. Deleting `init` earlier would leave a one-verb valley whose only review path is the envelope, and would spend the named deletion before `show` exists.

### Layer 0 — the envelope (exists)

The `specify` envelope is already the reporting channel: the re-mine diff is emitted, never persisted. `show` follows that precedent — rendered from the store, emitted, never a second authority. Superseded generations stay pruned on pointer swap, as today; the envelope computes the diff in memory before prune. On-demand history is a layer-3 concern, not a verb.

### Layer 1 — `emery show`

The CLI grows one read verb over the store:

- `emery show spec|design` — render that document of the current generation to stdout, with the generation id in the envelope.

Those are the reviewable documents — and the whole generation; there is no `show bindings` or `show receipts` (non-goals, [§2](#2-goals-and-non-goals)). `specify` stops writing `spec.md` / `design.md` into the working tree (step 6); review after that is the envelope plus `show`, and standing fact 3 says nothing rides on the generation-time working tree holding a copy.

### Layer 2 — read-only MCP resource

The pre-bound listener already serves MCP reference shelves only, with the typed C3 refusal (`crates/transport/src/http.rs`) rejecting everything else. A read-only resource exposing the current generation and its id fits that posture — reads were never what C3 fences — and serves the growing consumer that is not a human at a shell: IDEs and agents (the `plugins/emery/` skill included).

### Layer 3 — git projection

This is the sketch of the actual delivery path when code generation returns: the loop creates a temporary checkout, generates code, copies the specs beside it, commits, and raises a PR — review becomes code review at delivery. The heavier variant — a blobstore host binding backed by a git object database — additionally yields generation history for free. Deferred; recorded here so the container/key naming in step 3 does not preclude it.

## 5. Deleting `project.yaml`

`.emery/project.yaml` is the spec generator's authored project record: identity, the CLI version pin, and the source bindings `specify` extracts from. Written only by `emery init`; loaded fail-closed by `specify` via `RequestContext`. Humans must not edit it.

It is not a twin of the generation store. The store is output (`spec.md`, `design.md`). `project.yaml` is a **sticky copy of `init`'s argv** — which adapters, under which key, workspace vs `--value`. That list is not re-homed into the generation — the generation is the two documents alone. Later runs take adapters from argv / `sources.toml` ([§5.2](#52-the-replacement-authority)).

**Decision: the file is deleted**, not kept as a disk residue of this work. It dies with `init` in the `show` PR (step 5; [§4](#4-the-human-seam)), not as a storage-port prerequisite. This repo already gitignores `.emery/project.yaml`, so clone-and-specify from git alone is not current practice.

### 5.1 Jobs to re-home or drop

| Outcome | Today | After deletion |
| --- | --- | --- |
| Which adapters to extract, and workspace vs `--value` | `sources:` | Re-homed — this is `specify`'s input ([§5.2](#52-the-replacement-authority)). |
| "Has init run?" | file exists → else `not-initialized` | Follows the chosen authority; dies if `specify` no longer needs a prior step |
| Project version floor | `emery:` pin → exit 3 | **Dropped.** Adapter `emery-floor` already refuses a too-old binary (`AdapterCliTooOld`). A second pin is how the file justified itself as a compatibility document. |
| `init --upgrade` re-ensures without rewriting bindings | reload file, bump pin | Follows wherever bindings live; dies with `init` |
| `name` / `description` | written, never read by extract / reconcile / synthesise | **Dropped.** Unused. |

### 5.2 The replacement authority

One authority replaces the file: `specify`'s argv, optionally carried by an operator-owned `sources.toml`.

**Collapse `init` into `specify`.** `emery specify <adapter>... [--value <adapter>=<text>] | --sources <path>`. Ensure, extract, commit in one verb.

- Persistence of the binding list goes away. Repeat the adapters every run (Makefile, skill, CI) — or point at an operator-owned `sources.toml` (below).
- `not-initialized` dies. There is no project record to be missing.
- `--upgrade` dies. Re-running `specify` re-ensures as a side effect of resolve, the way extract already re-resolves.
- Local `.wasm` mirroring moves to the first `specify` (today that is `ensure` inside `init`).
- Changing sources is a different argv. Intent text (`--value`) is passed every time unless the operator points at a file they own.
- Deleting `init` is the named deletion for `show` ([§4](#4-the-human-seam)); both land in step 5.

This is the deletion-aligned shape: one generate verb, no project-config noun, no sticky file. Its stated cost — the adapter list is no longer remembered inside the tree — is removed by the file carrier below.

**The file carrier — `sources.toml` via `--sources <path>`.** The operator may write the binding list in a file they own and pass it explicitly: `emery specify --sources sources.toml`. The precedent is v1's hand-authored definition home (archived at the `v1` tag): an operator-authored input file, loaded fail-closed with typed errors, never engine-written. Rules:

- **Explicit `--sources <path>` only at this stage.** No implicit `./sources.toml` discovery — discovery is what turns a file into a config noun; the explicit flag keeps it an argv macro. Loosening to a default lookup would be a deliberate later decision.
- **The engine never writes it.** Mutation authority stays two-pole: engine state mutates only through the CLI; `sources.toml` mutates only in the operator's editor. The file is never read back between runs except when passed again.
- **Mixing refuses typed.** Positional adapters (or `--value`) together with `--sources` is `Error::Argument` (exit 2). Merge/override semantics can be added later if a need shows up.
- **TOML is deliberate.** Human-centric configuration files shift gradually to TOML — the idiomatic choice for Rust codebases — while engine-written artifacts stay YAML. Cost acknowledged: one new parse dependency beside `serde-saphyr`.
- **Cargo's dependency-table convention names the actual source.** Each entry is a named table (`[sources.<key>]`) whose key becomes the seam's binding key (`input.key` in `wit/emery.wit`) — so one adapter can bind more than one root, which the resolved-adapter-name key cannot express. Exactly one location key per entry — `path`, `git`, `url`, or `value`; omitted means the workspace lend at `.`, the analog of Cargo's implicit registry when no location key is given. Two location keys refuse typed (`Error::Argument`, exit 2). `path` resolves relative to the file containing it, as Cargo resolves `path` dependencies relative to their `Cargo.toml` — so a `sources.toml` works from any invocation directory, wherever the operator keeps it ([§5.3](#53-the-gating-question)). Cargo's machine-written source-ID string form (`git+https://…#rev`) is rejected: wrong precedent for a file the engine never writes.
- **Remote sources are read, never downloaded.** `git` and `url` name a resource the model *reads* — there is no fetch leg, no source-content cache, and the no-download posture (ADR-0002 §2) holds outright. The mechanism is the seam extract already uses: the adapter's `create` grant grows a read-only view of the location, and the host binding maps location to mechanism — `github.com` to the GitHub MCP server, a Figma URL to Figma's, a plain document to a fetch of that one resource — the same way storage backings are host policy ([§3.4](#34-host-bindings)). Endpoints and credentials live in the host binding, never in this file. The grant is pinned to read-only toolsets as policy (the GitHub MCP server exposes write tools; granting them would hand the model mutation authority over the operator's GitHub — C3 one level up from the listener).
- **`git` pins with a compact `@ref`; both remote keys are reserved, not implemented.** `git = "https://github.com/acme/api@v2.3.0"` — the `@` separator emery's adapter selectors already use; the ref is a tag, branch, or SHA, resolved to one commit before extract so claim anchors and the re-mine diff sit on an immutable revision; absent means the default branch, resolved the same way. No separate `tag`/`rev`/`branch` keys. `url` has no ref namespace and is a live read; digest pinning and verify-on-read come later, if at all. Until the read-view grant exists, both keys parse but refuse typed (`source-remote-unsupported`, in the mold of `adapter-github-uri-unsupported`) — reserving them keeps the file format stable across that future.

Draft schema, to be refined:

```toml
# sources.toml — operator-authored; the engine never writes this file.
# Passed explicitly: `emery specify --sources sources.toml`.

# Workspace lend of the invocation directory (the default: path = ".").
[sources.docs]
adapter = "emery:documentation@1.2.0"

# Local path, resolved relative to this file. The table key is the
# binding key, so one adapter may bind several roots.
[sources.api-surface]
adapter = "typescript"
path = "packages/api/src"

# Inline value instead of a filesystem view — the file form of `--value`.
[sources.intent]
adapter = "intent@1.0.0"
value = "Ship a location-independent spec generator."

# Local component path as the adapter selector, mirrored into the cache
# at ensure. Adapter location and source location are separate axes.
[sources.custom]
adapter = "./adapters/custom.wasm"

# Remote repository, read at the pinned ref over the host-bound read
# grant (GitHub MCP server) — never downloaded. Reserved; refuses typed
# until the read-view grant exists.
[sources.upstream-api]
adapter = "documentation"
git = "https://github.com/acme/api@v2.3.0"

# Remote resource behind an MCP-served product; the host binding routes
# the domain to Figma's MCP server. Reserved, as above.
[sources.checkout-flow]
adapter = "documentation"
url = "https://www.figma.com/design/aBcD3fG/checkout-flow"

# Document at a URL, read live at extract time. Reserved, as above.
[sources.api-contract]
adapter = "documentation"
url = "https://example.com/spec/openapi.yaml"
```

### 5.3 The gating question

With `project.yaml` gone, the binding list is an input owned by whoever runs `specify` — argv, optionally carried by a `sources.toml` they own ([§5.2](#52-the-replacement-authority)). Nothing shares it. Today's collaboration model is one operator generating and teammates reviewing the materialised documents (`emery show`); sharing the binding list between operators is a multi-operator concern, deferred until co-owned specifications are a decided feature (risk 3). With adapter-floor-only versioning that leaves no `project.yaml`-shaped residue for step 6 and `atomic.rs` to worry about. Any premature answer — bindings re-homed into engine storage, implicit `sources.toml` discovery — reintroduces the record this section deletes.

## 6. Deletions

- `crates/artifacts/src/atomic.rs` — blobstore writes are complete-on-finalize; the pointer swap is `StateStore::cas`. No working-tree write remains, so the module and its test suite go outright. Its live call sites are exactly two: `home.rs` (gone in step 3) and `project.yaml`'s writer (gone in step 5); `copy_write` is already test-only.
- The crash-litter half of `Home::prune` — no temp files exist to leak. Superseded-generation prune on pointer swap stays (today's semantics).
- `cache_dir()` and its `create_dir_all` in `src/main.rs`; the cache mount and `GUEST_CACHE_MOUNT`.
- The writable `.` mount — drops to read-only unconditionally (step 6); with no working-tree write, `specify` needs no write route at all.
- Path-math surface of `Locations` (`store_entry`, `store_meta`, `component`, `cache_dir`) replaced by key formulas.
- `ComponentMeta` and the cache's `<name>.meta.yaml` sidecar — write-only provenance. Its one reader is `init`'s `bind` (persisting the canonical `file://` source onto the project record), and both die in step 5. Deleted in step 4, never ported to keyvalue; `bind` falls back to `persist_value` for the single step until `init` goes. The store's verify-on-read `.meta` is **not** in this bullet: it is the fail-closed gate before executing adapter bytes and moves to keyvalue intact.
- `.emery/project.yaml`, `emery_engine::project::Project`, `RequestContext`'s project load, `Error::NotInitialized` / `Error::CliTooOld` as project-file gates, and — in step 5 — the `init` verb, `--upgrade`, and the `/emery:init` skill wrapper ([§5](#5-deleting-projectyaml)).

## 7. Execution steps

Each step is one reviewable change: journey test green, `cargo make ci` green, deletions named. Steps keep their numbers — the rest of this document cross-references them — but the **execution order swaps steps 3 and 4**: the cache/store move is independent of the pointer move, and landing it first lets step 3 and step 5 land back-to-back, closing the review valley in which `.emery/spec/` is no longer written but `show` does not yet exist (the only human read path inside that valley is the filesystem store backing).

```text
omnia prerequisite ──┐
step 1 (seam) ───────┴→ step 2 (host bindings) → step 4 (cache + store) → step 3 (pointer) → step 5 (show + init; two repos)
                                                                                                └→ step 6 → step 7 → step 8
```

**Omnia prerequisite — filesystem keyvalue + `StateStore` atomics.** The two items in [§3.5](#35-omnia-work-this-design-requires), one PR each. Exit criterion: a host test proving the CAS semantics against the filesystem backend — absent-expected, stale-expected, and contention under the per-key lock. Runs in parallel with emery step 1. **Blocks emery step 2.**

**Step 1 — the storage seam, filesystem-backed.** Introduce the engine storage capability traits (keyvalue + blobstore shapes, §3.3, `cas` included in the signature) following the existing `Source` provider pattern, and route `home.rs` and the resolve legs (`ensure::seed`, `ComponentMeta`, the resolver's verify-and-load) through them, with a native filesystem implementation that preserves today's on-disk layout byte-for-byte. Pure refactor: no observable change, no WIT dependency yet. The journey test moves its assertions off filesystem paths (`.emery/spec/current`, `.emery-cache/…`) onto the scripted in-memory storage provider and the envelope — which also retires its `set_current_dir` global-state hack. Update `docs/standards/testing.md` for the boundary shift (engine state is observed through the storage provider and envelope, not the filesystem). Exit criterion: no `std::fs` against engine-owned state outside the native storage implementation.

**Step 2 — omnia host bindings.** Wire the `WasiKeyValue` / `WasiBlobstore` host entries into `src/main.rs` over the §3.5 backends (`omnia-filesystem` + the new filesystem keyvalue), both roots at the invocation directory, and route the step-1 traits over the omnia guest capabilities (`StateStore` / `BlobStore`, including `cas`). The engine guest stops opening engine-state paths; the mounts table does not shrink yet (steps 4 and 6). Exit criterion: `init` → `specify` → process restart → `specify` still reports the re-mine diff (store durability, §4 scope 1).

**Step 4 — component cache and store move (lands before step 3).** Cache and global store entries become blobstore objects. The store's verify-on-read `.meta` sidecar moves to keyvalue, digests unchanged; the cache's `ComponentMeta` sidecar is deleted here, not ported ([§6](#6-deletions)) — its one reader is `init`'s `bind`, which falls back to `persist_value` for the single step until `init` dies in step 5. Local-`.wasm` mirroring (today `init`'s `ensure`; after step 5, `specify`'s) writes through the capability — incidentally fixing that `ensure::seed` copies with plain non-atomic `fs::copy` / `fs::write` today. Delete the cache mount, `cache_dir()` and its mkdir in `main.rs`, `GUEST_CACHE_MOUNT`, and the path-math methods of `Locations` this obsoletes.

**Step 3 — pointer and generations (lands after step 4, with step 5 immediately behind).** `current` becomes a keyvalue entry; generation sets become blobs keyed by digest; commit order is blobs-then-`cas` on the pointer ([§3.2](#32-authority-model)). The CAS conflict is a new operator-visible typed failure — two concurrent `specify` runs no longer last-write-wins — mapping to exit 1: environmental contention, not operator input. Delete the litter half of `prune`; superseded-generation prune on pointer swap stays. The `.emery/spec/` tree stops being written. Container/key naming is reviewed against layer 3 (git projection) here so nothing precludes it.

**Step 5 — `emery show` and collapse `init`.** One emery PR carrying layer 1 and the §5 collapse together: `emery show spec|design` of the current generation over the storage capability; `specify` takes the adapters positionally, via `--value`, or via `--sources sources.toml` (mixing refuses typed, exit 2); ensure/mirroring moves to `specify`'s resolve leg; and the deletions — `init`, `--upgrade`, `project.yaml`, `emery_engine::project`, `RequestContext`'s project load, `Error::NotInitialized`, and `Error::CliTooOld` (exit 3 keeps only `AdapterCliTooOld`). The journey test becomes a single `specify` over the mock source, then `show`, then the byte-stable re-run. Route-budget tripwire, gate tripwires naming the deleted decisions, `docs/reference/cli-output-shapes.md`, and the AGENTS.md verb list and exit table update in the same change; `cargo make links` gates the doc changes. A companion emery-adapters PR lands in the same phase: the eval runner drops its `init` leg and wall-clocks `specify` → committed generation, and the `/emery:init` skill wraps `specify` or is deleted — named in the same decision. Must precede step 6 so `atomic.rs` has no project-config call site.

**Step 6 — read-only working tree.** Nothing writes the working tree after steps 3 and 5, so the `.` mount drops to read-only and the C3 least-authority posture holds outright; `crates/artifacts/src/atomic.rs` and its test suite are deleted. Review is the envelope plus `show`. Update reference docs in the same change.

**Step 7 — read-only MCP resource.** Serve the current generation and id on the existing listener (layer 2), beside the adapter shelves. The C3 refusal contract is untouched — a wire-contract test asserts mutating routes still refuse. The plugin skill may consume it.

**Step 8 — deployment profiles.** Document and exercise one non-filesystem binding end-to-end (project-id-keyed, multi-project host) to prove the freedom is real, and measure remote-binding performance here — the numbers in risk 4 stay `unconfirmed` until this step. Layer 3 (git projection) is scoped as its own design if wanted.

## 8. Risks and open questions

1. **WIT instability** (§3.4): upstream `wasi-keyvalue` / `wasi-blobstore` churn lands on us as seam maintenance. Mitigated: omnia already vendors and pins its fork of both WITs and owns the host implementations, so churn is absorbed there as a versioned seam change, as with the adapter WIT.
2. **Test surface shift**: the filesystem stops being a public observable boundary for engine state; the scripted storage provider and the envelope become the boundary. Handled in step 1.
3. **Multi-operator ownership of a specification** ([§5.3](#53-the-gating-question)): sharing the binding list between operators — beyond one operator generating and teammates reviewing `emery show` output — is out of scope. If co-owned specifications become a feature, that is its own design; do not back into it via binding persistence (engine storage, implicit `sources.toml` discovery), each of which reintroduces a `project.yaml`-shaped residue.
4. **Performance of remote bindings**: unconfirmed; measure in step 8 before claiming anything.
5. **Omnia prerequisite latency** ([§3.5](#35-omnia-work-this-design-requires)): emery step 2 waits on the filesystem keyvalue backend and the `StateStore` atomics; step 1 does not. Do not substitute an in-memory `KeyValueDefault` or a blobstore-keyed pointer to skip the wait — that drops restart durability or forks `current` off keyvalue.
