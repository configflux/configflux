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
familiarity: CUE ([cuelang.org](https://cuelang.org)) is the sole authoring
format, and models are written as CUE chunks. The compiler ingests JSON exported
from those chunks as an internal artifact of the export pipeline, not a format
you hand-write. Consuming a resolved
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

Other failures are reported as a stable diagnostic code — `E_RESOLVE_FACET_UNBOUND`
and its siblings — that does not change when a message is reworded, so it is safe
to branch on and log. [`diagnostics.md`](diagnostics.md) lists every code the
compiler, interpreter, and runtime can emit, with its cause and remedy.

## What is the default value of a facet, and can I resolve without choosing?

Yes, if the facet declares a default. A facet's domain can be declared
first-class (a `#Facet` with `values`, an optional `default`, and open/closed —
see [getting started](getting-started-new-domain.md) and the
[glossary](glossary.md#facet)). When a declared facet is left unbound at resolve
time it **auto-binds to its declared default** (precedence: an explicit choice
wins over a context tag, which wins over the default), so a resolve with nothing
selected still succeeds. The resolved output records which facets took their
default under `defaulted_choices`, and `cfx options` surfaces the default arm.
A facet that is declared with **no** default and that an active condition needs
is reported precisely (`E_RESOLVE_FACET_UNBOUND`, naming the facet and its
domain) rather than as a generic "unsatisfiable".

## How do I express a rule like "no debug logging in production"?

Declare it as a **constraint**. A constraint is a named policy rule over facet
values, written in the same expression grammar conditions use, and declared in a
model's `constraints` namespace next to the facets it talks about:

```cue
constraints: {
    prod_forbids_debug: {
        condition: "environment != 'prod' || log_level != 'debug'"
        doc:       "Debug logging is not permitted in production."
    }
}
```

Giving the rule an id is the point: it is addressable, documented, and does not
have to be smuggled into the model as a component that exists only to carry the
condition string. The rule a constraint expresses is a single sentence — every
declared constraint must hold in every resolved configuration — and it is
deliberately different from what a `condition` on a component or a parameter
override means. A condition is an *inclusion selector*: it decides whether that
component or that value is part of the resolved configuration. A constraint
decides what a user is allowed to pick in the first place.

Constraints are validated at compile time: the expression must parse (an
unparseable constraint is a compile error, never a silently dropped rule), every
facet it names must be declared under `facets`, and every value it names must be
in a closed facet's declared domain. Declaring the facet is required rather than
merely recommended: only a declared facet gets the mutual-exclusion clauses that
let `cfx options` and `cfx select` enforce the rule the same way `cfx resolve`
does, so a constraint over an undeclared facet is refused instead of being
half-enforced. A declared constraint is then compiled into the model, so
`cfx options` stops offering a value that no valid configuration can hold and
`cfx explain` reports the rule when it blocks a selection. See
[glossary](glossary.md#constraint) and
[model-spec.md](model-spec.md) for the full specification.

All three surfaces agree, including the one that produces output. `cfx resolve`
evaluates every declared constraint against the finished assignment — your
choices and context tags, with each unbound facet filled in from its declared
default — and **refuses** a selection that breaks one: exit `3`, naming the
constraint and quoting its condition, and no snapshot is written. That last part
is the point. A policy you can route around by skipping the guided walk and
calling `resolve` directly is not a policy, so a violating configuration never
becomes a file, a hash, or something a service can load. A constraint that no
choice decides — because nothing binds a facet it names — is not a violation:
nothing was chosen, so nothing was broken.

## I have a model compiled with an earlier release — do I need to recompile?

Yes. The product schema version is `4` in this release, and inputs authored
against version 3 are rejected with `E_UNSUPPORTED_SCHEMA_VERSION` and a message
naming the required version. A compiled model package produced by an earlier
release is likewise rejected on load rather than read under the current shape.
There is no migration tool and no compatibility mode, and that is deliberate: it
guarantees there is no window in which the same bytes mean two different things.
Recompile your sources with this release's compiler, and set `schema_version` to
`4` in any request you send to the loader or interpreter.

Recompiling produces new hashes. The `constraints` namespace is part of a
model's content address, so the same sources compile to a different `model_hash`
than they did before — and every hash derived from it (`selection_state_hash`,
`resolve_hash`, and the software BOM hash) changes with it. Any hash pinned in a
deployment check or a provenance record has to be re-recorded after the
recompile.

### The one migration the version check cannot catch for you

Read this before recompiling if any of your sources predate this release.

The version checks described above cover the request you send to the loader and
the compiled model package you load. They do **not** cover your authored
sources: a `.cue` file or an exported `.json` chunk carries no version marker,
so the compiler cannot tell whether a source was written against this release or
an earlier one.

That matters for exactly one shape. In earlier releases a `condition` on a
component was, in effect, also enforced as a rule about the whole
configuration, so it was possible — and common — to express a policy by adding
a component that had a `condition` and nothing else:

```cue
// A policy written the old way. Under this release it no longer enforces
// anything.
components: {
    prod_forbids_debug: {
        type:      "policy_module"
        condition: "environment != 'prod' || log_level != 'debug'"
    }
}
```

A `condition` now means one thing only: it selects whether that component is
part of the resolved configuration. Such a model **recompiles under this
release without an error, and the rule it used to enforce is silently no longer
enforced.** Nothing fails, no diagnostic is emitted, and `options` and `resolve`
will simply start accepting combinations the rule used to forbid.

Migrate every component of that shape to a `constraints:` entry:

```cue
constraints: {
    prod_forbids_debug: {
        condition: "environment != 'prod' || log_level != 'debug'"
        doc:       "Debug logging is not permitted in production."
    }
}
```

The condition text moves across unchanged; delete the component that carried
it. If the facets the rule names were never declared under `facets`, declare
them as part of the move — a constraint asserts over a domain and never creates
one, so a facet that previously existed only because that condition mentioned it
has to be written down. The compiler enforces this: a constraint naming an
undeclared facet fails the compile, pointing at the constraint and the facet, so
a half-finished migration cannot ship a rule the tools would only partly apply.

To find the candidates, review every component that has a `condition` and ask
what it is for. If the condition decides whether the component is included, it
is a selector and is already correct — leave it alone. If it was written to
forbid a combination, it is a policy and must move. Component conditions that
select, and parameter-override conditions that pick a value, both need no
change.

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
