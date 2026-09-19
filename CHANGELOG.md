# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0] - 2026-09-19

### Added

- A parameter can declare `facet: <name>` to become that facet's handle: the
  compiler checks the declaration (the facet exists, the type is string, no own
  value, one handle per facet) and resolution sets the parameter's value to the
  facet's effective value.
- **Typed catalogues, bindings, and requirements.** A model can now declare a
  **catalogue** — a typed table of named entries, such as the physical
  containers a plant uses or the firmware modes a device supports — written
  once, as data, and checked at compile time. Any unit may declare one: the
  shared table belongs wherever it is owned, and a service can carry a table of
  its own.

  A **binding** is one shared choice of an entry from a catalogue. It behaves
  exactly like a facet whose options are that catalogue's entry ids: chosen per
  deployment, listed by `cfx options`, referenced by constraints, and explained
  like any other facet. A binding may declare a default entry, or a `derive`
  table that fixes its entry from another facet such as the deployment site.

  A component declares what it needs with `requires`, naming a binding under a
  slot of its choosing, and may narrow it to the entries it `accepts`. Two
  components that require the same binding always receive the same entry — that
  is how an author says "these must match" — while two bindings over one table
  stay independent decisions. When several components require one binding, the
  integrator is offered the intersection of their accept lists; an empty
  intersection is refused at compile time rather than discovered later as an
  unsatisfiable selection.

  Every resolved snapshot delivers the entry a requirement resolved to **inside
  the requiring service** — the entry's id and all of its fields — so a service
  reads its own configuration and never has to know where the catalogue lives.
- **An environment states only its free decisions.** Resolve now infers what the
  model's constraints already decide: when a selection leaves a facet unbound
  but the constraints admit exactly one value for it, that value is bound
  automatically and reported as an `implied:` line, recorded under
  `implied_choices`. Naming a deployment site is therefore enough to fix the
  container that site is equipped for. Explicit choices and context tags still
  win, and a declared default applies only when more than one value remains.
- **Two facets can be required to agree.** A constraint's right-hand side may
  now be another facet's name rather than a literal value
  (`sorter_container == line_container`, or `!=` for the opposite), so two
  independently bound choices can be tied together within a deployment without
  merging them into one facet and losing the ability to differ elsewhere.
- **A refused choice names the rule behind it.** `cfx explain` attributes a
  rejection to the construct you actually wrote: the requirement and the entries
  it accepts, or the binding and the derive entry that fixed its value. A
  rejection reads as `blocked by requirement compute_service.container: accepts
  c1, c2`, in your own words, rather than as a rule nobody authored.
- `cfx diff` compares two compiled models across every environment in a
  manifest and reports, per deployment target, whether the delivered
  configuration is unchanged, changed (with each parameter's before and after
  value), newly unsatisfiable, or newly satisfiable. It exits 0 only when no
  target changed, so a pull-request check can fail exactly when a model edit
  would alter what a running service receives.
- A new example, `examples/06-catalogue-polyrepo`, shows a model composed from
  chunks kept in four separate repositories: a shared catalogue of physical
  containers defined once as typed data with two bindings over it, and three
  services that declare what they require. Two of them name the same binding
  and receive the same container by construction; the third draws a different
  container from the same table and brings a catalogue and a binding of its
  own. Each environment names only its site and the decisions nothing in the
  model makes for it — the container the line runs is derived from the site —
  and every service reads what it was given out of its own snapshot. The
  walkthrough also forces three conflicts and shows `cfx explain` naming the
  rule behind each. It ships with `examples/export_pack.sh`, a reference
  exporter that resolves inheritance across any number of chunk files.
- `cfx resolve` now reads an environment manifest directly:
  `--manifest environments.json --environment <name>` resolves one named
  deployment target, and `--all` resolves every environment (optionally across
  several service scopes with `--scopes`) into one directory per target. The
  manifest is the same small JSON file the reference resolver script accepted;
  the script remains available for integrators driving the request envelopes by
  hand.
- Resolved snapshots now carry `resolved_output_hash`, a hash of the resolved
  payload alone. Unlike `resolve_hash`, which changes whenever the model
  changes, it changes only when the values a service would receive change — so
  a deployment check can tell an unrelated model edit from a real configuration
  change.
- `cfx resolve --out` now writes the resolved snapshot itself
  (`resolve_result.<root>.<selection>.json`, the JSON a service reads at
  startup) alongside the generated C++ early-binding files. The snapshot is
  byte-identical to `cfx resolve --format json` and to the interpreter's
  `resolve` response, so an output directory plus a copied `ccm/` is a complete
  delivery bundle.
