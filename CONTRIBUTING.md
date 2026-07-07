# Contributing to ConfigFlux

Thank you for your interest in contributing to ConfigFlux. This document
explains how to set up a development environment, make changes, and submit
them for review.

## License

ConfigFlux is dual-licensed under the Business Source License 1.1 (BUSL-1.1)
and commercial terms — see `LICENSE` and `LICENSING.md`. By submitting a
contribution you agree that you license your contribution to the Licensor
(configflux) such that it may be distributed under the BUSL-1.1, under the
Change License named in `LICENSE`, and under configflux's commercial license
terms. See `NOTICE` for a plain-English summary of the license.

## Getting Started

### Prerequisites

ConfigFlux is developed inside a dev container. The recommended workflow is to
open the repository in a dev-container-aware editor (for example VS Code with
the Dev Containers extension). The container image ships Bazel, Rust, and the
C++ toolchain preconfigured.

Alternatively, install the following manually:

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

## Development Workflow

### 1. Find or Create an Issue

All non-trivial changes should be tracked in an issue. Before starting work,
check for an existing issue that covers your change. If none exists, create
one.

### 2. Branch

Create a feature branch from `main`:

```bash
git checkout -b work/<short-description> main
```

Keep commits focused and atomic. Use conventional commit messages (for example
`feat(compiler): add chunk deduplication`).

### 3. Build and Test

ConfigFlux uses Bazel exclusively for builds and tests. Do not use `cargo test`
or `cargo check` directly.

```bash
# Build everything
bazel build //...

# Run the full test suite
bazel test //...
```

All tests must pass before a change can be merged.

### 4. Submit a Pull Request

Push your branch and open a pull request against `main`. Include:

- A clear title and description of the change.
- The issue the change addresses (if applicable).
- Evidence that tests pass.

## Coding Standards

### Rust

- Follow standard Rust idioms.
- `unsafe` blocks require a `// SAFETY:` comment explaining why the invariants
  hold.
- All public items must have doc comments.

### C++

- Minimum standard is C++20.
- Follow Google C++ style (root `.clang-format`).
- Use `clang`/`clang++` for compilation.
- SDK code lives in `sdk/cpp/` and `sdk/ros2/`; runtime code stays in
  `runtime/`.
- See `docs/cpp-engineering-standards.md` for detailed rules.

### General

- Snake case for all configuration definitions and keys.
- Keep parsing logic out of schema files; business logic belongs in resolvers.
- Significant architectural decisions should be documented.

## Testing

- Write tests for every non-trivial change.
- Use Bazel test targets. If a needed target does not exist, create it.
- The project enforces requirement traceability; see
  `docs/requirement-test-matrix.tsv`.

## Security

If you discover a security vulnerability, **do not** open a public issue.
Follow the reporting instructions in `SECURITY.md`.

## Questions

If you have questions about contributing, open a discussion on the repository
or email the maintainers at [hello@configflux.dev](mailto:hello@configflux.dev).
