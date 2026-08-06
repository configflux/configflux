# ConfigFlux Examples

Runnable examples that demonstrate ConfigFlux features progressively,
from the simplest possible model to real-world configurations.

## Prerequisites

Build the compiler once before running any example:

```bash
bazel build //compiler
```

Each example has a `run.sh` script that locates the compiler binary
automatically. You can also point to a custom binary:

```bash
export CONFIGFLUX_COMPILER=/path/to/compiler
```

## Examples

| # | Directory | What it shows | CUE lines |
|---|-----------|--------------|------------|
| 0 | [00-service-multi-env](00-service-multi-env/) | **Start here for services.** One web service resolved across three environments (dev/staging/prod) with `cfx`; a compiled policy (no debug logging in prod) shown by `cfx options` + `cfx explain` | ~55 |
| 1 | [01-hello-led](01-hello-led/) | 1 component, 0 selections — bare minimum pipeline | ~18 |
| 2 | [02-sensor-gateway](02-sensor-gateway/) | Selections, overrides, conditional component | ~40 |
| 3 | [03-motor-controller](03-motor-controller/) | Artifact references, 2-step selection, export-resolved, BOM | ~65 |
| 4 | [04-fleet-edge-node](04-fleet-edge-node/) | Full pipeline: compile → interpret → resolve → runtime handoff, 3 selection facets, region-conditional component | ~90 |
| 5 | [05-compose-fleet](05-compose-fleet/) | One model, two named environments × two service scopes; matrix resolve, delivery bundles, a containerized Pattern 1 service, and a standalone debug run | ~140 |

If you configure services across environments, start with
`00-service-multi-env`. To learn the pipeline from the ground up, start with
`01-hello-led` and work through `01`–`05` in order — each builds on concepts
introduced by the previous one.

Example 00 uses the compiler and `cfx` binaries and `python3` for JSON shaping.
Examples 01–03 use only the compiler binary. Example 04 additionally
exercises the interpreter and runtime binaries and uses `jq` between
stages. Example 05 adds named environments, the delivery bundle, and
container consumption; it uses the interpreter, `jq` (via the reference
resolver/verifier), `python3`, and — when available — `docker`.

## Running an example

```bash
cd examples/01-hello-led
./run.sh
```

Each `run.sh` compiles the exported JSON sources, runs verification, and
inspects the resulting model. Outputs land in an `out/` subdirectory.

## Structure

Every example directory contains:

- **`cue/*.cue` source files** — the configuration model, authored in CUE
  (the supported authoring front-end; see [`compiler/cue/`](../compiler/cue/)).
  `inherits` is resolved by CUE during whole-pack evaluation.
- **Exported `*.json`** — the inheritance-resolved chunks emitted from the CUE
  sources (`cue export`); this is what `run.sh` feeds to the compiler, which
  ingests JSON directly. Regenerate with the pinned `cue` binary; CUE owns
  `inherits` resolution, so the compiler no longer fills it at resolve time.
- **`run.sh`** — a self-contained script that runs the full pipeline
- **`README.md`** — what the example demonstrates and what to look at
