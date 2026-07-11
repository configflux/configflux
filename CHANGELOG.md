# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-07-11

### Added

- `cfx`, a new one-shot resolution CLI, now ships as a ready-to-run binary in
  the release tarballs alongside the compiler, interpreter, and runtime. It
  drives the full resolution pipeline from a single command — `cfx options`
  lists the still-selectable options for a model, `cfx resolve` produces a
  resolved configuration, and `cfx explain` reports why an unsatisfiable
  request was rejected — so you can explore and resolve a compiled model
  without wiring up the individual binaries.
- First-class declared facets. A scenario may now declare a facet's full value
  domain together with a default value directly in its source, and that
  declared domain is honored consistently across `options`, `resolve`, and
  `explain`: the default is offered, and every declared value appears in the
  selectable option set even when it is not otherwise referenced by a
  constraint.
- Component dependency graphs may now be any directed acyclic graph, including
  diamond shapes where a single component is reached through more than one
  dependency path. Such a component is included exactly once in the resolved
  result.

### Changed

- Product schema version is now 3 (`PRODUCT_SCHEMA_VERSION` 3), reflecting the
  declared-facet additions above. Inputs authored against an older product
  schema are rejected with a message naming the required version.

### Fixed

- `options`/`resolve` and `explain` now agree on satisfiability: a request
  that resolves is never reported as rejected by `explain`, and a request that
  `explain` reports as unsatisfiable never resolves, so the two views can no
  longer disagree on the same model and selection.
- The `--version` flag now reports the actual release version on all binaries.
- Documentation corrections across the getting-started and reference material.

## [0.1.0] - 2026-07-07

First public release of ConfigFlux — a configuration compiler, solver, and
runtime for building, validating, and deploying constraint-checked
configuration. ConfigFlux is dual-licensed: the source is available under the
Business Source License 1.1 (production use permitted under the license's
Additional Use Grant; each released version converts to Apache-2.0 four years
after its release), with commercial licenses available for uses outside that
grant — see `LICENSING.md`.

### Added

- Compiler binary: ingests configuration scenarios authored in
  [CUE](https://cuelang.org/) — CUE owns inheritance and cross-file
  composition, giving authors a typed, schema-validated front-end — and
  compiles them into a resolved snapshot plus a compiled solver model
  (`.ccm`). Resolved outputs carry a deterministic `resolve_hash`, so
  identical inputs provably produce identical resolutions. Includes software
  BOM export, component overlays, conditional components, and cardinality
  operators in the condition grammar (`any_of`, `all_of`, `exactly_one_of`)
  for expressing "pick at least one", "require all", and "pick exactly one"
  constraints directly in scenario sources.
- A Boolean solver as the decision engine for the whole pipeline: option
  enumeration, selection validation, and final resolution in both the
  interpreter and the runtime are answered by the solver over the compiled
  `.ccm` model, so all query surfaces return consistent, constraint-checked
  answers:
  - `valid_options` returns the set of still-selectable options given the
    choices made so far, so a configurator can grey out anything that would
    lead to an unsatisfiable configuration.
  - `apply` records a selection and narrows the remaining option space.
  - Built on a reduced ordered binary decision diagram, so queries stay fast
    as the configuration space grows.
- Interpreter binary with deterministic policy evaluation and an `explain`
  subcommand: when a selection or a resolve request is unsatisfiable, it
  reports a minimal set of conflicting constraints — labeled with the source
  constraint names involved — as structured JSON or rendered, human-readable
  text, so authors can see *why* a configuration was rejected instead of just
  that it was.
- Runtime binary with delta-first sync semantics, a layered store, commit
  rollback, an event bus, and an `explain-rejection` command for operators.
  Opening a model at runtime fails closed unless a usable `.ccm` solver model
  ships next to the snapshot; empty, unloadable, or symbol-less `.ccm`
  references are rejected at open time.
- A deployment surface for container fleets and multi-environment setups:
  - Named environment manifests: store each deploy target (scope, context,
    choices) — production variants, staging, a developer "local" — as a named
    entry in an `environments.json` file, resolvable individually or as a
    matrix with the bundled `resolve_environment.sh` reference script.
  - A delivery-bundle convention — the resolved snapshot, the `.ccm` solver
    model, and their hash lineage ship together as one verifiable unit — with
    a `verify_bundle.sh` reference script.
  - A Docker Compose fleet example (`examples/05-compose-fleet`) showing named
    environments, per-service scoped bundles, and running a single service
    standalone for debugging.
  - A service integration guide for consuming resolved configuration from any
    language (Python, C#, and others) through the runtime CLI's JSON
    interface — no ConfigFlux SDK required.
- Stable C ABI for cross-language runtime integration
  (`CONFIGFLUX_RUNTIME_C_ABI_VERSION` 1.0.0), a C++ SDK with core runtime
  bindings, and a ROS2 SDK adapter with lifecycle-mode coverage.
- Product schema version 1 (`PRODUCT_SCHEMA_VERSION` 1).
- Progressive, self-contained runnable examples (`examples/`), from a minimal
  single-component config to multi-step selection with C++ export and BOM,
  each with a `run.sh` script and regression coverage in CI.
- A `--version` flag on all three binaries — the compiler, the interpreter,
  and the runtime — printing the release version for support and
  reproducibility.
- Pre-built Linux binaries for `x86_64` and `aarch64`, attached to each
  release as ready-to-run tarballs.

### Security

- Release tarballs are published with per-artifact SHA-256 checksums and a
  signed `SHA256SUMS` manifest. Signatures use keyless signing with a
  transparency-log record, so consumers can verify both the integrity and the
  provenance of a download without managing long-lived signing keys.
- All dependencies pinned with lockfile; supply chain audit baseline
  established.
