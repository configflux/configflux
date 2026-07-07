# ConfigFlux FAQ

Answers to the questions evaluators most often ask about ConfigFlux. Every
performance figure below is drawn from the
[published benchmark](benchmarks/public-benchmark.md); where a limit is
not yet measured, this page says so plainly. Terms in `code font` are defined in
the [glossary](glossary.md).

## Is ConfigFlux deterministic, and how is that verified?

Yes — determinism is the core guarantee. The compiler, interpreter, and runtime
canonicalize and hash their outputs, so the same inputs always produce the same
bytes and the same hashes. This is verified rather than asserted, in three ways.
First, `cfx resolve` prints the `model_hash` → `selection_state_hash` →
`resolve_hash` lineage, so any change to inputs, selection, or output surfaces as
a changed hash. Second, the worked examples in these docs are replayed
automatically against the real binaries in continuous integration, so documented
output cannot drift from actual tool output. Third, each resolved bundle ships
with a snapshot content hash you can recompute locally to confirm the bytes you
received are the bytes that were produced. Together these make a resolved
configuration auditable by hash and traceable to its inputs.

## What does ConfigFlux produce?

The compiler produces a [Compiled Model Package](glossary.md#compiled-model-package-cmp)
(CMP): a validated, hash-addressable model of the whole product family.
Resolving a selection against that model produces a resolved output — the strict
[100% model](glossary.md#100-model) for one product context — together with the
emitted artifacts for that configuration, such as a generated C++ header, an
artifact manifest, and build flags, plus a first-party software bill of
materials (SBOM). Every one of these is canonicalized, byte-stable, and
hash-addressable.

## What scale is tested today?

The [published benchmark](benchmarks/public-benchmark.md) reports
read-side `valid_options` query latency against a model that has already been
compiled and loaded into memory; it excludes the one-time compile and load cost.
On the FAMA reference feature models, that query completes with a 99th-percentile
latency between roughly 2.6 and 47 microseconds. On the SPLOT *eShop* model of
about 290 features, it runs at roughly a 13-millisecond median and a
101-millisecond 99th percentile. Consult the benchmark for the authoritative
figures.

Two honest limits accompany those numbers. Performance on much larger and more
densely constrained models remains an active area of work; the figures describe
the workloads validated for the current public release, not a general scaling
promise. In particular, models at roughly ten-thousand-feature scale that also
carry cross-tree constraints are not yet in budget — the cross-tree-free case at
that scale is tractable, but the densely cross-constrained case is not something
ConfigFlux claims today, and you should not assume interactive explanation at
that scale. Treat the benchmark as the boundary of what is measured.

## Do I need CUE expertise to use ConfigFlux?

It depends on which side you are on. Authoring a *new* model does require CUE
familiarity: CUE ([cuelang.org](https://cuelang.org)) is the sole authoring and
ingestion format, and models are written as CUE chunks. Consuming a resolved
configuration does not: a service reads the resolved JSON snapshot with nothing
more than a JSON library, and needs no ConfigFlux binary embedded in it. A team
can therefore concentrate CUE knowledge in the people who author the model,
while every downstream service simply reads JSON.

## How does a resolved configuration reach my service?

There are three integration patterns, and most services need only the first.

- **Pattern 1 — read the snapshot.** Your service reads the resolved JSON
  snapshot at startup. Nothing from ConfigFlux is embedded in the service; it
  needs only a JSON library. This is the pattern to reach for by default.
- **Pattern 2 — validated session.** Your service runs the `configflux-runtime`
  CLI as a subprocess and talks to it over JSON on stdin and stdout, giving it a
  validated session for reading and mutating runtime-lifecycle parameters.
- **Pattern 3 — in-process C ABI.** Your service links the runtime in-process
  through its C ABI. This is an advanced path and is still under evaluation.

Start with Pattern 1, move to Pattern 2 if you need validated runtime-lifecycle
reads and writes, and consider Pattern 3 only for advanced in-process needs.

## What happens when a selection is invalid?

A contradictory selection is a normal, expected outcome, not a crash. When no
valid product satisfies the choices made, `explain` returns a labeled minimal
unsatisfiable subset — an [unsat core](glossary.md#unsat-core--explain) — that
names the smallest set of conflicting constraints as `{facet}.{option}`
identifiers rather than raw solver indices. The result is a successful response
whose payload is the reason the selection cannot be satisfied, and it is one
minimal witness of the conflict. This lets a caller show precisely which choices
collide. (For the scale at which this holds, see the scale answer above.)

## How does ConfigFlux relate to feature flags?

They solve different problems and can coexist. ConfigFlux binds configuration at
three lifecycle phases: *construction*, where a value is a compile-time constant
frozen into the CMP; *startup*, where a value is read once and then fixed for the
run; and *runtime*, where a value may change while the process runs. Its job is
to resolve build-time and deploy-time structure — which components, parameters,
and artifacts exist for a given variant. A runtime feature-flag system does
something else: it toggles behavior within a build that has already been
deployed. ConfigFlux decides what a variant is; a feature-flag system flips
switches inside one. Teams commonly use both.

## Is ConfigFlux a Kubernetes tool?

No. ConfigFlux is domain-neutral: it models anything expressible as components,
parameters, constraints, and artifacts, and its reference scenario packs span
robotics, industrial automation, and building systems. It can emit its resolved
output into Kubernetes packaging tools — for example a Helm values file or a
Kustomize base — but it neither requires Kubernetes nor targets it specifically.
See [comparisons.md](comparisons.md) for how ConfigFlux sits alongside those
tools.

## What license is ConfigFlux under?

ConfigFlux is dual-licensed: the Business Source License 1.1 (BUSL-1.1), a
source-available license that permits production use so long as you do not
offer ConfigFlux to third parties as a competitive hosted or embedded
service, plus commercial licenses for everything else (see
[`LICENSING.md`](../LICENSING.md)). Each released version converts to the
Apache License, Version 2.0 four years after its first public distribution.
The license text is in [`LICENSE`](../LICENSE) and attribution notices are in
[`NOTICE`](../NOTICE).
