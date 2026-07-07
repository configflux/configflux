# ConfigFlux — Solver Performance

This page reports the measured read-side query latency of the ConfigFlux
solver on two public feature-model benchmarks: the FAMA feature-model
corpus and the named SPLOT *eShop* model. The numbers below are what an
application embedding the solver can expect when asking "given the choices a
user has made so far, what can they still pick?"

## What the solver does

ConfigFlux compiles a configuration model into a compact binary decision
diagram (a Reduced Ordered Binary Decision Diagram, or ROBDD) ahead of time.
Compilation happens once. At runtime the solver loads that compiled model and
answers configuration queries against it:

- **`valid_options(facet)`** returns the set of values for a given feature
  ("facet") that are still consistent with every selection made so far. As the
  user makes choices, the set of still-valid options narrows. This is the call
  that powers a live configurator UI — for example, greying out the options
  that a previous choice has ruled out.
- **`apply(selection)`** records a choice onto the current session, so that
  subsequent `valid_options` queries reflect it.

Both operations run over the already-compiled model. They do not re-solve the
problem from scratch on each call, which is what makes interactive latency
possible on non-trivial models.

ConfigFlux ships with a decision-diagram engine backed by a mature,
production-grade BDD library. The latency figures on this page were measured on
that shipped engine.

## Methodology

All figures are **read-side query latency** — the wall-clock time to answer a
`valid_options` query against a model that has already been compiled and loaded
into memory. They do **not** include the one-time cost of compiling the model
or loading it from disk; that is a startup cost paid once, not per query.

For each benchmark the solver issues a warmed-up batch of `valid_options`
queries drawn from the model's feature vocabulary, records the per-query
timings, and reports the median (P50) and 99th-percentile (P99) latency. P99
is the headline number: it is the latency at or below which 99 out of every
100 queries complete, and it is the figure that matters for a responsive
interactive experience.

The benchmark models are published, third-party feature models, not figures
chosen to flatter ConfigFlux. The FAMA corpus and the SPLOT *eShop* model are
both standard reference workloads in the software-product-line literature.

## FAMA feature-model corpus

The FAMA corpus is a set of small, well-known feature models that exercise the
common relationship types in feature modelling — mandatory and optional
features, alternative and or groups, and cross-tree *requires* / *excludes*
constraints.

Across the corpus, `valid_options` is consistently fast: the slowest model
answers a full query batch at a 99th-percentile latency of roughly **47
microseconds**, and the fastest at under **3 microseconds**. In other words,
on these models a configuration query completes in well under a tenth of a
millisecond, hundreds of times faster than the threshold of human-perceptible
delay.

| Measure                          | Result            |
|----------------------------------|-------------------|
| `valid_options` P99 (worst model) | ~47 µs            |
| `valid_options` P99 (best model)  | ~2.6 µs           |
| Interactive budget                | comfortably met   |

For feature models of this scale, configuration queries are effectively
instantaneous.

## SPLOT *eShop* — a named real-world model

*eShop* is a real-world, business-to-consumer e-commerce feature model from the
SPLOT (Software Product Lines Online Tools) collection. With roughly 290
features and a richer constraint structure than the FAMA models, it is a more
demanding and more representative workload for a production configurator.

On the shipped decision-diagram engine, `valid_options` on *eShop* runs at a
median latency of about **13 milliseconds** and a 99th-percentile latency of
about **101 milliseconds**.

| Measure                  | Result    |
|--------------------------|-----------|
| `valid_options` P50      | ~13 ms    |
| `valid_options` P99      | ~101 ms   |

For an interactive configurator, a median around 13 ms keeps the typical query
imperceptibly fast, while the 99th-percentile tail stays comfortably inside a
hundred-millisecond responsiveness budget — the range a user experiences as
immediate rather than laggy. Real-world models of this size and complexity are
served interactively.

## Summary

| Workload                 | Scale          | `valid_options` P50 | `valid_options` P99 |
|--------------------------|----------------|---------------------|---------------------|
| FAMA corpus (worst case) | small          | —                   | ~47 µs              |
| SPLOT *eShop*            | ~290 features  | ~13 ms              | ~101 ms             |

ConfigFlux answers configuration queries fast enough for live,
interactive use across the public reference workloads measured here — from
sub-millisecond latency on the FAMA corpus to a comfortably interactive median
on a real-world model of nearly three hundred features. Performance on much
larger and more densely constrained models remains an active area of work; the
figures on this page describe the workloads validated for the current release.
