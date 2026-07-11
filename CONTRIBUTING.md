# Contribution Guide

Augentic welcomes community contributions to `omnia`.

Since the project is still evolving quickly, we **strongly** recommend opening
a GitHub issue to discuss any non-trivial change with the core team before you
start, so your work stays consistent with the project's direction and
architecture. There are many ways to help besides contributing code:

- File bugs or fix open issues
- Improve the documentation

## Getting started

- [AGENTS.md](AGENTS.md) — repository overview, key commands, and gotchas
  (toolchain pins, `wasm32-wasip2` target, nightly rustfmt).
- [docs/getting-started.md](docs/getting-started.md) — building and running
  your first guest.
- [docs/guides/testing.md](docs/guides/testing.md) — the integration-first
  testing policy; seam tests are the spec.
- [docs/glossary.md](docs/glossary.md) — project terminology.

## Before you open a pull request

Run the full CI check locally and make sure it passes:

```shell
cargo make ci
```

This runs clippy (warnings deny), the test suite (`cargo nextest`), doc tests,
a formatting check (`cargo +nightly fmt --all --check`), and the dependency
audits. Individual tasks are listed in [Makefile.toml](Makefile.toml).

Checklist:

1. Create a feature branch off `main`.
1. [Rebase](https://git-scm.com/book/en/v2/Git-Branching-Rebasing) your branch
   against `main` before submitting.
1. Include tests for your change, following the testing policy above. If a
   change is genuinely hard to test, say why in the commit message.
1. Accept the Developer's Certificate of Origin on every commit (see below).
1. Give each commit a conventional prefix describing the change
   (`fix:`, `feat:`, `perf:`, `docs:`, ...).

## Review

All contributions are made via pull request against `main`, and **all patches
from all contributors get reviewed**. At least one approval from a maintainer
is required (including for patches submitted by maintainers). When CI fails,
authors are expected to update the pull request until it passes. A maintainer
with write access merges their own pull request after approval.

## Code style

- Rust code must match the output of `cargo +nightly fmt --all`.
- Workspace lints are strict (`missing_docs`, clippy `pedantic`, warnings
  denied); `cargo make lint` must pass clean.
- See the code-comment guidance in [AGENTS.md](AGENTS.md): document intent,
  not mechanics.

## Developer's Certificate of Origin

All contributions must include acceptance of the
[DCO](https://developercertificate.org/):

```text
Developer Certificate of Origin
Version 1.1

Copyright (C) 2004, 2006 The Linux Foundation and its contributors.
660 York Street, Suite 102,
San Francisco, CA 94110 USA

Everyone is permitted to copy and distribute verbatim copies of this
license document, but changing it is not allowed.


Developer's Certificate of Origin 1.1

By making a contribution to this project, I certify that:

(a) The contribution was created in whole or in part by me and I
    have the right to submit it under the open source license
    indicated in the file; or

(b) The contribution is based upon previous work that, to the best
    of my knowledge, is covered under an appropriate open source
    license and I have the right under that license to submit that
    work with modifications, whether created in whole or in part
    by me, under the same open source license (unless I am
    permitted to submit under a different license), as indicated
    in the file; or

(c) The contribution was provided directly to me by some other
    person who certified (a), (b) or (c) and I have not modified
    it.

(d) I understand and agree that this project and the contribution
    are public and that a record of the contribution (including all
    personal information I submit with it, including my sign-off) is
    maintained indefinitely and may be redistributed consistent with
    this project or the open source license(s) involved.
```

To accept the DCO, add this line to each commit message with your name and
email address (`git commit -s` does this for you):

```text
Signed-off-by: Jane Example <jane@example.com>
```

For legal reasons, no anonymous or pseudonymous contributions are accepted;
open a GitHub issue if this is a problem for you.

## Conduct

Whether you are a regular contributor or a newcomer, we care about making this
community a safe place for you and we've got your back.

- We are committed to providing a friendly, safe and welcoming environment for
  all, regardless of gender, sexual orientation, disability, ethnicity,
  religion, or similar personal characteristic.
- Please avoid using nicknames that might detract from a friendly, safe and
  welcoming environment for all.
- Be kind and courteous. There is no need to be mean or rude.
- We will exclude you from interaction if you insult, demean or harass anyone.
  In particular, we do not tolerate behavior that excludes people in socially
  marginalized groups.
- Private harassment is also unacceptable. No matter who you are, if you feel
  you have been or are being harassed or made uncomfortable by a community
  member, please contact a member of the Omnia core team immediately.
- Likewise any spamming, trolling, flaming, baiting or other
  attention-stealing behaviour is not welcome.

We welcome discussion about creating a welcoming, safe, and productive
environment for the community. If you have any questions, feedback, or
concerns please let us know with a GitHub issue.
