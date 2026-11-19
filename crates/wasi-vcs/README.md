# Omnia WASI VCS

This crate provides the version-control interface for the Omnia runtime: the `omnia:vcs` boundary a guest reaches repositories and working copies through, in the nouns git and jj share.

## Interface

Implements `omnia:vcs` ([`wit/vcs.wit`](wit/vcs.wit)), the interface the
SDK's `Vcs` calls, over one `VcsClient` per backend.

- A guest opens the repository or working copy at a `location`, and calls
  it through the `workspace` handle `open` returns. A place that holds none
  fails with `not-found`. `init` lays a repository down, and keeps one
  already there.
- A `location` is a mount root the guest holds and a plain relative
  subpath, as `omnia:model`'s is. `init`, `add`, and `clone` lay its
  directory down where nothing stands. The host resolves it before any
  backend runs, and refuses:
  - a root that is not a mount, or a subpath that leads out of one, with
    `denied`;
  - a subpath that is not plainly spelled, or a name a file or a symlink
    holds where a directory would be laid down, with `invalid`;
  - a change beneath a read-only mount with `denied`.
- A backend works through a `Place`, an open directory handle beneath the
  mount that the host holds for the whole call. A guest that rearranges its
  tree meanwhile cannot redirect the call, and the backend never sees a
  descriptor.
- A lookup that finds nothing is `none`, not an error. That covers a
  revision, a bookmark, a remote's bookmark, and the `head` of a workspace
  with no commits.
- A merge that leaves conflicts answers a `merge-conflict` naming their
  paths. A push the remote moved past answers `diverged`. A merge into
  pending changes fails with `invalid`, and nothing is merged.
- `log` and `status` answer a page of at most 1,000 entries, and its
  `next` cursor resumes the listing.
- `workspaces` is optional, and adds and removes a further workspace of a
  repository. `remotes` is optional, and clones, fetches, reads a remote's
  bookmark, and pushes. `WasiVcs` serves both.
- A backend picks a failure's case by returning an `omnia_wasi_vcs::Error`:
  - `NotARepository` and `NotFound` are `not-found`;
  - `Exists` and `Pending` are `invalid`;
  - `Access` is `unavailable`;
  - `Diverged` is a push's `diverged` outcome, and `other` from any other
    call;
  - `Other` and any untyped failure are `other`.
- The host answers `unsupported` for what `VcsClient` cannot express yet:
  - a `log` with no base;
  - a `set-bookmark` with a precondition.

## Backend

There is no default backend: nothing in memory stands for a repository. A deployment that names no `WasiVcs` row links no `omnia:vcs` import, and a guest that imports it fails to instantiate there.

- **Production**: [`omnia-git`](https://github.com/augentic/omnia-backends/tree/main/crates/git) — the operator's `git` binary, one process per operation (see the [Production Backends guide](https://github.com/augentic/omnia/blob/main/docs/guides/production-backends.md)).
- **Tests**: a scripted `VcsClient` of the suite's own, set with `omnia_test::host::Backends::vcs`; the bundle holds `NoVcs` in the slot until then, so a deployment linking `WasiVcs` over the defaults fails to link.

## Usage

Add this crate to your `Cargo.toml` and name a backend in your runtime configuration:

```rust,ignore
use omnia_git::Client as Git;
use omnia_wasi_vcs::WasiVcs;

omnia::runtime!({
    hosts: {
        WasiVcs: Git,
    }
});
```

A guest reaches the interface through `omnia_sdk::Vcs`, naming every location as a deployment-local path (`"."` the project mount, `"./.cache/repo"` beneath it), or through the raw `omnia_wasi_vcs::omnia::{store, workspace, workspaces, remotes}` bindings.

## License

MIT OR Apache-2.0
