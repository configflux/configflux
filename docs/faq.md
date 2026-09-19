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

## Can my model's chunks live in several repositories?

Yes, and they can be *built* separately too.

What you own, and what you build, is a **unit**: the chunks that declare the
same `package` value. A folder in a monorepo is a unit; so is a whole
repository. A shared catalogue can be owned by one team in one repository while
each service that uses it lives in its own.

There are two ways to build such a model, and they produce the same package.

**One command, one checkout.** `compile` takes each repository's exported chunk
as a `--source`, groups the chunks by `package` into units, and links them. This
is the shortcut, and it is the right one whenever a single checkout holds
everything.

**One unit at a time.** `compile-object` compiles one unit — and nothing else —
into a content-addressed object, against the *headers* of the interface objects
it depends on. `link` then assembles objects into the package. This is what four
separate checkouts on four machines can actually do, and it is the form that
gives a mistake between units a name:

- `E_LINK_UNRESOLVED_IMPORT` — a unit needs something no linked object declares.
- `E_LINK_DUPLICATE_ID` — two units export the same id, naming both.
- `E_LINK_INTERFACE_MISMATCH` — a unit was compiled against a different version
  of an interface than the one being linked.

Those checks run over headers alone, before a single chunk file is opened, and
nothing is written unless they all pass. A **lockfile** pins, per unit, the
object hash an integration expects, so a build can also be refused for being the
wrong *set*.

Three things are worth knowing either way:

- **Hashes agree across checkouts.** Model identity is content-canonical, so the
  same chunks compiled from a git worktree, a CI checkout, or a vendored copy
  produce the same `model_hash`, and an object's hash is likewise invariant
  under its source path. Without that, every checkout would be a different model
  and no hash could be compared between machines.
- **A one-shot compile still needs the dependency closure.** A chunk that
  declares `depends_on` a component elsewhere, or `requires` a binding declared
  elsewhere, cannot be verified alone: `compile` fails with
  `E_UNKNOWN_COMPONENT_DEP` or `E_REQUIRES_INVALID`. Any *dependency-closed*
  subset compiles fine — it is simply a different model, with its own hash. This
  is exactly what `compile-object` relaxes: a unit compiled alone *records* an
  unresolved reference in its header instead of rejecting it, and the link
  settles it.
- **Ids are global to the linked set.** Two chunks declaring the same component,
  definition, facet, or constraint id is an error, not a merge.

ConfigFlux does not fetch or vendor anything for you, and the lockfile is
checked rather than fetched from: you check the repositories out however you
already do. `examples/06-catalogue-polyrepo` is a runnable worked example of
both forms over one model, and `examples/export_pack.sh` exports a pack of any
size — including chunks in several repositories inheriting from one shared
definitions chunk. See [getting started](getting-started-new-domain.md)
section 8 for the full rules.

## Does compiling units separately reduce memory?

It bounds the memory of the **compile**, not of everything downstream.

`compile-object` reads one unit's chunks and the *headers* of the interfaces it
was given. An interface's chunk files are never opened, so building a service
does not pull the parameters of every other unit into memory — the working set
is that unit plus a summary per neighbour. `link` then runs its whole cross-unit
graph check over headers alone, which is proportional to the number of declared
ids rather than to parameter volume. Building unit by unit also means an
unchanged unit is not rebuilt at all: its object is reused byte for byte.

Resolution is a different question, and the honest answer is that it is
unchanged. Resolving loads the linked package and resolves the closure you
asked for. Scope `component:<id>` and scope `all` load the same set; the scope
decides what comes out, not what goes in.

**The decision diagram is global, and separate compilation does not shrink it.**
Its size is set by the model's facets and clauses, not by how the model was
built or which scope is being resolved, and every constraint is pack-global and
must be evaluated. So a model with many parameters and few facets links and
resolves in a small working set, while a model with a large, densely
cross-constrained variability structure is bounded by that structure whichever
way you build it.

Loading only a scope's closure at resolve time is a possible future change, not
a shipped one. It is deliberately not treated as free: which chunk declares
which constraint is not recorded in the package index today, so a loader that
skipped chunks could silently skip policy.

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

## How do I see which deployments a model change affects?

