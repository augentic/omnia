# Omnia Architecture Review, November 2026

This review covers the Omnia runtime, its production backends, and its reference guest. It looks for ways to improve the architecture, the module boundaries, the design of the main abstractions, and the lower-level implementation. It is a read-only review: nothing in the three repositories was changed to produce it, apart from this file.

| Repository | Branch | Commit | Version at review |
| --- | --- | --- | --- |
| `augentic/omnia` | `main` | `802806f7` (2026-11-27) | 0.39.0, unreleased (crates.io holds 0.38.1) |
| `augentic/omnia-backends` | `main` | `b70386b` (2026-11-27) | 0.33.0, built against omnia `main` over git |
| `augentic/omnia-exemplar` | `main` | `d44a8fc` (2026-11-25) | 0.2.0, built against omnia 0.38.1 from crates.io |

The review ran in six phases, each appending its findings to [Appendix B](#appendix-b-running-findings-log) before the next began. Phase 0 fixed the charter in [Appendix A](#appendix-a-phase-0-charter). Phase 6 wrote sections 1 to 7 from the log. Every finding cites a path, and a line where one helps, at the commits above. Paths are relative to the repository named in the finding's area. The reviewer was Claude Opus 5.5 (Max) in Cursor's local agent, with read-only subagents on the same model for breadth.

This file is not listed in `docs/SUMMARY.md`, so the published book does not render it and its link check does not cover it.

## 1. Executive summary

_Written in phase 6._

## 2. Layer diagram

_Written in phase 6._

## 3. Consolidated findings

_Written in phase 6._

## 4. Top 10 incremental refactors

_Written in phase 6._

## 5. Top 10 risks

_Written in phase 6._

## 6. Out of scope and not reviewed

_Written in phase 6._

## 7. Rubric scores

_Written in phase 6._

---

## Appendix A. Phase 0 charter

### A.1 Scales

**Severity** says how soon a finding should be acted on.

| Severity | Meaning |
| --- | --- |
| P0 | A documented invariant is broken in a way that can cause a security hole, data loss, or a broken release today. Fix before the next release. |
| P1 | A structural problem that costs on every change, or a correctness risk under plausible conditions. Plan it within the next one or two releases. |
| P2 | A worthwhile gain in cohesion, clarity, or performance. Schedule it when the area is next touched. |
| P3 | Polish. Fix it in passing. |

**Type** says what kind of change the recommendation is.

- **boundary**: what a crate, a repository, or the guest/host line exposes or depends on.
- **module**: how responsibilities are split across crates, modules, and files.
- **design**: the shape of an abstraction, a protocol, or a policy.
- **implementation**: the code inside a function or type.

**Effort** is S for under a day, M for one to five days, and L for more than a week or any change that needs coordinated releases across repositories.

**IDs** are stable across re-ranking: `C-` for composition (phase 1), `W-` for WIT and the SDK (phase 2), `R-` for the runtime and its tests (phase 3), `B-` for backends (phase 4), `E-` for the exemplar (phase 5), and `X-` for cross-cutting findings that phase 6 raises.

### A.2 Rubric

Phase 6 scores each major area from 1 (poor) to 5 (exemplary) on six criteria.

| Criterion | What it looks for |
| --- | --- |
| Boundary clarity | The composition root, `omnia-core`, the `wasi-*` hosts, and the backends each expose only what their users need. A guest needs `omnia-sdk` alone. |
| Cohesion | Each crate and module has one job, and no module is a catch-all. |
| Extensibility | A new WASI capability or backend lands without edits scattered across the tree. |
| Correctness encoding | The `test-programs` pipeline and the contract rows hold the architecture's promises as executable checks. |
| Operability | Configuration, telemetry, deployment, and failure modes are clear to an operator. |
| Consistency | The docs, the `AGENTS.md` files, and the code agree. |

### A.3 Documented invariants

These are the promises the repositories make about themselves. A finding that breaks one is at least P1. Each line names where the promise is written.

**Dependency law**

1. An embedder depends on `omnia` alone. A deployment's `Cargo.toml` never names `omnia-core`, `omnia-link`, `omnia-plugin`, or `omnia-cli`, and no path `runtime!` emits does either. Source: `docs/Architecture.md` (Crate Organization) and `omnia/AGENTS.md` (Overview).
2. A capability crate targets `omnia-core`, not the composition root. Source: `docs/Architecture.md` (three-layer diagram).
3. `omnia-cli` and `omnia-compile` are leaf crates. `omnia-cli` has no `omnia-*` dependency, and `omnia-compile` depends on wasmtime alone. Source: `docs/Architecture.md`.
4. A guest depends on `omnia-sdk` alone, and the `omnia-wasi-*` crates are host-only. In the exemplar, an `omnia-wasi-*` crate is only ever a stem's target-gated dev-dependency, and `deny.toml` keeps `wasmtime` and `omnia-core` off the native non-dev graph. Sources: `omnia/AGENTS.md`, `omnia-exemplar/AGENTS.md`, `omnia-exemplar/README.md` (Guest packaging).

**Runtime law**

5. Law 2: the runtime core holds no domain knowledge. Which backend serves an interface is deployment configuration the core never parses, and the core embeds no adapter names, workflow policy, or vendor model ids. Source: `docs/glossary.md` (Law 2, Runtime core).
6. The namespace rule: an import or export under `wasi:` or `omnia:` belongs to the host, and every other interface crosses between guests. `omnia::is_host` is the predicate, and it is closed. Source: `docs/Architecture.md` (The Guest Registry), `docs/glossary.md` (Namespace rule).
7. One admission body: every admitter passes through `Runtime::admit_bytes` under a `Policy`, `Declared` or `CallerNamed`, so no two paths can drift in what they check. Source: `docs/security-model.md` (Deployment inputs are trusted).
8. The source kind fixes the artifact format. Pre-compiled artifacts load only from bytes the process held before any guest ran, or from a pinned `source.path`. A package or a caller-named load admits raw wasm alone. Source: `docs/security-model.md`.
9. Grants are opt-in per guest. Inside a table, an omitted list grants nothing. Each list is held at the points the security model's table names, and startup refuses a table that reaches nothing. Source: `docs/security-model.md` (Per-guest grants).
10. Every invocation runs in a fresh instance in its own store, and no state survives between requests. Source: `docs/Architecture.md` (Isolation and pooling).
11. No host call waits forever. `GUEST_TIMEOUT_MS` counts from when an invocation takes its slot of the concurrency gate, and the deadline a guest reads is the one the host enforces. Source: `docs/reference/provider-contract.md` (Bound every call), `RELEASES.md` 0.39.0.
12. A provider passes the shared `omnia_test::contract` rows and enforces each argument rule at the layer the provider contract names. Source: `docs/reference/provider-contract.md` (Obligations).

**WIT law**

13. A published WIT release is immutable. Any change to its WIT, docs included, needs a new version, and CI's `WIT` job fails otherwise. Source: `docs/reference/wit-packages.md` (Versions).
14. A `wit/deps` copy is edited only through its owner and `mise run wit-sync`. The guest bindings under `crates/sdk/src/bindings` are generated, never edited by hand. A published package holds no plain `//` comment. Source: `omnia/AGENTS.md` (Gotchas), `docs/reference/wit-packages.md` (Authoring a package).
15. `wit-bindgen` moves only with `wasip3`. Source: `omnia/AGENTS.md` (Gotchas).

**Test law**

16. A guest-instantiating test exists only through the `test-programs` pipeline, and `foreach_<capability>!` refuses a program without its paired test. Triggers are driven in-process. Each outcome is asserted in one place. Examples are not a test tier. Source: `docs/guides/testing-policy.md`, `omnia/AGENTS.md` (Testing policy).
17. In the backends, a protocol-faithful fake drives the guest e2e tier where the protocol is small enough to fake, and every other backend is accepted by `#[ignore]`-gated live tests. No file under `crates/*/src` carries a unit test today. Source: `omnia-backends/AGENTS.md` (Testing policy).

**Release law**

18. Every `omnia-*` dependency of the backends sits on one line and resolves from crates.io. `[patch.crates-io]` holds only a commented-out path block, and no git or path override is ever committed. Source: `omnia-backends/AGENTS.md` (Gotchas).
19. A merge to the exemplar's `main` is a release act, because the Emery adapter pins exemplar commits and scaffolds the guest template from them. `Role::Static` template files are never edited by a generated guest. Source: `omnia-exemplar/README.md` (Guest template contract), `omnia-exemplar/AGENTS.md`.

**Style law**

20. Comments and docs follow the shared code-style rules, and `#[expect(..)]` replaces `#[allow(..)]`. Clippy and nightly rustfmt are the gate. Source: each repository's `AGENTS.md` (Code style).

### A.4 Open questions for later phases

These are hypotheses from reading the canon, not findings. Each later phase answers its own with evidence.

**Phase 1, composition and dependency law**

1. RELEASES 0.39.0 hides `RuntimeParts`, `Runtime::from_parts`, `RegistryParts`, `Registry::assemble`, `LinkSeam`, `NoLinks`, and `Plugins::install` from the docs but keeps them public for omnia's own crates. Is that a de-facto public API a third-party capability crate can reach, and does semver cover it?
2. Does `omnia` re-export `omnia-core` through a curated list or a glob, and does any `omnia::` signature expose an `omnia_core::` type that rustdoc shows under its original path?
3. Are the `link`, `loader`, and `jit` features additive, and is each combination that changes behaviour built and tested?
4. Can `Deployment` plus `Wiring` stand on their own without `runtime!`, and does the generated code embed policy the library should own?
5. The namespace predicate is closed, so a third-party native host must live under `omnia:`. Is that a tension with Law 2, since a vendor's interface then sits in the runtime's own namespace?

**Phase 2, WIT, SDK, and xtask**

1. All fourteen packages are at 0.1.0. When one needs 0.2.0, can a host serve both versions while guests migrate, given that each host's `bindgen!` names its interfaces without a version?
2. `omnia-sdk` carries the bindings for every interface. Does a guest that uses only HTTP still import interfaces it never calls, and what does that do to its `interfaces` grant list?
3. Is the single `omnia:types` error a good fit for every interface, or does it flatten distinctions a guest needs?
4. Is the hand-maintained `wit/deps` copy model worth its cost next to `wkg fetch`?

**Phase 3, runtime execution and tests as architecture**

1. RELEASES 0.39.0 is dominated by fixes to the concurrency gate, leases, the messaging share, and pool exhaustion. Is the resource model coherent enough to explain in one page, or is it accreting rules?
2. Should telemetry, at 914 lines in `omnia-core`, be its own crate?
3. Is first-use loading under a burst correct, including how a failed load's error is shared and whether it is retried?
4. What does a link call cost, given a fresh callee instance per call, and how does a handle that cannot cross the seam surface to a guest author?
5. `omnia-test` is published and carries contract rows, build helpers, and host doubles. Is it one crate with one job, and what semver promise does it make?

**Phase 4, production backends**

1. The backends build against omnia `main` through a committed git patch. What does that do to reproducibility, to publishing, and to the stated release law?
2. Do the backends enforce the provider contract's argument rules uniformly: a zero TTL, an undeclared name, an unserved request?
3. The cursor backend is about 8,000 lines. Is it a backend, or an application that happens to implement `ModelClient`?
4. Is `omnia-sandbox` generic or model-specific, and how portable is it across macOS and Linux?

**Phase 5, exemplar and guest template**

1. The template ships as a crate whose files are read with `include_str!`, and consumers pin exemplar commits. Is "every merge is a release" sustainable, and what protects consumers between releases?
2. The exemplar is on omnia 0.38.1 while 0.39.0 changes public traits. What is its upgrade path, and what does the template tell a generated guest about it?
3. Do the guest `clippy.toml` bans cover every way a handler could reach around the host, and do they agree with what `omnia-sdk` itself exposes?

**Cross-cutting**

1. The docs read as a specification. Many paragraphs carry a dozen rules each. Is there a normative layer separate from the explanatory one, and can a reader find a rule's single home?

---

## Appendix B. Running findings log

Each phase appends one table and one note per finding. The table cells stay short. The notes carry the evidence.

### B.1 Phase 1: composition root and dependency law

| ID | Area | Severity | Type | Location | Observation | Recommendation | Effort |
| --- | --- | --- | --- | --- | --- | --- | --- |

### B.2 Phase 2: WIT, SDK, and xtask

| ID | Area | Severity | Type | Location | Observation | Recommendation | Effort |
| --- | --- | --- | --- | --- | --- | --- | --- |

### B.3 Phase 3: runtime execution and tests as architecture

| ID | Area | Severity | Type | Location | Observation | Recommendation | Effort |
| --- | --- | --- | --- | --- | --- | --- | --- |

### B.4 Phase 4: production backends

| ID | Area | Severity | Type | Location | Observation | Recommendation | Effort |
| --- | --- | --- | --- | --- | --- | --- | --- |

### B.5 Phase 5: exemplar and guest template

| ID | Area | Severity | Type | Location | Observation | Recommendation | Effort |
| --- | --- | --- | --- | --- | --- | --- | --- |

---

## Appendix C. P0 and P1 verification notes

_Written in phase 6._
