# ConfigFlux

[![License: BUSL-1.1](https://img.shields.io/badge/license-BUSL--1.1-blue.svg)](LICENSE)

<!-- evaluator-note: latest=v0.1.0 -->


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

## Quick Start


**Prerequisites:** Bazel (via Bazelisk), a C++20 toolchain (Clang/Clang++), and
Git. The Rust toolchain is fetched hermetically by Bazel.

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
model_hash: d617d8d6ebbc1f1b9c6be6cc791a1d590720d96a881e3f594dc9ca8f5bb39261
selection_state_hash: 07aec27325650f35fab59a7a558241e3b3ba4aa69c11a04e6b8cbd323f4df1a4
resolve_hash: 2954bc133248f69a4fb2157d03bc70db91a322ded63886094a09483a32466ea0
wrote: generated/config.hpp
wrote: generated/config_artifact_manifest.json
wrote: generated/config_build_flags.cmake
```

The resolved configuration lands under `snapshot/generated/`. Running the same
commands on the same source always produces the same hashes and the same bytes.

A full end-to-end worked example, from CUE chunks to a resolved runtime
snapshot, is in [`docs/canonical-worked-example.md`](docs/canonical-worked-example.md).
A non-robotics walkthrough using the building HVAC pack is in
[`docs/non-robotics-worked-example.md`](docs/non-robotics-worked-example.md).

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
- [`docs/comparisons.md`](docs/comparisons.md) — how ConfigFlux relates to
  Helm and Kustomize, CUE, Nix, and classical SPLE tools.
- [`docs/faq.md`](docs/faq.md) — common evaluator questions: determinism,
  tested scale, CUE, service integration, and license.
- [`docs/glossary.md`](docs/glossary.md) — the vocabulary used across
  ConfigFlux and its documentation.
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — how to set up, build, and submit
  changes.
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
