# ConfigFlux

[![License: BUSL-1.1](https://img.shields.io/badge/license-BUSL--1.1-blue.svg)](LICENSE)

<!-- evaluator-note: latest=v0.4.0 -->


**ConfigFlux compiles scattered configuration definitions into one validated
model, then resolves an exact, reproducible configuration for any specific
product or deployment — the same inputs always produce the same bytes.**

It is for teams that build many variants of a system from shared building
blocks — product lines, device fleets, multi-environment deployments — and
need every resolved configuration to be auditable and bit-for-bit
reproducible, the kind of guarantee regulated industries depend on.

Under the hood, ConfigFlux is a deterministic configuration compiler for
*software product line engineering* (**SPL** — see [Concepts](#concepts) for
the terms used below). It ingests decentralized configuration chunks (authored
in [CUE](https://cuelang.org)) into a **150% model** — the aggregate superset
of every option and constraint — validates that model as a typed graph, and
resolves a strict **100% model**, the exact configuration for one concrete
product context. Every output is byte-stable and hash-addressable, making
ConfigFlux suitable for regulated environments where the same inputs must
always produce the same artifacts.

The project ships reference scenario packs for robotics, industrial
automation, and building systems (water pump, wind turbine, automation cell,
mobile robot, and building HVAC). The same compiler, interpreter, and runtime
pipeline applies to any domain that can be modeled as components, parameters,
constraints, and artifacts.

## Concepts

A few terms used throughout ConfigFlux and its docs:

- **Software product line (SPL)** — a family of related products built from one
  shared, configurable set of assets, rather than maintained as separate forks.
- **150% model** — the aggregate superset: every component, option, and
  constraint across the whole product line, merged into a single typed graph.
- **100% model** — the resolved subset: the exact, validated configuration for
  one concrete product context, derived from the 150% model.
- **CUE** — the configuration language ([cuelang.org](https://cuelang.org)) used
  to author chunks; exported to JSON for ingestion.
- **CMP (Compiled Model Package)** — the compiler's output artifact: the
  validated, hash-addressable model that the interpreter and runtime consume.
- **CCM (Compiled Constraint Model)** — the compiled solver form emitted
  alongside the CMP (in the package's `ccm/` directory): a symbol table plus a
  reduced ordered binary decision diagram the solver queries to decide which
  options remain valid and whether a selection is satisfiable.

## Key Features

- **Deterministic by construction.** Compiler, interpreter, and runtime
  outputs are canonicalized and hashed. The same inputs always produce the
  same bytes and the same hashes.
- **Scenario packs.** Reusable product families under
  `compiler/scenarios/`, covering robotics, industrial, and building-systems
  examples with smoke, medium, and large gates.
- **Bazel-only build.** One build system, one command,
  hermetic targets. No mixed Cargo or CMake entry points.
- **Staged pipeline.** Compile a model package (`compiler`), run guided
  late-binding selection and resolution (`interpreter`), and serve resolved
  configuration to a target (`runtime`, C++ SDK, ROS 2 adapters).
- **Auditable outputs.** Resolved models, generated C++ headers, build flags,
  artifact manifests, and a first-party software BOM are all emitted with
  stable diagnostic codes and frozen schemas.
- **Requirement traceability.** Behaviour is mapped to tracked requirements
  end-to-end, so the guarantees above are auditable rather than assumed.
- **Model explorer** — a static, local, read-only web UI (`explorer/`) for
  browsing compiled models, resolved snapshots, and explain reports; no build
  step, no backend, works fully offline.

## Install

Every release publishes prebuilt, self-contained Linux binaries. No toolchain
is required — download, verify, run. To build from source instead, see
[Quick Start](#quick-start) below.

### 1. Download

Pick the tarball for your platform from the
[latest release](https://github.com/configflux/configflux/releases):

- Linux x86_64 — `configflux-vX.Y.Z-x86_64-linux.tar.gz`
- Linux aarch64 — `configflux-vX.Y.Z-aarch64-linux.tar.gz`

Each tarball is published with a `.sha256` checksum and a `.sig`/`.crt`
signature pair. The release also carries a combined `SHA256SUMS` manifest that
covers every tarball, signed as a single file.

### 2. Verify

Check the download before extracting it. Run this from the directory you
downloaded into:

```bash
# Verify one asset against its own checksum file
sha256sum -c configflux-vX.Y.Z-x86_64-linux.tar.gz.sha256

# Or verify against the combined manifest, ignoring platforms you skipped
sha256sum -c --ignore-missing SHA256SUMS
```

Optionally verify the signature as well. `SHA256SUMS` is signed with keyless
[cosign](https://github.com/sigstore/cosign) using the release workflow's
GitHub OIDC identity, so a good signature shows the checksums were produced by
this project's release pipeline and not substituted afterwards:

```bash
cosign verify-blob \
  --certificate-identity-regexp '^https://github\.com/configflux/configflux/\.github/workflows/release\.yml@refs/tags/v.*$' \
  --certificate-github-workflow-ref 'refs/tags/v<version>' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  --certificate SHA256SUMS.crt \
  --signature SHA256SUMS.sig \
  SHA256SUMS
```

Replace `<version>` with the version you downloaded; the certificate is bound
to that release's workflow run, so a signature from another release does not
verify.

### 3. Extract and run

The archive expands into a directory named after the asset:

```bash
tar -xzf configflux-vX.Y.Z-x86_64-linux.tar.gz
cd configflux-vX.Y.Z-x86_64-linux
./cfx --version
```

It contains four executables — `cfx` (the one-shot resolver), `compiler`,
`interpreter`, and `runtime` — alongside the license files. Put them on your
`PATH` to run them from anywhere:

```bash
export PATH="$PWD:$PATH"
```

From here the rest of this README applies unchanged, starting with
[Resolve your first configuration](#resolve-your-first-configuration). One
naming note: the compiler ships as `compiler` in the tarball, while the
source-build walkthrough below exposes it as `configflux-compiler`.

## Quick Start


**Prerequisites:** Bazel (via Bazelisk), a C++20 toolchain (Clang/Clang++), and
Git. The Rust toolchain is fetched hermetically by Bazel.

Authoring your own model additionally requires the pinned `cue` binary
(**version 0.16.1**), which exports your CUE chunks to the JSON the compiler
ingests. The quick-start below does not need it: it runs against the committed
exported JSON under `compiler/scenarios/`, which is a generated artifact checked
into the repository.

```bash
# clone
git clone https://github.com/configflux/configflux.git
cd configflux

# build everything
bazel build //...

# run the full test suite
bazel test //...
```

### Resolve your first configuration

The two commands below are `cfx` and `configflux-compiler`. After the build,
expose them under those names and run everything from the repository root:

```bash
mkdir -p .cfx-bin
ln -sf "$PWD/bazel-bin/compiler/compiler" .cfx-bin/configflux-compiler
ln -sf "$PWD/bazel-bin/cfx/cfx"           .cfx-bin/cfx
export PATH="$PWD/.cfx-bin:$PATH"
```

Compile the S1 water-pump reference model into a Compiled Model Package, then
resolve a concrete configuration for one selection. `cfx resolve` runs
open → select → resolve → export in a single command and prints the hash
lineage that makes the result reproducible:

```console
$ configflux-compiler compile --source compiler/scenarios/s1_water_pump/smoke/cue/00_definitions.json --source compiler/scenarios/s1_water_pump/smoke/cue/10_components.json --out build > /dev/null
$ cfx resolve --model build/cmp.manifest.json --select cooling_brand=hydra --select cooling_model=x200 --select pump_type=dual --select region=eu --out snapshot
model_hash: f8a6f2fbc454152033f8b41def9250b3ae1d5cc3fdb7d17938e59ff034e284b7
selection_state_hash: d8412156643c000a9c90e4a04fb5193f6217302a5cd55ed5bde5c5dda3f43d8c
resolve_hash: 8eda451da88c4cd7f74cbc80c6aaa1737f4a19bcab446d5fe01d93891c109187
resolved_output_hash: c2a0142a7698923fdcb3912ae36ca44a9037eaff90b8546c801db7ee8eeb9968
wrote: generated/config.hpp
wrote: generated/config_artifact_manifest.json
wrote: generated/config_build_flags.cmake
wrote: resolve_result.all.hydra-x200-dual-eu.json
```

The resolved snapshot — the JSON a service reads at startup — is written as
`snapshot/resolve_result.all.hydra-x200-dual-eu.json`, and the C++ early-binding
files land under `snapshot/generated/`. Running the same commands on the same
source always produces the same hashes and the same bytes.

A full end-to-end worked example, from CUE chunks to a resolved runtime
snapshot, is in [`docs/canonical-worked-example.md`](docs/canonical-worked-example.md).

Runnable examples are in [`examples/`](examples/). If you configure services
across environments, start with
[`00-service-multi-env`](examples/00-service-multi-env/) — the same pipeline on
a non-robotics domain: one web service resolved across dev, staging, and prod
with `cfx`, including a compiled policy (no debug logging in prod) that
`cfx explain` narrates.

### Explore the results visually

The repository ships a static model explorer — a local, read-only web UI for
the JSON artifacts the pipeline emits. Open `explorer/index.html` directly in
Firefox or Safari, or serve the folder with any static file server (Chrome
restricts `file://` module loading):

```bash
cd explorer && python3 -m http.server 8000
```

Load an artifact with the file picker or drag-and-drop: a model summary or
`cfx options --format json` output in the Model view, a `cfx resolve --format
json` snapshot in the Resolution view, or a `cfx explain --format json` report
in the Explain view. Ready-to-load samples live in `explorer/fixtures/`; see
`explorer/README.md` for details.

## Project Layout

```
cfx/            One-shot resolver CLI: open -> select -> resolve -> export in one command
compiler/       Rust compiler: ingest, merge, validate, emit a Compiled Model Package (CMP)
interpreter/    Rust interpreter: late-binding selection, resolution, export
runtime/        Rust runtime: deterministic CLI over resolved snapshots and overlays
sdk/cpp/        First-party C++ SDK over the runtime C ABI
sdk/ros2/       ROS 2 adapters and Bazel/colcon integration
tools/          Repository tooling: build helpers, release notes, colcon integration
docs/           Architecture, specifications, and operator runbooks
compiler/scenarios/
                Reference scenario packs (S1 water pump, S2 wind turbine,
                S3 automation cell, S4 mobile robot, S5 building HVAC)
```


## Where to Find More

- [`docs/design.md`](docs/design.md) — architecture and semantics.
- [`docs/model-spec.md`](docs/model-spec.md) — canonical end-to-end model
  specification across compiler, interpreter, and runtime.
- [`docs/interface-contracts.md`](docs/interface-contracts.md) — cross-
  application interface contracts and scale constraints.
- [`docs/diagnostics.md`](docs/diagnostics.md) — every diagnostic code the
  compiler, interpreter, and runtime can emit, with its cause and remedy.
- [`docs/comparisons.md`](docs/comparisons.md) — how ConfigFlux relates to
  Helm and Kustomize, CUE, Nix, and classical SPLE tools.
- [`docs/faq.md`](docs/faq.md) — common evaluator questions: determinism,
  tested scale, CUE, service integration, and license.
- [`docs/glossary.md`](docs/glossary.md) — the vocabulary used across
  ConfigFlux and its documentation.
- [`explorer/README.md`](explorer/README.md) — the local model-explorer UI:
  what each view loads, sample fixtures, schema compatibility.
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — how to report bugs, request features,
  and build from source.
- [`SECURITY.md`](SECURITY.md) — security policy and disclosure process.

## License

ConfigFlux is dual-licensed. The source code in this repository is
distributed under the [Business Source License 1.1](LICENSE) (BUSL-1.1), a
source-available license: you may read, modify, redistribute, and use
ConfigFlux — including in production — as long as your use does not include
offering ConfigFlux to third parties as a competitive hosted or embedded
service. Each released version automatically converts to the Apache License,
Version 2.0 four years after its first public distribution.

For uses outside the BUSL-1.1 grant, commercial licenses are available — see
[`LICENSING.md`](LICENSING.md). See [`LICENSE`](LICENSE) for the full text
and [`NOTICE`](NOTICE) for attribution notices.
