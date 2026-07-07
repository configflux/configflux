# 05 — Compose Fleet

A fleet of identical container instances in miniature: **one shared model**,
**two named environments**, **two service scopes**, consumed by a containerized
Python service. It shows the deployment surfaces ConfigFlux ships as
*conventions* working together end to end — named environments,
matrix resolution, the delivery bundle, and direct snapshot consumption — with
no product-specific deployment tooling involved.

The worked instances here (`robot-alpha`, `local`) are concrete examples of
*named environments + scoped delivery + container consumption*. Nothing in the
design is specific to robots, edges, or any topology: a "named environment" is
just a reproducible resolution target, and the same pattern serves staging
machines, per-tenant deployments, a developer laptop, or anything else that
consumes a resolved configuration.

## What it shows

- **One model, many targets.** A single compiled model is resolved for two
  named environments (`robot-alpha`, `local`) across two service scopes
  (`vision_service`, `telemetry_service`) — a 2×2 matrix yielding four
  independent, deterministic snapshots, each with its own `resolve_hash`.
- **Named environments are a convention, not a product input.** An environment
  is a name bound to `(scope, context_tags, choices)` in a small JSON file you
  own (`environments.json`). The reference resolver
  [`../resolve_environment.sh`](../resolve_environment.sh) desugars it into the
  ordinary `open → init → select → resolve` interpreter chain. The product
  binaries learn no new input format.
- **The delivery bundle is one unit.** Each target ships as a bundle: the
  resolved snapshot plus the `ccm/` solver model, together. A validated runtime
  session fails closed without a usable `ccm/` next to the snapshot, so the two
  always travel together. The reference verifier
  [`../verify_bundle.sh`](../verify_bundle.sh) cross-checks a bundle before
  delivery.
- **Container consumption is just reading JSON (Pattern 1).** The Python
  service reads its scoped snapshot directly at startup, verifies the resolve
  status and pins `resolve_hash`, and **fails closed** on anything it cannot
  trust. No ConfigFlux binary runs inside the container.
- **Override generation is your code, not the product's.** The product emits
  the rich snapshot; shaping it into a `docker-compose` override (or env-file,
  or Helm values) is a thin user-side transform. This example keeps that
  transform where it belongs — in [`tools/gen_compose_override.py`](tools/gen_compose_override.py) —
  and frames it as the seam it is.
- **The same service runs standalone for debugging.** The identical
  `service/app.py` runs inside the compose stack (against the `robot-alpha`
  bundle) and standalone on a developer machine (against the `local` bundle).
  Only the snapshot path changes.

## The two environments

Both environments resolve from the same model; their `choices` select different
override arms, so the resolved values differ:

| Parameter | `robot-alpha` (field) | `local` (developer) |
|-----------|------------------------|----------------------|
| `vision_service.broker_endpoint`    | `tcp://broker.fleet.local:1883` | `tcp://127.0.0.1:1883` |
| `vision_service.tick_interval` (ms) | `33`   | `200`  |
| `telemetry_service.tick_interval`   | `1000` | `5000` |
| `*.log_level`                       | quiet (`info`/`warn`) | chatty (`trace`/`debug`) |

## Files

| File | Purpose |
|------|---------|
| `cue/00_definitions.cue` | Parameter definitions (the inheritance roots) |
| `cue/10_components.cue` | Two service components + a shared `platform`, with facet-driven overrides |
| `00_definitions.json`, `10_components.json` | Inheritance-resolved chunks exported from `cue/`; fed to the compiler |
| `environments.json` | The two named environments (the `(scope, context_tags, choices)` convention) |
| `service/app.py` | The Pattern 1 consumer: reads the scoped snapshot, pins lineage, fails closed |
| `service/Dockerfile` | Containerizes the service over a delivery bundle |
| `docker-compose.yml` | A one-node stack running both scoped services from one image |
| `tools/gen_compose_override.py` | **User-side** snapshot → compose-override transform |
| `run.sh` | The full chain: compile → matrix-resolve → bundle+verify → override → compose → standalone |
| `run_standalone.sh` | Runs the service standalone against the `local` bundle |

## Run it

```bash
# Build the binaries once:
bazel build //compiler //interpreter

# Then run the example:
./run.sh
```

`run.sh` uses the reference resolver and verifier, which require `jq`. If
**docker** is available it brings up the compose stack and confirms both
containerized services load their scoped config; if docker is absent it prints
a notice and skips the compose phase — the standalone path then exercises the
same service code. Authoring the model from `cue/` uses the pinned `cue`
evaluator out of band; `run.sh` itself consumes the committed `*.json` and never
invokes `cue`.

## What to look at

After running, check `out/`:

- `cmp/cmp.manifest.json` and `cmp/ccm/` — the compiled model and its solver model
- `resolved/<env>/<scope>/resolve_result.*.json` — the four per-target snapshots
- `bundles/<env>--<scope>/` — the four delivery bundles (snapshot + `ccm/`)
- `vision-service.compose.override.yml` — the generated (user-side) override
- the compose logs and the standalone run print the resolved config each service loaded

## Key concepts

**A named environment is reproducible.** `environments.json` pins the three free
fields of a resolution. Resolving the same environment again yields a
byte-identical snapshot and the same `resolve_hash`, so "deploy the robot-alpha
configuration" means exactly one configuration, in version control.

**Matrix resolution is N independent resolves.** Resolving two environments
across two scopes is four ordinary, independent resolves — a documented loop,
not a batch product verb. Each cell is deterministic and has its own lineage.

**Fail closed is the headline operational fact.** The on-target guarantee is not
this example's pre-delivery `verify_bundle.sh` check (that is a convenience); it
is that a validated runtime session refuses to start without a usable `ccm/`
next to the snapshot. Shipping the bundle whole is what makes a target
trustworthy. The Pattern 1 service mirrors that posture in user code: it pins
`resolve_hash` and refuses to serve a configuration it cannot identify.

**The product stops at the snapshot.** Where an override or orchestrator config
is generated, and in what dialect, stays with the consumer who owns the
orchestrator. `tools/gen_compose_override.py` is that consumer-owned transform;
change it (not the product) to target a different orchestrator.

## Next steps

See the [examples index](../README.md) for the full list, and
[`docs/service-integration-guide.md`](../../docs/service-integration-guide.md)
for the three consumption patterns in depth.
