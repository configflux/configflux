# Contributing to ConfigFlux

Thank you for your interest in ConfigFlux. This document explains what the
project is looking for, how to report what you find, and how to build it from
source.

## License

ConfigFlux is dual-licensed under the Business Source License 1.1 (BUSL-1.1)
and commercial terms — see `LICENSE` and `LICENSING.md`. By submitting a
contribution you agree that you license your contribution to the Licensor
(configflux) such that it may be distributed under the BUSL-1.1, under the
Change License named in `LICENSE`, and under configflux's commercial license
terms. See `NOTICE` for a plain-English summary of the license.

## How to Contribute

The most useful thing you can send this project is what you learned by using
it: what broke, what confused you, what was missing, what you expected to
happen instead. Reports and ideas are read and acted on directly, and they
shape what gets built next.

One thing up front, so you do not spend a weekend on something that cannot
land: **ConfigFlux does not accept code contributions.** Pull requests opened
against this repository are not merged, and there is no route by which an
outside patch becomes part of a release. That is a decision about how the
project is maintained, not a judgement about your change — and it is not a
reason to stay quiet. If you found a bug, describe it. If you know the fix, say
what it is in the issue. The reasoning travels even when the patch cannot.

### What is welcome

- **Bug reports** — especially with a reproduction. These are the single
  highest-value thing you can file.
- **Feature requests and use cases** — describe the problem you are trying to
  solve, not only the feature you have in mind. The underlying use case is what
  gets designed against.
- **Design feedback and questions** — if a model, a schema, or a command-line
  surface does not fit how you actually work, that is worth knowing before it
  hardens.
- **Documentation gaps** — if something was wrong, missing, or misleading, say
  which page and what you expected to find instead.

The [issue tracker](https://github.com/configflux/configflux/issues) is the
place for all of it. There are templates for bug reports, feature requests, and
questions.

### What makes a good bug report

The bug template asks for these, and they are what turn a report into a fix:

- **A minimal reproduction** — the smallest input and the exact command that
  triggers the problem. This is worth more than everything else combined.
- **What you expected**, and what happened instead.
- **The version** you ran (a release tag or `--version` output), and how you
  installed it.
- **The output** — error text, diagnostic codes, or a stack trace, pasted
  rather than paraphrased.

A report that lets a maintainer reproduce the problem in one command gets fixed
quickly. A report that cannot be reproduced usually cannot be acted on at all,
however well it is written.

## Why the Project Works This Way

ConfigFlux keeps one source of truth. This repository is the published form of
the project rather than the tree where day-to-day development happens: each
release replaces the contents here in a single commit, so what you clone always
corresponds exactly to a released, tested version.

That has a direct consequence. Anything committed here — including a merged
pull request — would be overwritten by the next release. Merging one would
promise something the project cannot keep, and running a review process whose
results quietly disappear would waste your time and ours. So the project says
plainly that the inbound path is issues, and puts its attention there instead.

## Building from Source

You do not need to build ConfigFlux to file a good issue — a release binary is
enough for most reports. But building is the surest way to pin down a
reproduction, and the source is here to be read.

### Prerequisites

- **Bazel** (via Bazelisk)
- **Rust** (stable toolchain)
- **Clang/Clang++** (C++20 capable)
- **Git**

### Clone and Build

```bash
git clone https://github.com/configflux/configflux.git
cd configflux
bazel build //...
bazel test //...
```

ConfigFlux uses Bazel exclusively for builds and tests; `cargo test` and
`cargo check` are not supported entry points.

## Security

If you discover a security vulnerability, **do not** open a public issue.
Follow the reporting instructions in `SECURITY.md`.

## Questions

If you have questions about ConfigFlux, open a question issue or email the
maintainers at [hello@configflux.dev](mailto:hello@configflux.dev).
