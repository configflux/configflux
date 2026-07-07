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
selection assigns at most one option to each facet it constrains. See also:
[option](#option), [selection](#selection).

## option

An option is one allowed value for a facet. The set of options a facet declares
is its full vocabulary; the set a user may actually pick narrows as choices are
made, because only options that remain consistent with every constraint are
selectable. Options that a prior choice has ruled out are reported as pruned,
not offered as valid. See also: [facet](#facet), [selection](#selection).

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
