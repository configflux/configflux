# 00 · Multi-environment web service

One web service, three environments — `dev`, `staging`, and `prod` — expressed
as **selections over a single model** and resolved to deterministic snapshots
with the one-shot `cfx` CLI. This is the example to read first if you configure
services across environments.

It also shows the part that makes ConfigFlux different from a templating tool:
the model carries a **constraint** — debug logging is forbidden in `prod` — and
the toolchain surfaces that policy *before* a bad configuration can ship.

## The model

The model is one `webapp` service plus four **facets** (the selection
dimensions an environment picks from):

| Facet | Options | Meaning |
|-------|---------|---------|
| `environment` | `dev`, `staging`, `prod` | Deployment target |
| `log_level` | `info`, `debug` | Log verbosity |
| `replica_class` | `single`, `scaled` | Sizing selection |
| `beta_dashboard` | `off`, `on` | Feature toggle |

The service's values are shared defaults, hardened for production. Selecting
`environment=prod` tightens `request_timeout_ms` from `30000` to `5000` and
flips `deploy_tier` from `nonprod` to `production`. Those two overrides carry a
`condition` — `environment == 'prod'` — and a condition does exactly one thing:
it decides *whether that override applies*. It is an inclusion selector, never a
rule about what you are allowed to pick.

Policy is a separate construct. The model declares one **constraint**, alongside
the facets in [`00_definitions.json`](00_definitions.json):

```json
"constraints": {
  "prod_forbids_debug": {
    "condition": "environment != 'prod' || log_level != 'debug'",
    "doc": "Debug logging is not permitted in production."
  }
}
```

A constraint is a named assertion over facet values that must hold in **every**
resolved configuration. This one makes the pair `environment=prod` +
`log_level=debug` **unsatisfiable** — and because it is named, the tooling can
tell you *which* rule you hit. That is what the walkthrough below puts on
display.

## The three environments

[`environments.json`](environments.json) names the three deploy targets. Each is
a scope, an (empty here) context-tag set, and the facet choices for that
environment — the same named-environment shape used by
[`05-compose-fleet`](../05-compose-fleet/):

| Environment | log_level | replica_class | beta_dashboard | request_timeout_ms | deploy_tier |
|-------------|-----------|---------------|----------------|--------------------|-------------|
| `dev`       | `debug`   | `single`      | `on`           | `30000`            | `nonprod`   |
| `staging`   | `info`    | `scaled`      | `on`           | `30000`            | `nonprod`   |
| `prod`      | `info`    | `scaled`      | `off`          | `5000`             | `production`|

`dev` is allowed to run with `debug` logging. `prod` is not — and that is
enforced by the model, not by convention.

## Run it

```bash
bazel build //compiler //cfx      # once
cd examples/00-service-multi-env
./run.sh
```

`run.sh` compiles the model once, then resolves every environment named in
`environments.json` with `cfx resolve`, writing one snapshot per environment
under `out/resolved/<env>/`. It re-resolves each one and asserts the snapshots
are **byte-identical** across runs — determinism is the contract, so the same
model and selection always produce the same bytes and the same `resolve_hash`.

Each resolved snapshot is consumed the usual way (Pattern 1): the service reads
its snapshot JSON at startup, pins the `resolve_hash`, and fails closed if it
does not match.

## The unsat moment: `cfx options` and `cfx explain`

A `prod` deployment must never turn on `debug` logging. Two `cfx` commands make
that guarantee visible.

**`cfx options`** lists the choices still valid after a partial selection. Once
`prod` is chosen, `debug` is simply not offered:

```console
$ cfx options --model out/cmp/cmp.manifest.json --select environment=prod
facet beta_dashboard [closed, default: off]
  off
  on
facet environment [selected: prod, default: dev]
  prod
facet log_level [closed, default: info]
  info
facet replica_class [closed, default: single]
  scaled
  single
```

`log_level` shows only `info`. A guided walk can never land on the invalid
combination, because the model has removed it. The other three facets are
listed in full because `cfx options` always reports the whole decision space —
`prod` narrows `log_level` and nothing else.

**`cfx explain`** answers *why* when you deliberately force the bad pair:

```console
$ cfx explain --model out/cmp/cmp.manifest.json --select environment=prod --select log_level=debug
cannot select log_level.debug:
  blocked by your earlier choice: environment.prod
  blocked by constraint prod_forbids_debug: environment != 'prod' || log_level != 'debug'
(one minimal explanation; other minimal cores may exist)
```

That is the differentiator: instead of a downstream failure buried in a running
service, you get a **minimal, precise conflict** at configuration time — the
exact earlier choice (`environment.prod`) and the constraint that forbids the
rejected option, named by the id you gave it (`prod_forbids_debug`) with its
condition quoted back.

## What to notice

- **One model, many environments.** Environments are selections over a single
  model, not forked copies of a config file.
- **Determinism.** Re-resolving is byte-identical; the snapshot is a pure
  function of the model and the selection.
- **Policy lives in the model, in its own construct.** The
  `prod`-forbids-`debug` rule is a named `constraints:` entry, compiled in.
  `cfx options` hides the invalid option; `cfx explain` names the exact minimal
  conflict when you force it.
- **Conditions and constraints are different things.** A `condition` selects
  what a configuration *contains* (which override applies); a `constraint` says
  what a user is *allowed to pick*. Keeping them apart is why the two
  `environment == 'prod'` overrides above narrow no one's choices.

## Files

- [`cue/`](cue/) — the authored CUE sources (`00_definitions.cue` — definitions,
  facets, and the constraint; `10_components.cue` — the `webapp` service).
- `00_definitions.json`, `10_components.json` — the exported chunks the compiler
  ingests.
- [`environments.json`](environments.json) — the three named environments.
- [`run.sh`](run.sh) — compiles, resolves every environment, checks
  determinism, and runs the constraint showcase.

The resolved snapshots and explain output this example produces can be loaded
into the repository's model explorer — see `explorer/README.md`.