Run `cfx diff --base <old cmp.manifest.json> --head <new cmp.manifest.json>
--manifest environments.json`. It resolves every deployment target the manifest
names against both compiled models — asking each side the identical question —
and reports, per target, whether the delivered configuration is unchanged,
changed, newly unsatisfiable, or newly satisfiable. Changed targets list each
parameter that moved with its before and after value. This is a different
question from "did the model change": every edit rotates the model's identity
hash, but most edits leave what a given service actually receives untouched, and
`cfx diff` compares the delivered payload rather than the identity. It writes no
files and exits `0` only when no target changed, so the bare command works as a
pull-request check that fails exactly when a change would alter a running
service's configuration. See the
[service integration guide](service-integration-guide.md#reviewing-a-change-before-it-ships).

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

Yes. A facet's domain can be declared first-class (a `#Facet` with `values`, an
optional `default`, and open/closed — see
[getting started](getting-started-new-domain.md) and the
[glossary](glossary.md#facet)), and a resolve with nothing selected still
succeeds.

Four levels decide a facet's value, highest first:

1. an **explicit choice** you made
2. a **context tag** the environment pinned
3. an **implied** value, when the model's constraints leave exactly one
   admissible value for a facet you did not state
4. the facet's **declared default**

The third level is what makes a default mean "the model has no opinion here".
If your model says factory B only stocks the `c2` container, then selecting
`site=factory_b` and nothing else resolves with `container=c2` — the default
`c1` never applies, because the constraints had already decided. If more than
one value remains admissible, nothing is inferred and the default applies as
before.

The resolved output records where each value came from: `implied_choices` for
values the constraints decided, `defaulted_choices` for values nothing decided.
A facet appears in at most one of the two. `cfx options` surfaces the default
arm, and `cfx resolve` prints `implied:` lines before `defaulted:` ones.

A facet declared with **no** default, that nothing implies, and that an active
condition needs is reported precisely (`E_RESOLVE_FACET_UNBOUND`, naming the
facet and its domain) rather than as a generic "unsatisfiable". The same code
and the same message carry the case where a component *requires* the binding:
the message additionally lists every `<component>.<slot>` that was waiting for
it, so one undecided choice is reported once rather than site by site.

## How does the site decide the container?

By inference. If the constraints admit exactly one container for the site you
chose, `resolve` binds it for you and records it under `implied_choices` — you
never select it. An environment states only its free decisions.

So if your model declares a `site` facet and a `line_container` binding whose
`derive` table maps one to the other, selecting `site=factory_b` alone resolves
when `line_container` is the only undecided choice, with `line_container=c2`
inferred — the same answer `cfx options` would have given you. A pair of plain
facets tied together by a constraint behaves identically; the derive table is
sugar for exactly that constraint.
[`examples/06-catalogue-polyrepo`](../examples/06-catalogue-polyrepo/) shows
the inference end to end. It also shows what happens when that qualifier does
not hold: the same model carries `sorter_container`, a binding with neither a
`derive` table nor a default, and `sorter_service` requires it — so with the
sorter repository in the model, selecting `site=factory_b` alone exits `2`
with `E_RESOLVE_FACET_UNBOUND`. That is why the example's own `run.sh` selects
`sorter_container` alongside the site (and `sorter_lanes`, whose `narrow`
default `factory_b` overrides with `wide`), and still gets `line_container=c2`
inferred.

The rule is still doing real work. It **validates** a pair you do state:
selecting `site=factory_b` together with `line_container=c1` still exits `3`
naming the rule it broke, rather than being silently repaired. Inference only
fills a gap you left; it never overrides something you said. And it only fires
when the answer is unique — if two containers remain admissible, nothing is
inferred and the declared default applies.

The solver decides this, not a lookup table, which is why `cfx options`,
`cfx explain` and `cfx resolve` cannot disagree about what a selection entails.

## Can a service say which catalogue entries it supports?

Yes. A component declares what it needs with `requires`, and may narrow the
binding to the entries it `accepts`:

```cue
components: {
    vision_service: {
        requires: {container: {binding: "line_container", accepts: ["c1", "c2"]}}
    }
}
```

An entry outside that list is never offered. `cfx options` stops listing it, and
a forced choice is refused with the requirement's own name:

```console
$ cfx explain --model build/cmp.manifest.json --select line_container=c3
cannot select line_container.c3:
  blocked by requirement vision_service.container: accepts c1, c2
(one minimal explanation; other minimal cores may exist)
```

Leaving `accepts` out means the component takes any entry. When several
components require the same binding, the integrator is offered the intersection
of their lists — which is how the model knows what the whole line can run. If
that intersection is empty the model can never resolve, so the compiler refuses
it at compile time and names every list, rather than leaving you to discover it
as an unsatisfiable selection later.

The same naming applies to a binding's `derive` table: a rejection reports the
rule as "blocked by binding line_container, derived from site", in the words you
wrote, even though you never wrote a constraint.

## Where does a service read the catalogue entry it required?

Out of its own snapshot. A resolved snapshot delivers the entry the requirement
resolved to **inside the requiring component**, at
`components.<service>.requires.<slot>`, with the entry's id and all of its
fields:

```json
"requires": {
  "container": {
    "binding": "line_container",
    "entry": "c1",
    "fields": {"height_mm": 1000, "length_mm": 1200, "width_mm": 800}
  }
}
```

The service never names the catalogue, never names the binding's other
consumers, and does not change when the model is reorganised. The key is
**absent** when a component declares no requirement, so check for it rather than
expecting an empty object. At the runtime the same values are readable at
`component.<service>.requires.<slot>.<field>` and are not writable: they were
decided at resolve time.

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
decides what a user is allowed to pick in the first place. A condition's
right-hand side may also be an **unquoted** facet name rather than a quoted
value, which compares two facets instead of comparing one against a constant —
so `sorter_container == line_container` requires two independently bound
choices to agree within a deployment without merging them into one facet.

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

Yes. The product schema version is `5` in this release, and inputs authored
against version 4 are rejected with `E_UNSUPPORTED_SCHEMA_VERSION` and a message
naming the required version. A compiled model package produced by an earlier
release is likewise rejected on load rather than read under the current shape.
There is no migration tool and no compatibility mode, and that is deliberate: it
guarantees there is no window in which the same bytes mean two different things.
Recompile your sources with this release's compiler, and set `schema_version` to
`5` in any request you send to the loader or interpreter. Every request carries
it as the first field:

```json
{
  "schema_version": 5,
  "cmp_manifest_ref": "build/cmp.manifest.json"
}
```

Recompiling produces new hashes. The resolved snapshot now delivers each
component's required catalogue entries inside that component, which changes the
bytes a consumer receives — and the schema version is part of every hash
pre-image, so `selection_state_hash`, `resolve_hash`, `resolved_output_hash` and
the software BOM hash all move even for a model that declares no requirement.
Any hash pinned in a deployment check or a provenance record has to be
re-recorded after the recompile. `model_hash` moves only if the model's own
content changed.

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
