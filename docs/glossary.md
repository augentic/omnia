# Glossary

Shared vocabulary for Omnia, Specify, and the `backends` repo. Older docs and comments used **floor** for the runtime platform; that term is retired here in favor of the entries below.

## Runtime platform

### Runtime core

The domain-agnostic in-tree platform: the `omnia` kernel, WASI host crates (`wasi-*`), WIT contracts, guest registry, and host-mediated dispatch. It routes opaque identities and satisfies typed effects; it does not embed adapter names, workflow policy, vendor model ids, or other consumer-specific knowledge.

Same layer as **Kernel + WASI interfaces** in [Architecture.md](Architecture.md). In Specify RFCs this is also called the **Omnia runtime core**.

### Runtime contract

The stable WIT boundary and host behavior guests depend on — what must stay generic under **Law 2**. Often used when contrasting with pluggable backends (“vendor detail, never part of the runtime contract”).

### Law 2

Specify’s invariant that domain-specific knowledge lives in backends, guests, and orchestration — never in the runtime core. Omnia stays generic; which model, filesystem backend, or adapter satisfies an interface is deployment configuration the runtime core never parses.

## Host-side model completion (`wasi-model`)

### Host-side

Work done inside Omnia host crates (validation, dispatch, workspace resolution) as opposed to backend-side logic in the `backends` repo (e.g. genai’s tool loop).

### Host validation gate

The `complete` binding’s pre-checks and final answer validation before the guest sees a result. Backends may self-check internally; the host re-validates as the single authority.

### Host-injected tools

Tools the host merges into a completion from `grants` (`resolve`, `read`, `list`, `write`, `verify`). Guests must not redeclare these names in `prompt.tools`.

## Other uses of “floor” (not the runtime platform)

These appear elsewhere in Augentic docs and code with a **different** meaning. Do not read them as “runtime core.”

| Term | Meaning | Example |
|------|---------|---------|
| **Baseline / floor, not ceiling** | Minimum guaranteed behavior; implementations may do more | Declarative HTTP route table is the baseline; programmatic routing is allowed ([RFC-56](../../specify/rfcs/rfc-56-runtime-move.md)) |
| **Compatibility floor** | Minimum CLI or tool version required | `project.yaml` `specify_version`, adapter `requires_specify` in `specify-cli` |
| **Minimum cap / floor** | Hard lower bound on a spec limit, not a target to hit | Skill line caps, ignore-directive rationale length |

When in doubt: if the sentence is about **Omnia hosting guests**, use **runtime core**. If it is about **versions or numeric limits**, use **minimum** or **compatibility floor**.
