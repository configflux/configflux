# ConfigFlux Canonical Worked Example

This walkthrough takes the S1 water-pump reference model from CUE-authored
source chunks all the way to a resolved, byte-reproducible configuration,
using two commands: the ConfigFlux compiler and the `cfx` one-shot resolver.

**This document is a verified transcript.** Every command below is shown as a
`$ ` line inside a `console` block, followed by its exact, complete output. The
transcript is replayed automatically against the real binaries, so it can never
drift from what the tools actually print. If you run the same commands on the
same source, you will get the same bytes and the same hashes — that
reproducibility is the whole point.

## 1) Build the binaries and put them on your PATH

The commands in this walkthrough are `cfx` and `configflux-compiler`. Build
them with Bazel, then expose them under those names:

```bash
bazel build //compiler:compiler //cfx:cfx

mkdir -p .cfx-bin
ln -sf "$PWD/bazel-bin/compiler/compiler" .cfx-bin/configflux-compiler
ln -sf "$PWD/bazel-bin/cfx/cfx"           .cfx-bin/cfx
export PATH="$PWD/.cfx-bin:$PATH"
```

The reference source lives in the repository at:

- `compiler/scenarios/s1_water_pump/smoke/cue/00_definitions.json`
- `compiler/scenarios/s1_water_pump/smoke/cue/10_components.json`

These are the JSON siblings the compiler ingests; they are emitted from the
`*.cue` files next to them. Run every command below from the repository root.

## 2) Compile and verify the 150% model

Compile the two source chunks into a Compiled Model Package (CMP). The compiler
ingests each `--source` file, merges them into the 150% model, verifies the
graph, and writes the package under `--out`. It prints the compile result,
including the `model_hash` and a verify report with zero errors:

```console
$ configflux-compiler compile --source compiler/scenarios/s1_water_pump/smoke/cue/00_definitions.json --source compiler/scenarios/s1_water_pump/smoke/cue/10_components.json --out build
{
  "schema_version": 2,
  "status": "ok",
  "model_hash": "d617d8d6ebbc1f1b9c6be6cc791a1d590720d96a881e3f594dc9ca8f5bb39261",
  "compiled_model_package_ref": "build/cmp.manifest.json",
  "stats": {
    "source_count": 2,
    "chunk_count": 2,
    "definition_count": 3,
    "component_count": 3,
    "artifact_count": 3
  },
  "verify_report": {
    "schema_version": 2,
    "model_hash": "d617d8d6ebbc1f1b9c6be6cc791a1d590720d96a881e3f594dc9ca8f5bb39261",
    "status": "ok",
    "error_count": 0,
    "warning_count": 0,
    "checks": [
      {
        "check_id": "graph_integrity",
        "status": "pass",
        "summary": "All references and dependency constraints verified",
        "diagnostic_codes": []
      }
    ],
    "diagnostics": {
      "schema_version": 2,
      "diagnostics": [],
      "error_count": 0,
      "warning_count": 0
    }
  },
  "tool_version": "0.1.0"
}
```

The `model_hash` is a fingerprint of the whole 150% model. Recompiling the same
sources always yields the same hash — that is what makes downstream resolution
auditable.

## 3) Explore the available facets

The compiled model exposes a set of facets, each with a set of valid options.
`cfx options` lists them. With no choices applied, every facet is `[open]`:

```console
$ cfx options --model build/cmp.manifest.json
facet cooling_brand [open]
  aeroflux
  hydra
facet cooling_model [open]
  a9
  x200
facet pump_type [open]
  dual
facet region [open]
  eu
```

## 4) Apply a choice and see what remains valid

Pass a choice with `--select FACET=OPTION`. The listing marks that facet as
`[selected: ...]` and shows the options still valid for the others — this is
the guided-walk primitive you would drive one step at a time in an interactive
tool:

```console
$ cfx options --model build/cmp.manifest.json --select cooling_brand=hydra
facet cooling_brand [selected: hydra]
  aeroflux
  hydra
facet cooling_model [open]
  a9
  x200
facet pump_type [open]
  dual
facet region [open]
  eu
```

## 5) Resolve the 100% configuration

Once every facet you care about is chosen, `cfx resolve` performs
open → select → resolve → export in a single process. It prints the hash
lineage — `model_hash`, `selection_state_hash`, `resolve_hash` — and the
relative path of each exported file, then writes the resolved snapshot under
`--out`:

```console
$ cfx resolve --model build/cmp.manifest.json --select cooling_brand=hydra --select cooling_model=x200 --select pump_type=dual --select region=eu --out snapshot
model_hash: d617d8d6ebbc1f1b9c6be6cc791a1d590720d96a881e3f594dc9ca8f5bb39261
selection_state_hash: 07aec27325650f35fab59a7a558241e3b3ba4aa69c11a04e6b8cbd323f4df1a4
resolve_hash: 2954bc133248f69a4fb2157d03bc70db91a322ded63886094a09483a32466ea0
wrote: generated/config.hpp
wrote: generated/config_artifact_manifest.json
wrote: generated/config_build_flags.cmake
```

The `resolve_hash` is the fingerprint of this exact 100% configuration. The
same model and the same selection always produce the same `resolve_hash` and
the same exported bytes.

## 6) Inspect the resolved snapshot

The exported snapshot is the resolved configuration a build or runtime consumes.
For the C++ early-binding profile, the resolved parameters are emitted as a
header:

```console
$ cat snapshot/generated/config.hpp
#pragma once

namespace configflux::buildcfg {
inline constexpr const char* kThermalControlControlDriver = "hydra_x200_dual_driver";
}  // namespace configflux::buildcfg
```

Alongside it, `snapshot/generated/config_artifact_manifest.json` records the
resolved artifact ids and their bound parameter paths, and
`snapshot/generated/config_build_flags.cmake` carries the same values as CMake
definitions. Every file is byte-stable across runs.

## 7) Hash lineage

The lineage that ties this walkthrough together:

| Stage | Field | Value (S1 reference) |
|-------|-------|----------------------|
| Compile | `model_hash` | `d617d8d6…39261` |
| Selection | `selection_state_hash` | `07aec273…f1a4` |
| Resolve / export | `resolve_hash` | `2954bc13…6ea0` |

Because each hash is derived deterministically from its inputs, any change to
the source model, the selection, or the resolution logic changes the hash
downstream of it — making drift visible rather than silent.

## Machine and agent integration

`cfx` is the human-facing, one-shot entry point. Systems that drive selection
programmatically — a deployment agent, a service that resolves configuration on
demand — use the request/response envelope protocol described in
[`docs/service-integration-guide.md`](service-integration-guide.md) and
[`docs/interpreter-cli-contract.md`](interpreter-cli-contract.md). Both paths
resolve the same model to the same bytes.

## Related

- [`examples/`](../examples/) — runnable progressive examples
- [`docs/non-robotics-worked-example.md`](non-robotics-worked-example.md) — the
  same flow on the building-HVAC pack
- [`docs/interface-contracts.md`](interface-contracts.md) — cross-application
  interface contracts and scale constraints
- [`docs/model-spec.md`](model-spec.md) — canonical end-to-end model
  specification
