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
| 3 | [03-motor-controller](03-motor-controller/) | Artifact references, 2-step selection, a runtime write rejected by a declared constraint, export-resolved, BOM | ~75 |
| 4 | [04-fleet-edge-node](04-fleet-edge-node/) | Full pipeline: compile → interpret → resolve → runtime handoff, 3 selection facets, region-conditional component | ~90 |
| 5 | [05-compose-fleet](05-compose-fleet/) | One model, two named environments × two service scopes; matrix resolve, delivery bundles, a containerized Pattern 1 service, and a standalone debug run | ~140 |
| 6 | [06-catalogue-polyrepo](06-catalogue-polyrepo/) | **One model, four repositories.** A shared catalogue authored once as typed data, two bindings over it, and three services that declare what they `requires` — one bringing a catalogue of its own; per-site resolution, three named refusals, and the dependency-closure rule | ~290 |

If you configure services across environments, start with
`00-service-multi-env`. If your model is split across repositories, read
`06-catalogue-polyrepo`. To learn the pipeline from the ground up, start with
`01-hello-led` and work through `01`–`05` in order — each builds on concepts
introduced by the previous one.

Example 00 uses the compiler and `cfx` binaries and `python3` for JSON shaping.
Examples 01–02 use only the compiler binary. Example 03 adds `cfx` and the
runtime binary — it resolves the model, then shows one accepted and one refused
runtime write — and uses `jq` to read the refusal back. Example 04 likewise
drives `cfx` and the runtime binary, and uses `jq` between the pipeline
stages. Example 05 adds named environments, the delivery bundle, and
container consumption; it uses the interpreter, `jq` (via the reference
resolver/verifier), `python3`, and — when available — `docker`. Example 06
composes one model from four repository directories and uses the compiler,
`cfx`, and `python3`.

`export_pack.sh` in this directory is the reference CUE exporter: it resolves
`inherits` across any number of chunk files against one shared definitions
chunk, so it covers the single-file, two-file, and multi-repository pack
layouts alike. It regenerates example 06's committed JSON and is documented in
[getting started](../docs/getting-started-new-domain.md) section 9.

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

Example 06 nests that layout one level deeper — `repos/<repository>/cue/*.cue`
with the exported JSON in `repos/<repository>/` — because each directory stands
in for a separate repository.
