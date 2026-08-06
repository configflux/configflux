# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] - 2026-08-06

### Added

- `constraints`, a new top-level authoring namespace for policy rules. A rule
  such as "debug logging is not permitted in production" can now be declared
  as a named constraint, with its own id and doc string, alongside the facets
  it talks about — instead of being encoded as a component that exists only to
  carry the rule and that then shows up in the component graph. Constraints are
  written in the same expression grammar conditions already use, so there is no
  second language to learn. The compiler validates every declaration: a
  constraint whose expression does not parse is a compile error rather than a
  silently dropped rule, every facet it names must be declared under `facets`,
  and every value it names must belong to a closed facet's declared domain. A
  declared constraint is compiled into the model, so `cfx options` stops
  offering a value that no valid configuration can hold and `cfx explain`
  reports the rule that blocks a selection. Naming a facet that is not declared
  is a validation error: declaring the facet is what lets `cfx options` and
  `cfx select` enforce a rule exactly as `cfx resolve` does, so a constraint
  that could only be half-enforced is refused at compile time rather than
  shipped.
- **Migration note for models written against an earlier release:** a
  `condition` on a component now means one thing only — it selects whether that
  component is part of the resolved configuration. A component that existed
  only to carry a rule (a `condition` and nothing else) recompiles without an
  error and no longer enforces that rule. Move every such rule to a
  `constraints:` entry; the condition text moves across unchanged. Neither the
  request schema version nor the compiled-package version check can detect this
  case, because authored sources carry no version marker. See the FAQ entry
  "The one migration the version check cannot catch for you".
- Model Explorer, a local, read-only web app for inspecting compiled models.
  Open `explorer/index.html` in a browser — no build step, no server, no
  network access — to browse a model's component/facet/option tree with
  search, trace a resolution's hash lineage from model hash through selection
  state to resolved snapshot, and read `explain` unsatisfiability reports
  rendered with the same wording as the CLI. Sample fixtures are bundled, the
  UI guards against unsupported schema versions with a visible error naming
  both versions, and the app is keyboard-accessible with a light/dark theme.
- A new first example, `examples/00-service-multi-env`: a multi-environment
  (dev/staging/prod) service configuration demonstrating declared facets,
  cross-facet policy constraints, and a guided `cfx explain` walkthrough of
  an unsatisfiable request.
- The provenance lineage record format is now specified in the published
  runtime contract (`docs/runtime-v2-contract.md`), so external tooling can
  parse promotion lineage against a documented format instead of reverse
  engineering it.
- Every diagnostic code the compiler, interpreter, and runtime can emit is now
  documented in one place (`docs/diagnostics.md`), with what causes it and what
  to do about it. A code such as `E_RESOLVE_FACET_UNBOUND` is part of the
  interface contract and does not change when a message is reworded, so it is
  safe to branch on, log, and search for.

### Changed

- **Breaking:** the product schema version is now 4 (`PRODUCT_SCHEMA_VERSION`
  4), and compiled model packages carry a matching new package format version.
  Inputs authored against product schema 3 are rejected with
  `E_UNSUPPORTED_SCHEMA_VERSION` and a message naming the required version, and
  a model package produced by an earlier release is rejected outright rather
  than read as though it simply had no constraints. There is no migration tool
  and no compatibility mode; this is deliberate, so that the same bytes never
  mean two different things. To upgrade: recompile your sources with this
  release's compiler and set `schema_version` to `4` in any request you send to
  the loader or interpreter.
- **Breaking:** model hashes change for every model. The new `constraints`
  namespace is part of a model's content address, so recompiling unchanged
  sources with this release produces a different `model_hash` — and with it a
  different `selection_state_hash`, `resolve_hash`, and software BOM hash. Any
  hash you have pinned in a deployment check, a golden file, or a provenance
  record must be re-recorded after recompiling.
- `cfx explain` now names the constraint that blocks a combination, by the id
  you gave it, and quotes its condition — for example:
  `blocked by constraint prod_forbids_debug: environment != 'prod' || log_level != 'debug'`.
  It previously listed the facet and option values the conflicting path
  branched on — including some with no bearing on the conflict — and never
  named the rule you had written. A conflict that no declared constraint
  accounts for is reported as the model being over-constrained, rather than
  attributed to the nearest rule.

### Fixed

- cfx resolve now rejects selections that violate the model's policy constraints (exit 3, E_SELECTION_CONFLICT) instead of emitting a resolved snapshot — resolve, options, and explain now agree on which selections are valid.
- The `cfx explain` command that `cfx resolve` suggests after refusing a
  selection now reproduces the selection it refused. When part of that
  selection came from a `--selection-file`, the suggested command left the file
  out, so running it as printed asked about a different selection — commonly
  answering that the selection you had just been refused was fine.
- Applying a selection that a declared constraint forbids is now reported as a
  selection conflict that names the rule and quotes it, with the same message
  `resolve` gives. It was previously reported as an internal error advising you
  to recompile the model, even though the model was correct and the rejection
  was the rule doing its job.
- The `explain-rejection` command of the `configflux-runtime` binary no longer
  disagrees with `cfx explain` about which rule blocks a conflict. The
  `unsat_core` it returns now carries the violated constraint's authored id and
  its condition text — the same constraint and condition `cfx explain` names for
  the same conflict. It previously returned an unattributed model rule that only
  listed the parameters and values involved and carried no constraint id at all,
  so the two surfaces described one conflict on one model two different ways.
- `cfx options` and `cfx explain` no longer withdraw options that are still
  valid. A conditional parameter override — "use this value when `environment`
  is `prod`" — was being read as a rule about which selections are allowed, so
  an unrelated option could vanish from the offered set model-wide, including
  options that resolve successfully. An override condition now selects a value
  without restricting what you may choose, so an option is offered wherever
  some valid configuration still contains it, and withdrawn only where a rule
  genuinely forbids it.
- Option pruning reported in resolution responses (`pruned_options`) now
  follows the solver's authoritative valid set rather than the compiler's
  narrowing, so the reported pruning can never contradict the option set that
  `options` offers for the same selection state.
- `E_RUNTIME_CLI_REQUEST_INVALID` diagnostics now name the offending request
  field instead of reporting an unspecified invalid request.
- Release tarballs now include the `LICENSE`, `NOTICE`, and `LICENSING.md`
  files, and a tarball can no longer be produced without them.
- Integration guides: corrected stale `schema_version` and
  `ir_format_version` examples and an incorrect description of the
  `resolve_hash` preimage.

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
