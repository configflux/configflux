# ConfigFlux Glossary

The vocabulary used across ConfigFlux and its documentation. Terms are grouped
in the order they appear along the pipeline — from the concepts that frame the
problem, through the model layers and the compiled artifacts, to the act of
selecting, resolving, and hashing a configuration — rather than alphabetically,
so that each term builds on the ones above it. Definitions here are consistent
with [`docs/model-spec.md`](model-spec.md), the canonical model specification.

## Software product line (SPL, SPLE)

A software product line is a family of related products built from one shared,
configurable set of assets rather than as separate forks or copies. Software
product line engineering (SPLE) is the discipline and tooling category concerned
with modeling that shared asset base, expressing where products vary, and
deriving individual products from it. ConfigFlux is a deterministic compiler for
this category: it takes the shared, variable definition of a product family and
resolves one concrete product from it.

## 150% model

The 150% model is the aggregate superset of a product family: every component,
option, and constraint across the whole line, merged into a single typed graph.
It is called "150%" because it deliberately holds more than any one product
needs — the union of all variants, not a single configuration. ConfigFlux
ingests decentralized authored chunks into this model and validates it as a
typed graph before anything is resolved. See also: [chunk](#chunk),
[Compiled Model Package (CMP)](#compiled-model-package-cmp).

## 100% model

The 100% model is the resolved subset: the exact, validated configuration for
one concrete product context, derived from the 150% model. Where the 150% model
describes everything a family could be, the 100% model is a single product that
a target can build or run. It is produced by [resolution](#resolution) and is
strict — every value is concrete and typed.

## chunk

A chunk is an authored source unit of the 150% model: a single CUE file that
declares the `configflux` package with one top-level `chunk` value. Splitting a
model into chunks lets a product family be authored across many files and
contributors instead of one monolith; the compiler merges all chunks into the
150% model. Each chunk is hashed as its `chunk_hash`, so a change to any source
unit is visible in that unit's identity. See also: [150% model](#150-model).

## Compiled Model Package (CMP)

The Compiled Model Package is the compiler's output artifact: the validated,
hash-addressable model that the interpreter and runtime consume. It comprises a
manifest (`cmp.manifest.json`), an index (`index.cfir.json`), and one compiled
chunk file per source chunk (`chunk-<hash>.cfir`). The CMP is identified by its
`model_hash`; because it is content-addressed, two builds of the same sources
produce the same package. See also: [Compiled Constraint Model
(CCM)](#compiled-constraint-model-ccm),
[resolve_hash lineage](#resolve_hash-lineage).

## Compiled Constraint Model (CCM)

The Compiled Constraint Model is the compiled solver form emitted alongside the
CMP, in the package's `ccm/` directory. It holds a manifest, a symbol table that
maps human-readable facet and option names to solver variables, and a reduced
ordered binary decision diagram that encodes the model's constraints. The solver
queries the CCM at selection and resolution time to answer — quickly and
exactly — which options remain valid and whether a selection is satisfiable. See
also: [facet](#facet), [option](#option),
[unsat core / explain](#unsat-core--explain).

## facet

A facet is a named dimension of variation — a selection axis with a set of
allowed options. Wherever a product family varies, each independent choice is
modeled as a facet (for example, a cooling brand, a region, or a pump type). A
selection assigns at most one option to each facet it constrains.

A facet's domain can be **declared** as a first-class construct (ADR-0047): an
authored `#Facet` names the facet's ordered `values`, an optional `default`
arm, and whether the domain is `open` (extensible) or closed (exhaustive; the
default). A declared closed facet's values are its complete vocabulary — every
value is emitted into the option universe, including a default arm that no
condition happens to name. Where a facet is **not** declared, its domain is
still inferred from the values conditions compare it against, exactly as
before, so declaration is opt-in per facet. See also:
[option](#option), [selection](#selection), [defaulted_choices](#defaulted_choices).

## facet binding (parameter)

A facet binding is a parameter's declaration that it **is** a facet's runtime
handle: `facet: <name>` on the parameter (ADR-0064). The compiler checks the
declaration — the facet is declared, the parameter's effective type is `string`,
it authors no `value` of its own, and at most one parameter in the whole model
binds a given facet — and resolution sets the parameter's value to that facet's
effective value (explicit choice, else implied choice, else declared default).
The binding travels inside the resolved output, so a runtime enforces a write
against the declared constraints of the facet the parameter declares.

A parameter that merely shares a facet's name is **not** its handle. That
distinction is the point: the mapping the runtime previously derived from the
last path segment was many-to-one, so two parameters could land on one facet and
disagree, and the facet was then dropped from the assignment entirely — a
declared constraint silently not enforced. One handle per facet makes that
collision impossible rather than contained. The runtime reads the binding out
of the resolved output to decide which writes it enforces, so enforcement is
either on (the parameter declares a facet) or unambiguously off; see
`docs/runtime-v2-contract.md` §6.12 and `docs/runtime-c-abi.md` §4.3. See also:
[facet](#facet), [option](#option), [resolution](#resolution),
[implied choice](#implied-choice).

## defaulted_choices

`defaulted_choices` is resolve-time provenance on a resolved output: the map of
declared facets whose resolved value came from the facet's declared `default`
arm rather than from an explicit choice or a context tag (precedence: explicit
choice > context tag > declared default). It is how a declared default behaves
like a default — an unbound declared facet auto-binds to its default at resolve
time, and `defaulted_choices` records exactly which facets that happened to.
It folds into the `resolve_hash` (skip-if-empty, so a facet-free or
nothing-defaulted resolve is byte-unchanged) and is deliberately absent from
`selection_state_hash`, which stays pure user input. See also:
[facet](#facet), [selection](#selection),
[implied choice](#implied-choice),
[closed_facet_domains](#closed_facet_domains).

## implied choice

An implied choice is a facet value nobody selected that the model's constraints
nevertheless entail: the solver found exactly one admissible value for a facet
the selection left unbound, and bound it (ADR-0057). Four levels decide a
facet's value, highest first — an explicit **choice**, a **context tag**, an
**implied** value, then the facet's declared **default** — so inference only
ever fills a gap you left, and never overrides something you said. It fires only
when the answer is unique; if two values remain admissible, nothing is implied
and the default applies.

The resolved output records the provenance in `implied_choices`, the sibling of
[defaulted_choices](#defaulted_choices): a facet appears in at most one of the
two, and `cfx resolve` prints its `implied:` lines before its `defaulted:` ones.
Like `defaulted_choices` it folds into the `resolve_hash` skip-if-empty and is
absent from `selection_state_hash`, which stays pure user input. This is what
lets an environment carry only its free decisions — a deployment that names its
site need not repeat the container that site is equipped for. See also:
[defaulted_choices](#defaulted_choices), [binding](#binding),
[constraint](#constraint), [selection](#selection).

## closed_facet_domains

`closed_facet_domains` is the map, recorded on a resolved output, of every
**closed** facet to the values it declares. A closed facet is exhaustive — its
declared values are all the values it can take — and an open one is not, so open
facets are omitted from the map rather than carried with a flag: nothing may be
entailed about a domain that is still extensible.

It exists because a deployed runtime holds a compiled model and a resolve result
but never the model sources the facet declarations live in, and exhaustiveness
is what lets an explanation reason backwards: if a conflict says a two-valued
closed facet is *not* one of its values, it must be the other, and the
constraint that value breaks can be named. Without it, the same conflict is
reported only as "the model is over-constrained here". Carrying it on the
resolved output is therefore the channel by which a device-side rejection can
say which policy was violated.

Unlike [defaulted_choices](#defaulted_choices) it is **not** part of the
`resolve_hash`: it projects static declarations the `model_hash` already covers
rather than provenance about one resolve's selection, so it is a sibling of the
lineage hashes and a member of none of them. Skip-if-empty, so a model without
closed facets is byte-unchanged. See also: [facet](#facet),
[constraint](#constraint), [defaulted_choices](#defaulted_choices).

## option

An option is one allowed value for a facet. The set of options a facet declares
is its full vocabulary; the set a user may actually pick narrows as choices are
made, because only options that remain consistent with every constraint are
selectable. Options that a prior choice has ruled out are reported as pruned,
not offered as valid. See also: [facet](#facet), [selection](#selection).

## constraint

A constraint is a named policy rule over facet values — "debug logging is not
permitted in production" — declared in a model's `constraints` namespace with an
id, a Boolean expression in the condition grammar, and an optional doc string
(ADR-0054). Its rule is one sentence: every declared constraint must hold in
every resolved configuration. A constraint is deliberately distinct from
a `condition` on a component, parameter, or override: a condition is an
*inclusion selector* that decides what a resolved configuration contains, while
a constraint decides what a user is allowed to pick. Constraints are validated
at compile time — the expression must parse, every facet it names must be
declared under `facets`, and every value it names must be in a closed facet's
domain — and are compiled
into the CCM as the model's only authored root conjuncts, so a declared
constraint changes which options are offered and which selections can be
explained.
Elsewhere in this glossary "constraint" also appears in the general sense of any
rule the compiled model screens against. See also: [facet](#facet),
[option](#option), [unsat core / explain](#unsat-core--explain).

## unit

A unit is a directory of chunks that share one `package` value and are authored
and exported together (ADR-0057). A folder inside a monorepo is a unit; a
repository may hold one unit or several. A service is usually one unit, a shared
catalogue is one unit, and the integration that binds everything is one unit.
The `package` field every chunk already carries is the unit's name, so grouping
by that value is the whole definition. A unit is also the granularity at which a
model is built: one unit compiles into one object. See also: [chunk](#chunk),
[object](#object), [catalogue](#catalogue).

## object

An object is one unit, compiled (ADR-0058). It is a directory holding the unit's
chunk files, a deterministic provenance sidecar, and `object.json` — the
**header**, which carries what the unit exports, what it still needs from
elsewhere, the clauses it contributes to the constraint model, and the
`object_hash` of every interface it was compiled against. The `object_hash` is
the hash of the header, so an object's identity is content-addressed and
invariant under source path and argument order, exactly as `model_hash` is. An
object holds no constraint model and no package index; those are products of the
link. `compile-object` writes one. See also: [unit](#unit),
[interface](#interface), [link](#link).

## interface

An interface is an object read for its header alone (ADR-0058). Compiling a unit
with `--interface <other>.cfo` lets the compiler check this unit's references
against what that object exports, without ever opening its chunk files. An
**interface unit** is the natural companion: a unit that declares definitions,
facets, catalogues, bindings and constraints and few or no components — the
header file of a model, which service units are compiled against and inherit
from. A reference that no interface supplies is not an error at object time; it
is recorded in the header's imports and settled at link time. See also:
[object](#object), [link](#link).

## link

Link is the step that turns a set of objects into a Compiled Model Package. It
checks the whole cross-unit graph from the headers first — unit names unique,
exported ids unique across units, every import declared by some object, every
interface hash matching the object actually linked — then builds the constraint
model from those headers, then copies each chunk file after verifying it hashes
to its own name. Nothing is written unless every check passes, and the faults it
names carry the `E_LINK_*` codes. The one-shot `compile` is this same linker fed
by objects it builds in memory and never writes, so the two forms produce
byte-identical packages. See also: [object](#object), [lock](#lock),
[Compiled Model Package (CMP)](#compiled-model-package-cmp).

## lock

A lock is a file that pins, per unit, the `object_hash` an integration expects
(ADR-0058). `link --write-lock` writes it from a link; `link --lock` refuses a
later link whose objects do not match those pins, naming the unit and both
hashes. The pins are **checked, never fetched**: nothing downloads an object,
and the entry's `source` field is a free-text note that no code path reads.
Bringing objects to the linker stays the job of a checkout, a submodule, an
artifact store or a CI job. See also: [link](#link), [object](#object).

## catalogue

A catalogue is a typed table of named entries declared in a model's
`catalogues` namespace: a set of fields with declared types, and entries that
each supply every field with a value of its type (ADR-0057). It is where a
shared *thing* lives — the physical containers a plant uses, the firmware modes
a device supports — as opposed to a definition, which describes a parameter's
shape and deliberately carries no value. Any unit may declare a catalogue, and
exactly one chunk declares each. The compiler checks that every entry supplies
exactly the declared fields with values of the declared types, so a mistyped
table is a compile error rather than a surprise at the service that reads it.
See also: [binding](#binding), [unit](#unit).

## binding

A binding is one shared choice of a catalogue entry, declared in a model's
`bindings` namespace by naming the catalogue it draws from (ADR-0057).
Semantically a binding *is* a declared closed facet whose options are that
catalogue's entry ids, and it shares the facet id space, so nothing downstream
treats it specially: it is chosen per deployment, listed by `cfx options`,
referenced by constraints, and explained like any other facet. Two components
that name the same binding receive the same entry by construction — that is how
an author says "these must match". A binding may declare a default entry, or a
`derive` table that fixes the entry from another declared facet such as the
deployment site, and never both. See also: [catalogue](#catalogue),
[facet](#facet), [option](#option), [requirement](#requirement).

## requirement

A requirement is a component's declared need for a binding, written in the
component's `requires` map under a slot name it chooses (ADR-0057). It is the
only way a component receives a catalogue entry: resolve delivers the entry the
binding took *inside* the requiring component, at
`components.<component>.requires.<slot>`, with the entry's id and all of its
fields — so a service reads its own configuration and never names the catalogue
or the binding's other consumers.

A requirement is deliberately **not** a `depends_on` edge. It names a shared
choice rather than another component, so it joins no dependency closure and
constrains no build order; scoping a resolve to one service still hands that
service its entry. Two components requiring the same binding receive the same
entry by construction, which is how an author says "these must match". A
requirement may narrow the binding to the entries the component `accepts`, and
the integrator is then offered the intersection of every list naming that
binding; an empty intersection is refused at compile time rather than
discovered later as an unsatisfiable selection. A requirement naming a binding
the compile set does not declare is `E_REQUIRES_INVALID`. See also:
[binding](#binding), [catalogue](#catalogue), [chunk](#chunk).

## selection

A selection is the set of choices a user has made over facets. It is
canonicalized as a stateless `SelectionState` payload — a plain description of
which option is chosen for which facet, carrying no hidden session state — and
hashed as its `selection_state_hash`. Because the selection is stateless and
hashed, the same choices always describe the same selection, independent of how
they were entered. See also: [facet](#facet),
[resolve_hash lineage](#resolve_hash-lineage).

## resolution

Resolution is the act of pruning the 150% model down to the strict 100% output
for one selection context. Given a validated model and a selection, the resolver
removes everything the selection excludes and produces a resolved output whose
values are all concrete. The resolved output is hashed as its `resolve_hash`. In
the `cfx` CLI, resolution is the third step of the open → select → resolve →
export pipeline. See also: [100% model](#100-model),
[resolve_hash lineage](#resolve_hash-lineage).

## unsat core / explain

When a selection contradicts the model — no valid product satisfies it —
`explain` returns a labeled minimal unsatisfiable subset, or unsat core: the
smallest set of conflicting constraints that together make the selection
impossible, named by labeled `{facet}.{option}` identifiers rather than raw
solver variable indices. A rejection of this kind is not an error; `explain` is
a normal, successful result whose payload is the reason. The returned core is
one minimal witness of the conflict, not necessarily the only one. See also:
[option](#option), [selection](#selection).

## resolve_hash lineage

The resolve_hash lineage is the deterministic chain `model_hash` →
`selection_state_hash` → `resolve_hash`. Each hash is derived from its inputs:
the model hash from the compiled model, the selection hash from the model hash
plus the choices, and the resolve hash from the selection context plus the
resolved output. Because every downstream hash depends on its upstream inputs,
any change anywhere in the chain changes the hashes below it — so configuration
drift is visible as a hash mismatch rather than a silent difference. See also:
[Compiled Model Package (CMP)](#compiled-model-package-cmp),
[resolution](#resolution).
