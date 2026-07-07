# ConfigFlux Compared to Adjacent Tools

ConfigFlux is a deterministic configuration compiler for software product line
engineering. It resolves an exact, validated configuration for one product from
a family-wide model and emits a byte-reproducible result. That places it
upstream of, and alongside, several well-known tools rather than in competition
with them.

The comparisons below describe where each tool's job ends and ConfigFlux's
begins. None is a performance claim about another tool, and each pairs "use the
other tool when …" with "use ConfigFlux when …" so the boundary is explicit.
Where the roles overlap, they compose: a ConfigFlux resolved output is an
ordinary file that the other tools can consume. The terms used below —
[150% model](glossary.md#150-model), [100% model](glossary.md#100-model),
[facet](glossary.md#facet), [option](glossary.md#option), and
[unsat core](glossary.md#unsat-core--explain) — are defined in the
[glossary](glossary.md).

## vs Helm and Kustomize

Helm is the de facto package manager for Kubernetes: a chart templates
Kubernetes YAML from a `values.yaml` file, and the rendered manifests are
applied to a cluster, with validation happening downstream at apply time.
Kustomize takes a template-free approach to the same packaging problem,
composing a base set of manifests with overlays and patches; it has no
templating language and no constraint solver. Both answer the question "how do I
package and apply this configuration to Kubernetes."

ConfigFlux answers an earlier question: "which configuration is valid for this
variant, and can I prove it." It decides and proves a selection against the
family model first — resolving the 150% model to a 100% model, and returning a
labeled unsat core when a selection is contradictory — and then emits the result
into whatever comes next, including a Helm values file or a Kustomize base. The
roles are complementary: ConfigFlux chooses and proves; Helm or Kustomize
packages and applies.

**Use Helm or Kustomize when** your task is to template, compose, and apply
Kubernetes manifests. **Use ConfigFlux when** you need to choose and prove a
valid configuration across a product family before rendering, then emit a
byte-reproducible result into those tools.

## vs plain CUE

CUE ([cuelang.org](https://cuelang.org)) is a configuration language that
unifies schema and data and validates configurations through unification and
disjunction. It is a capable constraint language in its own right. ConfigFlux
does not replace CUE — it is authored *in* CUE: models are written as CUE chunks
and ingested through it. What ConfigFlux adds is a variability-resolution layer
on top of CUE: a 150% → 100% selection engine that resolves one product from a
family-wide model and, when a selection cannot be satisfied, returns a labeled
minimal unsat core naming the conflicting `{facet}.{option}` constraints.

**Use CUE on its own when** you need to define and validate a single
configuration through schema-and-data unification. **Use ConfigFlux when** that
configuration is one member of a product family and you need Boolean variability
resolution over the whole family — selecting a valid product, proving it, and
emitting a byte-stable result — while still authoring in CUE.

## vs Nix

Nix is a purely functional package manager and build system that achieves
reproducibility through a content-addressed store and pinned inputs. Its
determinism is genuine, and it is shared ground: ConfigFlux is likewise
deterministic and content-addressed, and the two share the goal of a build that
yields the same result every time. The difference is which problem each solves.
Nix reproduces a configuration you have already specified. ConfigFlux focuses on
the upstream problem — choosing and proving a valid configuration across a
product family's variability — and then emits a reproducible result that a build
system, Nix included, can consume.

**Use Nix when** you need reproducible builds, packages, or environments from a
configuration you have already decided. **Use ConfigFlux when** the open
question is which configuration to build across many variants, and how to prove
it valid, before you hand a resolved result to a build or packaging system.

## vs classical SPLE tools

The classical software-product-line tooling category centers on feature models
that capture variability, configuration of a valid product from those models,
and product derivation, often supported by SAT-based analyses. Representative
tools exist both as academic and open-source projects and as commercial desktop
or IDE suites. FAMA and SPLOT — used as the corpora in the ConfigFlux
[published benchmark](benchmarks/public-benchmark.md) — are established
reference tools and datasets in this space, cited here as standard reference
workloads rather than as competitors.

ConfigFlux implements the same established SPLE concepts — feature-style
variability, selection of a valid product, and derivation — but delivers them as
a deterministic, file-in / file-out CLI compiler. Its outputs are
byte-reproducible and hash-addressable, which suits version-controlled and
GitOps-shaped workflows where configuration is reviewed, diffed, and replayed
like source code, rather than an interactive desktop modeling environment.

**Use a classical SPLE suite when** you want an interactive desktop or IDE
environment for modeling and analyzing variability. **Use ConfigFlux when** you
want the same SPLE concepts as a deterministic CLI compiler whose
byte-reproducible, hash-addressable outputs fit a version-controlled,
GitOps-style pipeline.

## At a glance

The table summarizes where each tool focuses and where ConfigFlux fits
alongside it. Rows are ordered alphabetically.

| Tool | Its focus | Where ConfigFlux fits |
|---|---|---|
| Classical SPLE suites | Interactive modeling, configuration, and analysis of feature models, often in a desktop or IDE | The same SPLE concepts delivered as a deterministic CLI compiler with byte-reproducible outputs |
| CUE | Unifying schema and data and validating a configuration | The authoring language ConfigFlux builds on; ConfigFlux adds variability resolution over a product family |
| Helm and Kustomize | Packaging and applying Kubernetes manifests | Chooses and proves a valid configuration first, then emits into a Helm values file or Kustomize base |
| Nix | Reproducible builds from a specified configuration | Chooses and proves which configuration to build across variants, then emits a reproducible result to consume |