- Models can now be built unit by unit. `compile-object` compiles one unit — a directory of chunks, whether a folder in a monorepo or a separate repository — into a content-addressed object against the interfaces it depends on, and `link` assembles objects into the same package the one-shot compiler produces. Link reports a named error when an import is unresolved, an id is declared twice, or a unit was compiled against a different version of an interface than the one being linked. A lockfile pins the object hashes an integration expects, and unchanged units are reused byte for byte.
- **The official toolchain image is now published.** Every tagged release
  publishes `ghcr.io/configflux/toolchain:vX.Y.Z`, carrying the `cfx`, `compiler`,
  `interpreter` and `runtime` executables together with the exact pinned CUE
  evaluator the project builds against. It lets you run the toolchain on any
  host with a container runtime, including platforms where ConfigFlux ships no
  native binary, and a compile-and-resolve run inside the container reproduces
  the host result byte for byte. The image is signed with the same keyless
  signing the binaries use, and the digest is what is signed — so pinning
  `ghcr.io/configflux/toolchain@sha256:<digest>` gives you a verifiable,
  reproducible toolchain. There is deliberately no `latest` tag. See
  [the toolchain image guide](docs/toolchain-image.md) for the invocation
  contract and the verification recipe.

### Changed

- `compiler inspect parameter` now reports the facet a parameter is the declared
  handle for, so the compile-time view names where a bound parameter's value
  comes from, as the runtime read of the same parameter already did.
- Example 03 now shows a runtime write refused by a declared constraint, with the violated rule named in the response.
- Runtime constraint enforcement now applies to parameters that declare a
  `facet` binding; a parameter that merely shares a facet's name is no longer
  treated as that facet.
- The runtime C ABI minor advanced to 1.3. Exported symbols and signatures are
  unchanged from 1.2; the minor advertises the declared facet-binding fields in
  snapshot and rejection payloads.
- `compiler verify` now reports the same `model_hash` that `compile` emits for
  the same sources; when verification fails the field is omitted.
- **Facet keys, binding ids, catalogue ids and catalogue entry ids must be
  snake_case identifiers, and facet values must be tokens of letters, digits,
  `_`, `.` and `-`.** The compiler refuses anything else when it reads the model
  instead of emitting one that misreads it.
- **The runtime API contract now documents the `explain-rejection` command**, which answers why setting a parameter to a given value would be refused: section 6.13 of `docs/runtime-v2-contract.md` carries its request and response shapes, the unsatisfiable-core payload they nest, and the one common envelope field its response does not carry.
- Context tags are now part of the environment every `cfx` surface reasons over. `cfx options` no longer lists values that a context tag rules out, `cfx explain` attributes a binding to the context tag that forced it, and `cfx select` now refuses a choice that a context tag excludes through a constraint. Previously only `cfx resolve` accounted for context tags, so the surfaces could disagree about which values a tagged deployment allowed.
- **Breaking:** compiled model packages carry a new format version and every
  `model_hash` changes on recompile; packages produced by an earlier release
  are rejected on load.
- **Breaking:** The product schema version is now 5. Inputs authored against
  version 4 are rejected with `E_UNSUPPORTED_SCHEMA_VERSION`; recompile and set
  `schema_version` to 5 in requests.
- **The signature verification recipe now pins the signing identity to the
  release workflow and to tag refs**, so a signature is accepted only from the
  run that actually published a release. The previous, broader identity pattern
  matched any workflow in this repository at any ref. Copy the updated `cosign`
  command from the install instructions or the toolchain image guide; the older
  command still verifies today's artifacts, but checks less than it appears to.
- Compare-and-swap guards on `set_parameters_atomically` and
  `commit_configuration` are fail-closed on every transport that forwards the
  field as sent (CLI, C ABI and agent for both operations; the ROS2 C++ adapter
  for `commit_configuration`, the only guard it exposes): a blank expected id
  is refused, and an expected-id field sent to the wrong operation is refused
  instead of ignored. Over the ROS2 service interface an empty
  `expected_base_configuration_id` still means no guard, because the message
  type cannot express absence.
- Runtime write requests now refuse undeclared fields inside individual write
  entries, so a misplaced compare-and-swap guard is reported instead of ignored.

### Fixed

- `compiler verify` and `compile` now report `E_FACET_VALUE_UNDECLARED` when a
  facet's `default` is not one of its declared values, instead of the generic
  `E_COMPILE_INPUT_INVALID`.
- The runtime refuses a model directory whose files are not regular files,
  bounds every model-file read, and no longer follows a manifest's
  `partition_manifest` outside the model directory.
- **A very long chain of inherited parameter definitions is now refused instead
  of crashing the compiler.** Asking about a parameter whose definition inherits
  through tens of thousands of links aborted the process; the chain depth is now
  bounded, and the refusal names the definition it stopped at.
- **A mistyped configuration id is now reported as bad input, not as a
  configuration that moved.** When a commit or an atomic parameter batch is
  guarded by an expected configuration id that is not a configuration id at all,
  the refusal says so and names the field, instead of telling the operator that
  another writer changed the configuration first.
- **A number that is not a finite number is now refused at compile time.** A
  parameter value, a limit, or a catalogue entry field written as `nan`, `inf`
  or `-inf` is rejected with a message naming where it was authored; previously
  such a value was silently dropped from the compiled model, and models that
  differed only in which of those three was written became indistinguishable.
