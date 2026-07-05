# Design: MCP Grants — Enforcement, Resolution, and Delivery

> Status: Proposal · Depends: the `wasi-model` boundary (`crates/wasi-model`), the `omnia-cursor` backend · Owns: the MCP grant surface (`tool::mcp`) from WIT contract to backend delivery

## Abstract

The MCP strategy today is sound in shape: a guest grants a *logical* server name per completion (`tool::mcp { name, tools, url }` in `omnia:model@0.1.0`), the grant itself carries the endpoint `url`, and serving MCP is itself just another `wasi:http` route — an MCP server can be an omnia guest (`examples/mcp`).

This document does not rethink that shape. It tracks three gaps between the contract and the implementation, ordered by priority. The first is the one worth doing now.

## Current placement

| Layer | Behavior today |
|-------|----------------|
| WIT contract | `tool::mcp { name, tools, url }`; the guest supplies the endpoint `url` directly; `tools` documented as an allowlist |
| `wasi-model` host | Passes MCP grants through `PreparedRequest` untouched; the gate only checks *function*-name shadowing |
| `omnia-genai` | Bails loudly on any MCP grant (no MCP client) |
| `omnia-cursor` | Reads each grant's guest-supplied `url`, merges entries into `<workspace>/.cursor/mcp.json` under a refcounted guard, passes `--approve-mcps`, prepends a natural-language hint |

Backends own only *delivery*: `.cursor/mcp.json` for cursor, native provider API or the §2 bridge for others. A backend that wires MCP reads the grant's `url` directly.

## 1. Enforce the tool allowlist (contract gap)

The WIT documents `mcp.tools` as a "tool allowlist; empty exposes every tool". The cursor backend only turns it into prose in the prompt ("use only: …") — the spawned agent still sees, and can call, every tool the server advertises. A stated capability guarantee that is actually advisory is the worst of both worlds.

Resolution, in preference order:

1. **Enforce it.** Front the granted server with a per-completion filtering MCP proxy endpoint: `tools/list` responses are filtered to the allowlist, `tools/call` outside it is rejected. The spawned agent (or any backend) is pointed at the proxy URL, never the raw endpoint.
2. **Failing that, re-document it.** If enforcement is deferred, the WIT doc must say "advisory hint", not "allowlist".

The proxy is the same component §2 needs, so one piece of work covers both.

## 2. Bridge MCP into the genai tool loop

genai's hard bail on MCP grants is correct today (fail loudly rather than drop a grant), but once the genai tool loop grows past `resolve`-only dispatch, a small shared MCP client (HTTP `tools/list` + `tools/call`) can present a granted server's tools as ordinary function tools to any hosted provider. MCP grants stop being cursor-only, and the same client is the enforcement proxy of §1.

## 3. Harden the `.cursor/mcp.json` guard (stopgap, treat as one)

The refcounted per-workspace registry handles concurrent completions correctly, but it mutates shared state in the user's real workspace:

- A SIGKILL mid-spawn leaves the file modified with no restore.
- An external edit mid-completion is lost on the guard's re-merge.

Mitigations, cheapest first:

- Write the captured original to a sidecar (e.g. `.cursor/mcp.json.omnia-orig`) so a later run detects and repairs a crashed restore.
- Watch for a `--mcp-config`-style flag in `cursor-agent` (its absence is the sole reason the module exists). The moment it ships, the module deletes itself — that is the real simplification.

## Out of scope

- The MCP server guest itself (`examples/mcp`) — the established pattern for serving MCP as a `wasi:http` route.
- Streaming (`create-stream`) — YAGNI-commented out of the WIT until a backend streams (see `model.wit`).
- Transcript capture for spawned-agent MCP calls — rides the replay expansion in [RFC-58](rfc-58-backend-router.md).

## References

- `crates/wasi-model/wit/model.wit` — the authoritative `mcp` record.
- `backends/crates/cursor/src/mcp.rs` — the `.cursor/mcp.json` guard §3 hardens.
