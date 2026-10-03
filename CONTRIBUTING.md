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
- [docs/guides/testing-policy.md](docs/guides/testing-policy.md) — what earns
  a unit test or a live test.
- [docs/glossary.md](docs/glossary.md) — project terminology.

## Pull request procedure

Pull requests should be targeted at the `main` branch. Before creating a pull request, go through this checklist:

1. Create a feature branch off of `main`.
2. [Rebase](https://git-scm.com/book/en/Git-Branching-Rebasing) your local changes against `main`.
3. Run `make ci` and confirm that it passes: exactly the CI jobs, in order.
4. Accept the Developer's Certificate of Origin on all commits (see above).

All contributions are made via pull request. All patches from all contributors get reviewed. At least one review from a maintainer is required for all patches (even patches from maintainers). When CI fails, authors are expected to update the pull request until it passes.

Normally, all pull requests must include tests that cover your change. Occasionally, a change will be very difficult to test for; in those cases, include a note in your commit message explaining why.

`make ci` runs the formatting check, clippy with warnings denied (natively, then for `wasm32-wasip2` over lib, bins and examples), the test suite (`cargo nextest`), doc tests, rustdoc, `cargo vet`, and `cargo deny`. `make check` adds the local advisory extras (`cargo audit`, `cargo outdated`, `cargo udeps`) and rewrites formatting in place. After changing dependencies, `make vet-regen` refreshes the `supply-chain/` files that `vet` only checks.

Tests follow the testing policy above. Give each commit a conventional prefix describing the change (`fix:`, `feat:`, `perf:`, `docs:`, ...). A maintainer with write access merges their own pull request after approval.

## Code style

- Rust code must match the output of `cargo +nightly fmt --all`.
- Workspace lints are strict (`missing_docs`, clippy `pedantic`, warnings
  denied); `mise run lint` must pass clean.
- See the code-comment guidance in [AGENTS.md](AGENTS.md): document intent,
  not mechanics.

## Developer's Certificate of Origin

All contributions must include acceptance of the [DCO](https://developercertificate.org/):

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

To accept the DCO, add this line to each commit message with your name and email address (`git commit -s` will do this for you):

```text
Signed-off-by: Jane Example <jane@example.com>
```

For legal reasons, no anonymous or pseudonymous contributions are accepted; open a GitHub issue if this is a problem for you.

## Conduct

Whether you are a regular contributor or a newcomer, we care about making this community a safe place for you and we've got your back.

- We are committed to providing a friendly, safe and welcoming environment for all, regardless of gender, sexual orientation, disability, ethnicity, religion, or similar personal characteristic.
- Be kind and courteous. There is no need to be mean or rude.
- We will exclude you from interaction if you insult, demean or harass anyone. In particular, we do not tolerate behavior that excludes people in socially marginalized groups.
- Private harassment is also unacceptable. If you feel you have been or are being harassed or made uncomfortable by a community member, please contact a member of the core team immediately.
- Likewise any spamming, trolling, flaming, baiting or other attention-stealing behaviour is not welcome.

We welcome discussion about creating a welcoming, safe, and productive environment for the community. If you have any questions, feedback, or concerns please let us know with a GitHub issue. The [Code of Conduct](CODE_OF_CONDUCT.md) applies throughout.