- **The runtime v2 contract document now matches the shipped request and
  response shapes.** Section 6 named fields the runtime does not accept, so a
  caller shaping a request from it got a rejection or a silently wrong call;
  each operation now carries a worked JSON example of the payload it actually
  exchanges.
- **A supplied selection is now checked against the model before anything is
  answered over it.** `select`, `options`, `explain` and `resolve` refuse a
  selection state that names a facet the model does not have, or gives a facet a
  value outside its options, and the refusal names the facet and the value. A
  context tag is held to the declared values only where the model declares that
  facet's options exhaustively; a tag on a facet the model leaves open is still
  yours to set. Previously a selection assembled by hand rather than built up
  through `select` could carry such an entry, and the entry was silently ignored
  — so the answer that came back described a deployment nobody had asked for.
- **A selection the model proves impossible is now reported as a conflict on
  every surface.** `resolve` in particular could deliver a complete
  configuration, with a resolve hash over it, for a combination that cannot
  exist — when the contradiction ran through a binding whose value comes from a
  `derive` table and nothing in the model required that binding, there was
  nothing left for the resolver to fail on, while `cfx options` and `cfx
  explain` had already called the same selection unsatisfiable. All four
  surfaces now agree, and each names the choice that cannot hold.
- **A working model is no longer reported as an internal fault.** Selecting a
  value that empties a derived binding used to be answered with an
  engine-divergence code and an advice to recompile, on a model that was doing
  exactly what it says. It is now reported as the conflict it is. That code is
  reserved for three genuine internal faults, one of which is new and
  actionable: a compiled model missing a value its sources declare is now
  reported as a stale build, naming the value, so the fix is to recompile.
- **One partially modelled facet no longer switches off inference for a whole
  model.** A context tag carrying a value that appears nowhere in the model —
  a deployment region only some components care about, say — caused the
  implied-choice inference to give up entirely, so a resolve lost bindings the
  constraints themselves decide. Such a tag is now passed over, and everything
  the model does decide is still inferred.
- `cfx` now rejects a selection that binds the same facet twice with
  different options — whether by repeated `--select` flags or a duplicate
  key in a selection file — instead of silently keeping the last value. The
  interpreter already refused this; the two surfaces now agree.
- A runtime write or `explain-rejection` blocked by a policy over a two-valued
  facet — `on`/`off`, `enabled`/`disabled`, the most common shape there is —
  now names the constraint that was violated and quotes its condition, instead
  of reporting only that the model is over-constrained. The developer-facing
  `cfx explain` already did this; the runtime, which is what an operator sees
  on a device, did not, because it had no way to learn which facets are
  exhaustive. Resolved snapshots now carry that information as
  `closed_facet_domains`, and the runtime open request accepts it: a service
  that projects the whole resolve result into its open payload — as the
  fleet-edge example documents — gets the better message with no other change.
  A payload that omits the field still opens and keeps the previous message.
  The runtime C ABI minor moves to 1.2 to advertise this; clients that
  handshake with a lower expected minor stay compatible.
- **The `cfx explain` command suggested after an unsatisfiable selection can
  now be pasted and run as printed.** `cfx resolve` shell-quotes the model
  and selection-file paths it echoes into that suggestion, so the command
  still names the right files when either lives under a directory whose name
  contains a space or a quote; previously the shell re-split such a path and
  the suggested next step failed on files that do not exist.
- **`options` no longer reports valid options alongside a failed request.** When
  `options` cannot answer — the facet is one the model does not offer, or the
  selection it was given is not one the model admits — the refusal now comes back
  on its own, naming what went wrong and listing nothing. Previously the same
  answer carried a list of still-valid options underneath the error, so a caller
  reading the list without reading the status acted on options for a question the
  product had just declined to answer.
- `cfx` refuses a `--selection-file` or `--manifest` that is not a regular file
  and bounds the read at 8 MiB, matching the request bound of the other
  binaries.

### Security

- The documented signature-verification recipes now bind the certificate to the
  release being verified, so a signature from a different release cannot satisfy
  them.
- A compiled package whose facet, catalogue or binding symbols violate the
  symbol rule is now refused when it is loaded, not only when it is compiled.
- `link` now refuses an object whose header does not match its chunk files.
- Opening a runtime session now refuses a solver model that belongs to a
  different model than the snapshot being opened, so a session you opened
  cannot decide its writes against one model while reporting another.

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
- **Breaking:** a model's identity is now a function of its content alone. The
  address of a compiled chunk is the hash of the configuration it holds, so any
  reader can recompute it from the file and confirm that the content is the
  content the address names. Changing a `package` or `version` label no longer
  changes `model_hash`: two models that say the same thing now have the same
  identity whatever they are called. Because the rule that produces these
  hashes changed, they all rotate once with this release, and a model package
  built by an earlier release is refused when you open it, naming that reason.
  To upgrade: recompile your sources and re-record any pinned hash.
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
