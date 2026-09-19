# Starting from Scratch: Modeling Your Domain with ConfigFlux

This guide walks you through authoring a ConfigFlux configuration model for
a new product domain. It assumes you have built and run the existing examples
(see `docs/canonical-worked-example.md` and the packs under `examples/`) and
now want to model your own equipment, product family, or system.

Models are authored in **CUE**. You write typed `.cue` chunks, export them to
inheritance-resolved JSON, and feed that JSON to the compiler. CUE is the sole
authoring format; the exported JSON the compiler ingests is an internal artifact
of that export pipeline, not a format you hand-write. This guide teaches the flow
end to end.

## Prerequisites

- The two ConfigFlux commands, built from this repo and exposed on your `PATH`:

  ```bash
  bazel build //compiler:compiler //cfx:cfx

  mkdir -p .cfx-bin
  ln -sf "$PWD/bazel-bin/compiler/compiler" .cfx-bin/configflux-compiler
  ln -sf "$PWD/bazel-bin/cfx/cfx"           .cfx-bin/cfx
  export PATH="$PWD/.cfx-bin:$PATH"
  ```

  `configflux-compiler` compiles your authored model into a Compiled Model
  Package (CMP). `cfx` is the one-shot resolver that turns that package into a
  concrete resolved configuration. Every command in this guide is written
  against those two names, and every command is run from the repository root.

- The pinned `cue` binary, **version 0.16.1**. The export step requires this
  exact version; other versions are not supported. Set the `$CUE` environment
  variable to point at it.
- Familiarity with [CUE](https://cuelang.org/) syntax: structs, fields, the
  `&` unification operator, and definitions (`#Name`).
- A clear picture of the "things" your product configures and the choices a
  customer or integrator makes when ordering or commissioning it.

## The Big Picture: Author, Export, Compile, Resolve

ConfigFlux authoring has four stages. Keep this pipeline in mind as you read
the rest of the guide:

```
  .cue chunks            resolved .json            CMP (compiled model package)
 (you author)   ---->   (export step emits)  ---->   (compiler ingests)
  package configflux      inheritance filled in       cmp.manifest.json, *.cfir
  chunk: #Config & {...}  by #ResolvePack             index.cfir.json, ccm/
                                                                |
                                                                |  cfx resolve
                                                                v
                                                         resolved snapshot
                                                       (one configuration,
                                                        hash-pinned)
```

1. **Author** your model as CUE chunks validated against `compiler/cue/schema.cue`.
2. **Export** the chunks to JSON. The export step runs your chunks through the
   schema's inheritance engine (`#ResolvePack`), so the emitted JSON has every
   inherited field filled in. This is the JSON the compiler ingests.
3. **Compile** the resolved JSON into a Compiled Model Package (CMP) with
   `configflux-compiler compile`. The CMP is your **150% model** — every variant
   your chunks describe, merged and validated.
4. **Resolve** the CMP down to one concrete configuration — the **100% model** —
   with `cfx resolve`. Section 11 walks through this stage.

The fastest way to see the whole pipeline run is to execute a shipping example:

```bash
cd examples/00-service-multi-env && ./run.sh
```

That script compiles the example's already-exported JSON and then resolves it
with `cfx`: one model, three environments, each exported to its own
deterministic snapshot. Read the rest of this guide to learn how to build your
own.

## 1) Think in Components

A **component** is a named unit of configuration that the system treats as a
whole. Components map to the logical parts of your product: a controller, a
sensor module, a power stage, a display panel.

### When to create a component

- The part has its own parameters that make sense as a group.
- The part can be present or absent depending on product variant.
- The part has a dependency relationship with other parts (it needs them to
  exist).

### When to combine vs. split

- **Combine** parameters into one component when they always travel together
  and share the same lifecycle (e.g., all PID tuning gains for a single
  controller).
- **Split** into separate components when parameters can appear independently,
  when one part can be conditionally removed without removing the other, or
  when different teams own different parts.

### Component structure in CUE

Components live under `components` inside the chunk's `#Config`. Every
component needs at least a `type` field. Parameters live inside the component
under `params`:

```cue
components: {
    my_controller: {
        type: "controller"
        params: {
            max_speed: {
                type:      "float"
                unit:      "rpm"
                value:     3000.0
                lifecycle: "startup"
            }
        }
    }
}
```

`#Component` is a closed struct: it accepts only `type`, `condition`,
`depends_on`, and `params`. A misspelled field name is a hard error at export
time, not a silent no-op.

### Component dependencies

If one component requires another to be present, declare it with `depends_on`.
The compiler validates that dependency targets exist and that the dependency
graph is acyclic. The graph may be any DAG: a component reachable via multiple
paths from the same root (a diamond) is fine — model a shared platform or HAL
layer with many consumers directly (ADR-0048). Only cycles are rejected:

```cue
components: {
    motor_drive: {
        type: "actuator"
        depends_on: ["power_stage"]
    }
}
```

The compiler also checks **condition compatibility**: if component A depends on
component B, then A's condition must logically imply B's condition. A
conditional component cannot depend on a more-narrowly-scoped component that
might not be present when A is active.

## 2) Parameter Types and Lifecycle Levels

Every parameter has a **type**, a **value**, and a **lifecycle**.

### Supported value types

The schema's `#Value` admits integers, floats, booleans, and strings. The
`type` keyword names the kind:

| Type keyword | CUE value | Description |
|---|---|---|
| `"float"` | `3.14` | Floating-point number |
| `"integer"` | `42` | Signed 64-bit integer |
| `"boolean"` | `true` / `false` | Boolean flag |
| `"string"` | `"some_text"` | Text string |
| `"artifact"` | `"artifact_id"` | Reference to an artifact entry (see section 5) |

### Lifecycle levels

Lifecycle tells the system when a parameter's value is bound and whether it can
change after that point. The allowed values (`#Lifecycle`) are `construction`,
`startup`, and `runtime`:

| Lifecycle | Meaning | Typical use |
|---|---|---|
| `construction` | Compile-time constant. Frozen into the compiled model package. Cannot change after compilation. | Hardware variant selection, driver binary references, board revision |
| `startup` | Read-only after boot. Set during commissioning, locked when the system starts. | Calibration values, safety limits, commissioning profiles |
| `runtime` | Mutable. Can be changed while the system is running (subject to constraints). | Trim gains, setpoints, operator-adjustable thresholds |

Choose the most restrictive lifecycle that makes sense. A parameter that never
changes after the factory should be `construction`, not `runtime`.

### Safety levels

Safety classification follows IEC 61508 integrity levels. The allowed values
(`#SafetyLevel`) are `q_m`, `sil1`, `sil2`, `sil3`, and `sil4`:

| Safety | Meaning |
|---|---|
| `q_m` | Quality Managed (standard, no safety claim) |
| `sil1` | Safety Integrity Level 1 |
| `sil2` | Safety Integrity Level 2 |
| `sil3` | Safety Integrity Level 3 |
| `sil4` | Safety Integrity Level 4 |

Note the value is `q_m`, with an underscore -- not `qm`. The schema rejects any
other spelling.

```cue
definitions: {
    safe_flow_limit: {
        type:      "float"
        unit:      "lpm"
        safety:    "sil2"
        lifecycle: "startup"
    }
}
```

### Access roles

Access controls who can view or modify a parameter. The allowed values
(`#Role`) are:

| Role | Typical persona |
|---|---|
| `developer` | Software engineer building the product |
| `integrator` | System integrator commissioning the product |
| `technician` | Field technician performing maintenance |
| `supervisor` | Operations supervisor |
| `super_user` | Unrestricted access |

### Constraints (limits)

You can constrain numeric parameters with min/max bounds and string parameters
with length bounds. Limits live in a `limits` struct (`#Limits`), which is
closed and accepts only `min`, `max`, `min_len`, and `max_len`:

```cue
components: {
    motor: {
        type: "module"
        params: {
            max_speed: {
                type:      "float"
                unit:      "rpm"
                value:     3000.0
                lifecycle: "startup"
                limits: {
                    min: 0.0
                    max: 6000.0
                }
            }
        }
    }
}
```

`min_len` and `max_len` must be non-negative integers. Any other key inside
`limits` is rejected.

## 3) Definitions: Reusable Parameter Templates

**Definitions** are named parameter templates that live at the top level of a
chunk under `definitions`. They let you declare a parameter's type, unit,
safety, lifecycle, access, limits, and doc once, then reuse that shape across
components.

A definition is **valueless**. The schema's `#Definition` forbids the `value`
field: the concrete value is always authored on the *using* component
parameter, never on the definition. This mirrors the resolution rule -- value
is never inherited.

```cue
// In 00_definitions.cue
definitions: {
    safe_flow: {
        type:      "float"
        unit:      "lpm"
        safety:    "sil2"
        lifecycle: "startup"
        access:    "integrator"
        doc:       "Safe commissioning flow limit"
    }
}
```

A component parameter inherits from a definition by setting `inherits` to the
definition's id and supplying its own `value`:

```cue
// In 10_components.cue
components: {
    controller: {
        type: "module"
        params: {
            max_flow_at_commissioning: {
                inherits: "safe_flow"
                value:    42.5
            }
        }
    }
}
```

You do **not** repeat `type`, `unit`, `safety`, `lifecycle`, `access`,
`limits`, or `doc` on the component parameter -- those are filled in from the
definition during the export step (see section 9). You author only what is
specific to this use: the `inherits` pointer and the `value`.

### What inheritance fills in (and what it does not)

The export step gap-fills exactly these fields from the parent definition, and
only when the child left them absent:

- `type`, `unit`, `safety`, `lifecycle`, `access`, `limits`, `doc`

These fields are **never** inherited -- they stay author-owned on the child:

- `value` (child only -- definitions are valueless, so there is no parent value
  to push down)
- `inherits`, `req_id`, `condition`, `overrides`

"Fill gaps, do not overwrite": if the child authors a field, the child's value
wins and the parent's is ignored. The parent only supplies fields the child
omitted.

### When to use definitions

- When multiple components share the same parameter shape (type, unit, safety,
  lifecycle).
- When you want to enforce that all instances of a concept (e.g., "driver
  slot") share the same metadata.
- When a parameter type like `"artifact"` should be reused with the same
  lifecycle and access across components.

The compiler validates that every `inherits` target exists in the definitions
and that there are no inheritance cycles.

## 4) Selection Domains: Designing Facets for Your Product Family

ConfigFlux uses a **150% model**: the source chunks describe every possible
variant of your product. At selection time, a user picks values for each
**facet** (a named dimension of variation), and the system prunes the model
down to the **100% model** for that specific configuration.

### What is a selection domain?

A selection domain is a facet name mapped to its allowed values. Facets
represent the choices that differentiate one product variant from another.
A resolution profile (`#Profile`) carries the selection domains and a default
context. In CUE:

```cue
profile: #Profile & {
    selection_domains: {
        cooling_brand: ["hydra", "aeroflux"]
        cooling_model: ["x200", "a9"]
        pump_type: ["single", "dual"]
        region: ["us", "eu"]
    }
    default_context: {
        cooling_brand: "hydra"
        cooling_model: "x200"
        pump_type:     "dual"
        region:        "us"
    }
}
```

`#Profile` requires `selection_domains` and `default_context`; `profile_id` and
`scenario_id` are optional metadata. A profile is a *resolution input*, not part
of the 150% model -- it describes the choices a user makes, while the `#Config`
chunks describe the components those choices select among.

### How to choose your facets

1. **List the choices** that change your product's behavior: which hardware
   variant, which region, which safety level, which optional module.
2. **One facet per independent axis of variation.** If two choices always change
   together, they are one facet. If they can vary independently, they are
   separate facets.
3. **Keep the value set small and meaningful.** Each value should correspond to
   a real product option, not a synthetic combination.

### Requiring two facets to agree

Rule 2 above says that choices which always change together are one facet. Some
choices only have to agree *within a deployment*, though, and merging them would
throw away the ability to differ elsewhere. Say a line and a sorter usually draw
from the same container class, but a maintenance site runs them apart. Keep them
as two facets and write the agreement as a constraint whose right-hand side is
the other facet's name, unquoted:

```cue
constraints: {
    groups_equal: {
        condition: "sorter_container == line_container"
        doc:       "The sorter and the line draw from the same container."
    }
}
```

Both facets stay independently bindable and keep their own domains; only the
constraint ties them together. `!=` requires the opposite. The quotes are what
distinguish the two readings — `container == 'c1'` still compares against the
literal value `c1`, while `container == other_container` compares against
another facet — so an unquoted name that is not a declared facet is a compile
error rather than a silently mistyped literal.

### Examples of facet design

| Domain | Facets | Values |
|---|---|---|
| Water pump system | `cooling_brand`, `cooling_model`, `pump_type`, `region` | brand names, model numbers, single/dual, us/eu |
| Automation cell | `conveyor_brand`, `vision_stack`, `safety_mode`, `network_topology` | vendor names, pl_d/pl_e, ring/star |
| Building HVAC | `occupancy_class`, `filtration_grade`, `region` | office/hospital, merv13/hepa, us/eu |

### Declaring a facet's domain and default (optional)

By default a facet is *implicit*: its domain is inferred from the values your
conditions compare it against. That is enough for many models, but it cannot
represent a **default arm** — a value that is correct when nothing is selected
and that therefore appears in no condition. To make the full domain and its
default first-class, declare the facet with a `#Facet` in your
`00_definitions` chunk (ADR-0047):

```cue
facets: {
    region: {
        values:  ["us", "eu"]   // ordered, non-empty, unique
        default: "us"           // must be one of values; the arm no condition names
        doc:     "Deployment region"
    }
}
```

A declared **closed** facet (the default; add `open: true` only for an
extensible domain) publishes its whole vocabulary into the option universe, and
at resolve time an unbound declared facet **auto-binds to its `default`** — so a
user who selects nothing still gets a valid resolution, and the resolved output
records the auto-bind under `defaulted_choices`. Declare a facet only when you
want its full domain or its default to be first-class; leaving a facet implicit
keeps today's inferred behavior — unless you want to write a policy constraint
over it, which requires a declaration (see "Declaring a policy constraint"
below). Every value your conditions reference must be in a closed facet's
`values`, and `default` must be one of them — the compiler re-checks both.

A facet **key** must be a snake_case identifier — a lowercase letter, then
lowercase letters, digits and single underscores, with at most one trailing
underscore — and so must a binding id, a catalogue id and a catalogue entry id;
a facet **value** must be a non-empty token of letters, digits, `_`, `.` and `-`
(so `eu-west-1` and `1.5` are fine, but a space, a quote or a non-ASCII
character is not). The compiler writes every one of these into a condition
clause of its own, so it refuses anything else when it reads the model rather
than emitting one that would misread it.

### The default context

Every profile must provide a `default_context` that assigns a value to each
facet. This is the starting point for selection. For an implicit facet it must
name a value the model can resolve; for a declared facet you may instead rely on
its declared `default`, which auto-binds when the facet is left unbound. The
compiler and interpreter use the default context as the baseline.

### Declaring a policy constraint

Facets say what a user may choose. A **constraint** says which *combinations*
of those choices are legal — "debug logging is not permitted in production".
Constraints are pack-global — any chunk may declare them, and an id may be
declared only once across the pack — but by convention they live in the same
`00_definitions` chunk that owns your facets (ADR-0054):

```cue
constraints: {
    prod_forbids_debug: {
        condition: "environment != 'prod' || log_level != 'debug'"
        doc:       "Debug logging is not permitted in production."
    }
}
```

A constraint is a named rule with an id of its own, so it can be documented and
referred to rather than smuggled into the model as a component that exists only
to hold a condition string. The expression uses exactly the condition grammar of
section 6 — there is no second language — and the rule it expresses is a single
sentence: every declared constraint must hold in every resolved configuration.

The compiler checks each declaration when you compile: the expression must parse
(an unparseable constraint is a compile error, not a silently ignored rule),
every facet it names must be **declared** under `facets`, and every value it
names must be in a closed facet's `values`. That last check covers `!=` as well
as `==`, because against a closed domain a mistyped `environment != 'prod0'` is
not a harmless no-op — it is always true, and would quietly void the rule.

The declaration requirement is worth calling out, because it is the one thing a
constraint needs that a `condition` does not. A facet you only ever compare
against in conditions is fine as an implicit facet — but the moment a constraint
names it, you have to declare it with its value domain. Only a declared facet
gets the mutual-exclusion clauses that make `cfx options` and `cfx select`
enforce a rule the same way `cfx resolve` does; without them a rule like
`arch == 'x86'` would still leave `arch=arm` on offer while `resolve` refused
it. Rather than half-apply the rule, the compiler rejects the model and tells
you which constraint and which facet to fix.

**Where a constraint is enforced.** A declared constraint is compiled into the
solver model, and the selection surfaces screen against it. `cfx options` offers
a value only if some configuration satisfying every constraint still contains it
— pick `environment=prod` in the example above and `log_level=debug` disappears
from the offered options. `cfx explain` names the blocking constraint by its id
and quotes its condition when it reports why a combination is impossible:
`blocked by constraint prod_forbids_debug: environment != 'prod' || log_level
!= 'debug'`. A constraint is not advisory: declaring one changes what the tools
will let a user pick. And `cfx resolve` refuses a selection that violates one —
exit `3`, naming the constraint — so a selection your own code assembles, rather
than walking out of `cfx options`, is screened too.

If a combination is impossible but no constraint you declared rules it out, that
is not a policy violation and `cfx explain` will not pretend otherwise: it
reports the model as over-constrained instead of naming the nearest constraint.
Choosing two values for one facet is the everyday case — a facet holds exactly
one value, which is the model's own structure rather than something you wrote.

## 4b) Catalogues and Bindings: Shared Things, Chosen Once

Section 3 covered definitions, which describe a parameter's *shape* and
deliberately carry no value. Sometimes what you need to share is not a shape but
a **thing**: the physical containers your plant runs on the line, the firmware
modes your device supports, the motor variants a machine can be built from. Each
of those is a small table of named entries with real values, and several
services need to agree on which entry they are using.

### Declaring a catalogue

A **catalogue** is a typed table. You declare its columns once and every entry
supplies all of them:

```cue
catalogues: {
    containers: {
        doc: "The physical containers this plant runs on the line."
        fields: {
            height_mm: {type: "integer", unit: "mm", doc: "Internal height"}
            width_mm: {type: "integer", unit: "mm", doc: "Internal width"}
        }
        entries: {
            c1: {height_mm: 1000, width_mm: 800}
            c2: {height_mm: 900, width_mm: 600}
            c3: {height_mm: 400, width_mm: 400}
        }
    }
}
```

A field's `type` is one of `integer`, `float`, `boolean`, `string`. The compiler
checks every entry against those declarations: an entry that omits a field,
carries a field the catalogue never declared, or gives a field a value of the
wrong type is `E_CATALOGUE_INVALID`, naming the catalogue, the entry, and the
field. An integer is accepted where a `float` is declared — `1000` and `1000.0`
are the same number — and nothing else is promoted, so a typo cannot slip
through as data.

**Any unit may declare a catalogue** (see section 8 for what a unit is), and
exactly one chunk declares each — a second declaration is
`E_INGEST_DUPLICATE_CATALOGUE`. A shared unit declares the plant's containers; a
service unit declares the modes its own firmware supports and offers them to the
integrator; the integration unit may declare its own. The level is simply which
unit's chunk carries the declaration.

### Binding an entry

A catalogue on its own is a table nobody has chosen from. A **binding** is one
shared choice of an entry:

```cue
bindings: {
    line_container: {
        catalogue: "containers"
        default:   "c2"
        doc:       "The one container every service on this line draws from."
    }
}
```

**A binding is a facet.** That sentence is the whole design, and it is literal:
a binding is a declared *closed* facet whose options are its catalogue's entry
ids. It shares the facet id space, so naming a binding after a facet is
`E_INGEST_DUPLICATE_FACET`. Everything you already know about facets applies
unchanged — bind it in an environment's `choices` or `context_tags`, write
constraints over it, and see it in the listing:

The pack behind this listing is `compiler/scenarios/s_catalogue_binding`,
compiled into `build-cat/`.

All three listings in this section come from small packs that ship in the
repository. Run these three commands from the repository root to produce the
model packages they read:

```bash
configflux-compiler compile \
  --source compiler/scenarios/s_catalogue_binding/00_definitions.json \
  --source compiler/scenarios/s_catalogue_binding/10_components.json \
  --out build-cat

configflux-compiler compile \
  --source compiler/scenarios/s_catalogue_binding_accepts/00_definitions.json \
  --source compiler/scenarios/s_catalogue_binding_accepts/10_components.json \
  --out build-cat-accepts

configflux-compiler compile \
  --source compiler/scenarios/s_requires_accepts/00_catalogue.json \
  --source compiler/scenarios/s_requires_accepts/10_bindings.json \
  --source compiler/scenarios/s_requires_accepts/20_components.json \
  --out build-cat-derive
```

```console
$ cfx options --model build-cat/cmp.manifest.json
facet line_container [closed, default: c2]
  c1
  c2
  c3
```

Entries are offered in id-ascending order. That is the catalogue's declaration
order as the wire format carries it: the exported JSON does not preserve the
order you typed, so ids are what the ordering is derived from.

Two components that read the same binding receive the same entry by
construction. That is how you say "these must match" — you do not write a rule,
you name one binding instead of two.

### Deriving the entry from another facet

Sometimes the choice is not free: at factory A it is always container 1. Write
that as a `derive` table instead of a default:

```cue
facets: {
    site: {values: ["factory_a", "factory_b"], default: "factory_a"}
}

bindings: {
    line_container: {
        catalogue: "containers"
        derive: {site: {factory_a: "c1", factory_b: "c2"}}
    }
}
```

The source may be any declared facet or binding, and there is exactly one of
them. The table may be **partial**: a source value it does not cover implies
nothing, so you can pin the two sites that matter and leave the rest open.
`default` and `derive` are mutually exclusive — a derive table already fixes the
entry — and declaring both is `E_BINDING_INVALID`, as is naming a catalogue that
does not exist, defaulting to an entry it does not contain, deriving from
something undeclared, or using a key or entry outside the relevant domain. Each
message names the binding and the half that did not resolve.

### Declaring what a component needs

A binding says which entry the line uses. It does not say who reads it. A
component declares that with `requires`, naming a **slot** — the role the entry
plays inside that component:

```cue
components: {
    vision_service: {
        type: "service"
        requires: {container: "line_container"}
    }
}
```

A requirement is the only way a component receives a catalogue entry, so a
service that forgets to declare its need has nothing to read and the compiler
says so rather than letting the failure move to startup. Naming a binding no
chunk declares is `E_REQUIRES_INVALID`, and the message names the component and
the slot, not just the missing binding.

A requirement is **not** a `depends_on` edge. `depends_on` says "this component
needs that component to exist"; `requires` says "this component needs a choice
someone made". Requirements do not join the dependency closure and do not affect
build order.

### Narrowing a binding to the entries you support

Not every service works with every entry. Say so with `accepts`:

```cue
components: {
    vision_service: {
        type: "service"
        requires: {container: {binding: "line_container", accepts: ["c1", "c2"]}}
    }
}
```

Now `c3` is not offered at all:

This narrowed model is `compiler/scenarios/s_catalogue_binding_accepts`,
compiled into `build-cat-accepts/`.

```console
$ cfx options --model build-cat-accepts/cmp.manifest.json
facet line_container [closed, default: c2]
  c1
  c2
```

And a forced choice is refused by the requirement's own name:

```console
$ cfx explain --model build-cat-accepts/cmp.manifest.json --select line_container=c3
cannot select line_container.c3:
  blocked by requirement vision_service.container: accepts c1, c2
(one minimal explanation; other minimal cores may exist)
```

Leaving `accepts` out means every entry is acceptable. That is not the same as
writing an empty list, which is refused: a component that accepts nothing can
never be satisfied. Every accepted entry must be an entry of the binding's
catalogue, with no repeats.

When several components require the same binding, the integrator is offered
exactly the **intersection** of their lists. That is the point — it is how the
model knows which containers the whole line can actually run. If the
intersection is empty the model can never resolve, so the compiler refuses it up
front with `E_BINDING_NO_ACCEPTABLE_ENTRY`, naming the binding and every list so
you can see which one to widen.

A component with a `condition` narrows nothing while it is excluded. Its
`accepts` list applies only where the component is included, exactly as its
parameters do.

### Where a derived entry is explained

The same naming applies to a `derive` table. Selecting a site that fixes the
container and then asking for a different one is refused by the rule that fixed
it:

The derived model is `compiler/scenarios/s_requires_accepts`, compiled into
`build-cat-derive/`.

```console
$ cfx explain --model build-cat-derive/cmp.manifest.json --select site=factory_a --select line_container=c2
cannot select site.factory_a:
  blocked by your earlier choice: line_container.c2
  blocked by binding line_container, derived from site: site == 'factory_a' -> 'c1'
(one minimal explanation; other minimal cores may exist)
```

You never wrote that rule as a constraint — you wrote a derive table — and the
tool still names it in the vocabulary you used.

### What the service actually reads

The requirement is not only a check. Resolve delivers the entry the binding took
**inside the requiring component**, so the service reads its own configuration:

```json
"components": {
  "vision_service": {
    "requires": {
      "container": {
        "binding": "line_container",
        "entry": "c1",
        "fields": {"height_mm": 1000, "length_mm": 1200, "width_mm": 800}
      }
    },
    "params": {"...": "..."}
  }
}
```

`vision_service` reads `requires.container.fields.width_mm`. It never names the
catalogue, never names the unit that owns it, and does not change when the plant
reorganises. Scoping the resolve to that one service changes nothing: a
requirement is not a dependency edge, so the catalogue is not in the service's
closure, and the entry still arrives.

Two consequences worth knowing. A component that declares no requirement
carries no `requires` key at all, so read it as absent rather than empty. And at
the runtime these values are read-only, at
`component.<service>.requires.<slot>.<field>` — they were decided when the
deployment was resolved, so a write to one is refused.

## 5) Artifact References

An **artifact** represents an external deliverable that the configuration
selects but does not contain: a driver binary, a firmware image, a calibration
file.

### Declaring artifacts

Artifacts are declared at the top level of a chunk under `artifacts`, alongside
components and definitions. `#Artifact` requires `name`; `version`, `hash`,
`source`, `target`, and `doc` are optional:

```cue
artifacts: {
    hydra_x200_single_driver: {
        name:    "hydra_x200_single_driver"
        version: "1.0.0"
        hash:    "sha256-hydra-x200-single"
        source:  "artifact://drivers/hydra_x200_single.so"
        target:  "/opt/configflux/drivers/hydra_x200_single.so"
        doc:     "Hydra X200 single-pump driver"
    }
}
```

| Field | Required | Description |
|---|---|---|
| `name` | yes | Human-readable artifact name |
| `version` | no | Artifact version string |
| `hash` | no | Content hash for integrity verification |
| `source` | no | Where to fetch the artifact from |
| `target` | no | Where to deploy the artifact on the target system |
| `doc` | no | Documentation string |

### Referencing artifacts from parameters

A parameter with `type: "artifact"` holds an artifact id as its value. The
definition pattern works well here -- declare the slot once, then point each
component at the artifact it selects:

```cue
// In 00_definitions.cue
definitions: {
    driver_slot: {
        type:      "artifact"
        lifecycle: "construction"
        access:    "developer"
        doc:       "Control driver artifact reference"
    }
}
```

```cue
// In 10_components.cue
components: {
    controller: {
        type: "module"
        params: {
            control_driver: {
                inherits: "driver_slot"
                value:    "hydra_x200_single_driver"
            }
        }
    }
}
```

### When to use artifacts

- The selected item is a binary, firmware image, or external file -- not a
  simple parameter value.
- You need to track version, hash, and deployment path alongside the selection.
- Different product variants select different binaries.

## 6) Conditional Components: Region and Variant Gating

A **condition** on a component controls whether it appears in the resolved
output. If the condition evaluates to false for the current selection context,
the component is removed entirely.

### Component conditions

```cue
components: {
    eu_label: {
        type:      "label_module"
        condition: "region == 'eu'"
    }
}
```

This component only exists when the `region` facet is set to `eu`. For any
other region value, it is pruned from the resolved model. The shipping
`examples/02-sensor-gateway` pack uses the same pattern for its
`network_monitor` component, which is gated on `bus_type == 'ethernet'`.

### Parameter overrides

Conditions also apply to parameter values. A parameter can have a base value
and one or more **conditional overrides** that replace the value when their
condition matches. Each override is an entry in the `overrides` list with a
required `condition` and a `value`:

```cue
components: {
    controller: {
        type: "module"
        params: {
            control_driver: {
                inherits: "driver_slot"
                value:    "hydra_x200_single_driver"
                overrides: [
                    {
                        condition: "cooling_brand == 'hydra' && cooling_model == 'x200' && pump_type == 'dual'"
                        value:     "hydra_x200_dual_driver"
                    },
                    {
                        condition: "cooling_brand == 'aeroflux' && cooling_model == 'a9'"
                        value:     "aeroflux_a9_driver"
                    },
                ]
            }
        }
    }
}
```

`condition` strings and `overrides` blocks are opaque to the CUE layer -- it
does not interpret them. They pass through verbatim to the compiler and solver,
which evaluate them during resolution.

An override condition is a **branch selector**, not a rule about what is legal:
it picks which value applies when it matches, and it does not restrict what a
user may select. Writing `condition: "environment == 'prod'"` on an override
says "use this value in prod", never "the selection must be prod".

### Condition syntax

Conditions are boolean expressions using the facet names defined in your
selection domains. The supported operators are:

| Operator | Example |
|---|---|
| `==` (equality) | `region == 'eu'` |
| `!=` (inequality) | `region != 'us'` |
| `&&` (logical AND) | `brand == 'hydra' && model == 'x200'` |
| `\|\|` (logical OR) | `region == 'us' \|\| region == 'ca'` |

String literals in conditions must be quoted. Inside a CUE string, use single
quotes for the literal (as in the examples above) so they do not collide with
the double quotes delimiting the CUE string.

### Rules the compiler enforces

- **Condition compatibility**: If component A `depends_on` component B, then
  A's condition must logically imply B's condition. You cannot have a
  conditional component depend on another component that might not exist.
- **Facet coverage**: Every facet used in a condition must be defined in the
  selection domains, so the selection system can evaluate it.

### Conditions gate inclusion; constraints state policy

A condition answers "is this component (or this value) part of the resolved
configuration?" If what you actually want to say is "this combination of choices
is never legal", that is a **constraint** (section 4) — declare it under
`constraints` with an id of its own, rather than adding a component whose only
purpose is to carry the rule. A rule declared that way is named, documented, and
kept out of the component graph, so it never turns up in a resolved output, a
manifest, or a bill of materials.

## 7) Naming Conventions

ConfigFlux enforces **snake_case** for all authored identifiers -- definition
ids, component ids, artifact ids, parameter keys, and `depends_on` targets. The
schema's `#snakeId` constraint rejects any id that does not conform.

### Rules

- Only lowercase ASCII letters (`a-z`), digits (`0-9`), and underscores (`_`).
- Must start with a lowercase letter.
- No double underscore (`__`).
- A single trailing underscore is permitted (e.g., `foo_`), but is rarely
  useful -- prefer ids that read cleanly.

Because the keys of `definitions`, `components`, `artifacts`, and `params` are
all `#snakeId`-constrained inside closed structs, a key that violates the rule
is a hard "field not allowed" error at export time.

### Naming patterns by entity

| Entity | Convention | Examples |
|---|---|---|
| **Package** | `domain_name` (snake_case) | `hello_led`, `sensor_gateway`, `building_hvac` |
| **Chunk files** | `NN_purpose.cue` (zero-padded, ordered) | `00_definitions.cue`, `10_components.cue` |
| **Definitions** | Describe the shared concept | `safe_flow`, `driver_slot`, `trim_gain` |
| **Components** | Name the logical part | `thermal_control`, `power_bus`, `eu_label` |
| **Parameters** | Name the specific setting | `max_flow_at_commissioning`, `control_driver` |
| **Artifacts** | Name the deliverable | `hydra_x200_single_driver`, `office_standard_controller` |
| **Facets** | Name the axis of variation | `cooling_brand`, `region`, `safety_mode` |

## 8) Units and pack layout

### The chunk: package and `#Config`

Each authored CUE file is a **chunk**. A chunk declares the `configflux`
package and defines a top-level `chunk` bound to `#Config`:

```cue
package configflux

chunk: #Config & {
    package: "my_domain"
    version: "1.0.0"
    definitions: { /* ... */ }
    components: { /* ... */ }
    artifacts: { /* ... */ }
}
```

`#Config` requires `package` and `version`; `definitions`, `components`, and
`artifacts` are all optional. `#Config` is closed -- a misspelled top-level key
(for example `defintions`) is a hard "field not allowed" error rather than a
silently ignored field.

To validate the chunk against the schema, you export it with both the chunk and
`compiler/cue/schema.cue` in scope (see section 9). The schema file supplies
`#Config`, `#snakeId`, and the rest of the constraints.

### The unit: the chunks that share one `package`

A **unit** is the set of chunks that declare the same `package` value. That is
the whole definition — the field every chunk already carries is the unit's name,
and nothing else marks the boundary.

Where a unit *lives* is a question about ownership rather than about the tool. A
folder inside a monorepo is a unit; so is a whole repository. A service is
usually one unit, a shared catalogue is one unit, and the integration that binds
everything together is one unit. `examples/06-catalogue-polyrepo` has four, one
per repository: `site_catalogue`, `vision_service`, `compute_service` and
`sorter_service`.

Inside a unit, the layout is the pack layout below: **one definitions chunk and
any number of components chunks**, all sharing the `package` value and exported
together, so CUE resolves their `inherits` pointers against that one definitions
chunk. A unit that inherits from *another* unit needs no definitions chunk of
its own — each of the three service units in example 06 is a single components
chunk that inherits from the catalogue unit.

An **interface unit** is not a separate kind of thing. It is a unit that
declares definitions, facets, catalogues, bindings and constraints and few or no
components — the header file of a model. Service units are compiled *against* it
and inherit from its definitions; the catalogue repository in example 06 is one.

The unit matters because it is the granularity at which a model can be built:
one unit compiles into one object, and the objects are linked. Step 2b below is
that build, and the one-shot `compile` at Step 2 is its shortcut for a checkout
that holds every unit at once.

### A pack: definitions plus components

A **pack** is the set of chunks in one `cue/` directory. The canonical layout
splits a model across two files:

- `00_definitions.cue` -- the inheritance roots (definitions only, valueless).
- `10_components.cue` -- components and artifacts; component parameters inherit
  from the definitions via `inherits` and supply their own `value`.

The numeric prefix orders the files for readability. Both chunks declare the
**same** `package` and `version`. The `examples/02-sensor-gateway` pack is a
worked instance of this layout: a `00_definitions.cue` declaring
`poll_interval_ms`, `protocol`, and `buffer_depth`, and a `10_components.cue`
with three components that inherit from them.

### Single-file packs

For a very small model you can author definitions and components in a single
chunk. The `examples/01-hello-led` pack does exactly this in one
`cue/config.cue`: one definition, one component, one parameter. Use the
two-file layout once a model grows past a handful of entities. Section 9 gives
a separate export recipe for the single-file case, since its resolved output is
one combined `config.json` rather than a per-file pair.

### Packs with more than two chunks, and chunks in several repositories

Nothing about a pack is limited to two files. A pack has **one** definitions
chunk and **any number** of components chunks, and those chunks do not have to
sit in one directory -- or in one repository. A shared catalogue can be owned by
one team in one repository while each service that uses it lives in its own,
and the compiler takes each repository's exported chunk as a `--source`.

The rules that actually apply:

- **One definitions chunk per pack.** It holds the inheritance roots; every
  components chunk resolves its `inherits` pointers against it, wherever that
  chunk lives.
- **Any number of components chunks.** Order does not matter.
- **Ids are unique across ALL chunks.** Two chunks declaring the same component,
  definition, facet, or constraint id is a hard error, not a merge.
- **Every `depends_on` target must be in the compile set.** A chunk whose
  dependency was not handed to the same `compile` invocation fails with
  `E_UNKNOWN_COMPONENT_DEP`. This is deliberate: the tool will not certify a
  fragment whose dependency it has never seen.
- **Every `requires` binding must be in the compile set too.** A component that
  requires a binding no chunk in the invocation declares fails with
  `E_REQUIRES_INVALID`, naming the component, the slot and the binding. Same
  rule, applied to the shared choice rather than to a sibling component.
- **`package` names the unit; `version` is a label.** The merged root is
  synthetic, so chunks may carry different `package` values and a compile
  accepts them: it groups the chunks by that value into units and links them.
  Nothing cross-checks a `version` against anything.
- **The compile set is the unit of verification.** A dependency-closed subset is
  a perfectly good model -- a different one, with its own `model_hash`, because
  it contains different components.
- **A worktree or another checkout path does not change `model_hash`.** Identity
  is content-canonical, so the same chunks compiled from a git worktree, a CI
  checkout, or a vendored copy produce the same hash and remain comparable
  across machines.

`examples/06-catalogue-polyrepo` is a worked instance: a catalogue repository
that declares the shared definition, the site facet, the container catalogue and
the binding that derives one from the other, plus two service repositories that
inherit from it and declare what they `require`. Section 9 exports an N-chunk
pack like it with a single command.

### Directory structure

A self-contained domain typically looks like this:

```
my_domain/
  cue/
    00_definitions.cue        # definitions (valueless inheritance roots)
    10_components.cue         # components, params, artifacts, overrides
  00_definitions.json         # exported, inheritance-resolved (see section 9)
  10_components.json          # exported, inheritance-resolved
```

The `.cue` files are the human-authored source. The `.json` files are the
exported, inheritance-resolved output that the compiler ingests. You commit
both: the JSON is what compiles, and an export check keeps it in sync with the
CUE source.

## 9) Validating, Exporting, and Compiling

### Step 1: Export CUE to inheritance-resolved JSON

The compiler ingests JSON, not CUE. Crucially, it ingests the
**inheritance-resolved** JSON -- the JSON in which every `inherits` pointer has
already been gap-filled with its definition's `type`, `unit`, `safety`,
`lifecycle`, `access`, `limits`, and `doc`.

A bare `cue export -e chunk file.cue schema.cue` emits the *raw* chunk: it
contains only `inherits` and `value`, with no gap-fill. That is **not** what the
compiler should ingest. Resolution happens by running the whole pack through the
schema's `#ResolvePack` engine, which gap-fills every component parameter
against the pack's definitions chunk.

`#ResolvePack` (defined in `compiler/cue/schema.cue`) takes the pack's
definitions map and components map, gap-fills each component parameter against
the definitions, and exposes a resolved `definitions` and `components`. You do
not have to wire that up by hand: `examples/export_pack.sh` is a reference
exporter that does it for a pack of any size, and it runs entirely against your
own directory -- nothing in it is tied to this repository's layout.

What the script does, so the mechanism stays visible: it exports every chunk
*raw* (`cue export -e chunk`), wraps each raw chunk into its own field of a
generated driver, and resolves each components chunk with its own
`#ResolvePack` against the **one** definitions map the pack declares. It then
emits one JSON per input chunk, named after that chunk -- so attribution stays
per-file and no definition leaks into a components chunk. The wrap-then-resolve
detour is needed because every chunk file binds a top-level `chunk`; exporting
each one raw first is what lets them occupy distinct fields in a single
resolving evaluation.

> The in-repo helper `compiler/cue/export_fixtures.sh` is **not** the command to
> export your own pack. It is a drift guard/regenerator for the example and
> scenario fixtures that ship *inside this repository*: its pack roots are
> hard-coded to `compiler/scenarios` and `examples`, so it never discovers a
> `my_domain/` you create elsewhere. Use it only to check or regenerate the
> in-repo fixtures (see the end of this section). For your own pack, use
> `examples/export_pack.sh`.

#### Two-file pack: export `00_definitions.json` and `10_components.json`

```bash
examples/export_pack.sh \
  --schema compiler/cue/schema.cue \
  --definitions my_domain/cue/00_definitions.cue \
  --components  my_domain/cue/10_components.cue \
  --out my_domain
```

`my_domain/00_definitions.json` and `my_domain/10_components.json` are now the
inheritance-resolved JSON the compiler ingests. Outputs are named after their
input chunk, so `00_definitions.cue` becomes `00_definitions.json`, and no
scratch files are left behind.

A resolved component parameter carries the inherited fields inline. For example,
the `protocol` parameter in `examples/02-sensor-gateway` -- authored with just
`inherits`, `value`, and `overrides` -- resolves to:

```json
"protocol": {
  "inherits": "protocol", "type": "string", "lifecycle": "construction",
  "access": "developer", "doc": "Field-bus protocol used by the gateway",
  "value": "modbus_rtu",
  "overrides": [{ "condition": "bus_type == 'ethernet'", "value": "modbus_tcp" }]
}
```

The `type`, `lifecycle`, `access`, and `doc` were filled in from the `protocol`
definition; the `value` and `overrides` came from the component parameter.

#### Packs with more chunks, including chunks in several repositories

`--components` is repeatable, and the paths are ordinary paths -- they can point
anywhere, including into another repository's checkout. Every components chunk
is resolved against the same definitions chunk, so a parameter in one repository
can inherit a definition declared in another:

```bash
examples/export_pack.sh \
  --schema compiler/cue/schema.cue \
  --definitions repos/catalogue/cue/00_catalogue.cue \
  --components  repos/vision/cue/10_vision.cue \
  --components  repos/compute/cue/10_compute.cue \
  --out <dir>
```

Each chunk's JSON lands in `<dir>` named after its input, and you commit it next
to the chunk it came from. Two chunks may not share a basename -- their outputs
would collide, so the script refuses rather than emitting fewer files than the
pack has chunks. `examples/06-catalogue-polyrepo` is exactly this shape; see
section 8 for the rules that govern a pack spread across repositories.

#### Single-file pack: export one `config.json`

If you authored a single-file pack (one chunk holding both definitions and
components, like `examples/01-hello-led/cue/config.cue`), pass it as `--single`:

```bash
examples/export_pack.sh \
  --schema compiler/cue/schema.cue \
  --single my_domain/cue/config.cue \
  --out my_domain
```

`my_domain/config.json` is the resolved JSON the compiler ingests.

#### Which `cue` the script uses

Inside this repository the script resolves the hermetic, version-pinned `cue`
for you. Outside it, set `$CUE` to the evaluator you trust:

```bash
CUE=/path/to/cue examples/export_pack.sh --schema ... --out ...
```

There is deliberately no `$PATH` fallback: an unpinned evaluator can change the
compiled `model_hash`, so an unresolvable `cue` is an error rather than a
warning. The script exits `0` on success, `1` when `cue` rejects a chunk (its
own stderr is forwarded unchanged), and `2` on a usage error.


#### Checking the in-repo fixtures (not your own pack)

To verify that *this repository's* example and scenario JSON still matches a
fresh export -- a drift guard for the fixtures under `compiler/scenarios` and
`examples` -- run the in-repo helper with `--check`:

```bash
CUE=/path/to/cue compiler/cue/export_fixtures.sh --check
```

This does not touch or discover your own `my_domain/`; it only inspects the
fixtures that ship inside the repository. See `compiler/cue/README.md` for the
front-end's schema and verification harness.

### Step 2: Compile the resolved JSON into a CMP

Once you have resolved JSON, compile it with `configflux-compiler`. The compiler
auto-detects format: a source whose content starts with `{` is parsed as JSON,
so no format flag is needed. Pass one `--source` per chunk and an `--out`
directory:

```bash
configflux-compiler compile \
  --source 00_definitions.json \
  --source 10_components.json \
  --out out/cmp
```

`compile` prints a JSON result to stdout. A successful run looks like:

```json
{ "schema_version": 5, "status": "ok",
  "model_hash": "89d1692f...6352f3",
  "compiled_model_package_ref": "out/cmp/cmp.manifest.json",
  "stats": { "source_count": 2, "chunk_count": 2, "definition_count": 3, "component_count": 3, "artifact_count": 0 } }
```

The `--out` directory then contains the Compiled Model Package:

- `cmp.manifest.json` -- package manifest with the model hash and stats
- `index.cfir.json` -- component, definition, and artifact indices
- `chunk-<hash>.cfir` -- one IR chunk per source file
- `ccm/` -- the compiled solver form

The compiler exits `0` on success, `1` on a user or input error, and `2` on a
compilation error.

### Step 2b: Compiling one unit

Step 2 needs every chunk of the model on one command line, so it needs one
checkout that holds every unit at once. That is the common case and it is why
`compile` exists — but it is a *shortcut*, not the shape of the build. The
general form is the next two steps: **compile each unit into an object, then
link the objects.** `compile` is that same linker, fed by objects it builds in
memory and never writes, which is why the two produce identical bytes.

Read 2b and 2c as one story. Nothing in 2b is finished until 2c runs: a unit
compiled alone records what it could not resolve rather than rejecting it, and
the link is where those obligations are settled.

`compile-object` compiles one unit -- the chunks sharing one `package` value, as
section 8 defines it -- into a content-addressed object:

```bash
configflux-compiler compile-object \
  --source repos/catalogue/00_catalogue.json \
  --out out/site_catalogue.cfo
```

```text
object: out/site_catalogue.cfo  unit: site_catalogue  object_hash: 24d934a...  chunks: 1  exports: definitions=1 components=0 artifacts=0 facets=1 bindings=2 catalogues=1 constraints=0  imports: components=0 definitions=0 facets=0 bindings=0 catalogues=0
```

Add `--format json` to print the object header instead of the summary line.

A unit that references another unit's declarations is compiled **against** that
unit's object, with `--interface`. Only the interface's `object.json` is read:

```bash
configflux-compiler compile-object \
  --source repos/vision/10_vision.json \
  --interface out/site_catalogue.cfo \
  --out out/vision_service.cfo
```

The object directory holds the header, the unit's chunk files exactly as a CMP
would store them, and a provenance sidecar. There is no constraint model and no
package index in an object -- those come from linking.

Two rules are worth knowing before you use this:

- **One object is one unit.** If the `--source` chunks do not all declare the
  same `package`, the compile is refused with `E_OBJECT_UNIT_MISMATCH`, naming
  both values and both files.
- **Unresolved references are recorded, not rejected.** A `depends_on` or
  `inherits` target, a required binding, or a facet that neither this unit nor
  its interfaces declare is written into the header's `imports` and checked when
  the objects are linked. The `--interface` list changes what can be *checked*
  here, never what is *recorded*: compiling `vision_service` with and without the
  catalogue interface produces the same `imports`.

`compile-object` exits `0` on success, `1` on a user or input error (an
unreadable `--source` or `--interface`), and `2` on a compilation error.

### Step 2c: Linking units

`link` turns a set of objects into a Compiled Model Package -- the same package
`compile` writes. Pass one `--object` per unit and an `--out` directory:

```bash
configflux-compiler link \
  --object out/site_catalogue.cfo \
  --object out/vision_service.cfo \
  --out out/cmp
```

```text
model_hash: 1d9803bc...  objects: 2  partitions: 1
```

Add `--format json` to print the full compile result instead of the summary
line. `link` takes the same `--cluster-size`, `--max-rss-mb`, `--max-threads`,
`--progress` and `--stamp-time` flags as `compile`, and they mean the same
thing: the constraint model is built by the link, so the flags that shape it —
and the one that reports on it — belong here.

Argument order does not matter. Objects are linked by unit name, so the same
objects in any order produce the same bytes.

**What `link` checks first, from the headers alone.** Reading an object's
header costs one small file, so every cross-unit question is answered before a
single chunk file is opened:

- Two objects claiming the same unit -- `E_LINK_DUPLICATE_UNIT`.
- Two objects exporting the same id -- `E_LINK_DUPLICATE_ID`, naming both units.
- An import no linked object declares -- `E_LINK_UNRESOLVED_IMPORT`, naming the
  unit and the id: `unit 'sorter_service' requires binding 'sorter_container';
  no linked object declares it`.
- A unit compiled against a different version of an interface than the one being
  linked -- `E_LINK_INTERFACE_MISMATCH`, naming the hash it was compiled against
  and the hash linked. Recompile the dependent unit, or link the object it was
  compiled against.

Then the chunk files are checked against the header that names them and against
themselves -- a chunk file is named by its content, so a body that no longer
hashes to its own name is refused even when the header still agrees with it --
and the header is checked back against those chunk files, rebuilt from them, so
an object whose header says something its own chunks do not say is refused even
though the header itself is consistent. The chunks are then copied
(`E_LINK_OBJECT_CORRUPT` otherwise). Nothing is written under `--out` unless
every check passes.

The `E_LINK_*` codes are what the same faults are called when the linker may
hold only *some* of the objects. A one-shot `compile` holds the whole model, so
it still reports `E_UNKNOWN_COMPONENT_DEP` and `E_REQUIRES_INVALID` for the two
cases named in section 8. Neither form accepts a model the other rejects.

**The package is the same package.** For the same model, `compile` and
`compile-object` per unit followed by `link` produce byte-identical output --
the chunk files, the index, the manifest and the constraint model. That is what
makes the split worth using: an unchanged unit's object is reused as it is, only
the unit you edited recompiles, and the link runs over headers rather than over
every parameter in the model.

So choosing between the two forms is a question about your build, never about
your model. Take Step 2's shortcut while one checkout holds everything. Reach
for 2b and 2c when it does not -- separate repositories, a job that rebuilds one
unit, an agent working in a worktree of a single service -- and when you want a
mistake between units named at link time instead of found later.
`examples/06-catalogue-polyrepo` runs both forms over one model and asserts,
file by file, that the packages are identical.

**Pinning what you link.** Once several units are in play, the question a
release asks is not "does this link" but "is this the set we reviewed". A
lockfile answers it. `--write-lock` records, per unit, the object hash the link
used; `--lock` refuses a later link whose objects do not match those pins:

```bash
# Record what this link used.
configflux-compiler link \
  --object out/site_catalogue.cfo \
  --object out/vision_service.cfo \
  --out out/cmp --write-lock configflux.lock

# Later, or on another machine: build exactly that, or fail.
configflux-compiler link \
  --object out/site_catalogue.cfo \
  --object out/vision_service.cfo \
  --out out/cmp --lock configflux.lock
```

A pin that does not match the object being linked is `E_LINK_LOCK_MISMATCH`,
naming the unit and both hashes; a pinned unit that was not linked is
`E_LINK_LOCK_UNLINKED`, waived by `--lock-allow-extra` when the subset is
deliberate. The compiler **checks** pins and never fetches anything -- getting
the objects to the machine is your checkout's or your CI's job, and the
`source` field is a note for humans that nothing reads. The full format is in
[the interface contracts](interface-contracts.md), and the workflow it exists
for is in [the service integration guide](service-integration-guide.md).

`link` exits `0` on success, `1` on a user or input error, and `2` on a link
error.

### Step 3: Verify and inspect (optional)

`verify` runs every validation check without emitting a CMP. Use it during
authoring to catch problems early:

```bash
configflux-compiler verify \
  --source 00_definitions.json \
  --source 10_components.json
```

Validation covers: JSON parse validity, snake_case id enforcement, definition
inheritance integrity (targets exist, no cycles), component dependency
validation (targets exist, no cycles; any DAG including diamonds is allowed),
and condition compatibility (dependent components have compatible conditions).

`inspect ... summary` prints an overview of the merged model:

```bash
configflux-compiler inspect \
  --source 00_definitions.json \
  --source 10_components.json \
  summary
```

Use `inspect` to confirm the merged model looks correct before moving on to
selection and resolution in section 11.

## 10) From an Empty Directory to a Resolved Configuration

Here is the full sequence for modeling a new domain from scratch. Every command
below is one you run directly; substitute your own pack directory and file
names.

```bash
# 0. One-time: build both commands and put them on your PATH.
bazel build //compiler:compiler //cfx:cfx
mkdir -p .cfx-bin
ln -sf "$PWD/bazel-bin/compiler/compiler" .cfx-bin/configflux-compiler
ln -sf "$PWD/bazel-bin/cfx/cfx"           .cfx-bin/cfx
export PATH="$PWD/.cfx-bin:$PATH"

# 1. Author the pack. Create my_domain/cue/ and write two chunks:
#      my_domain/cue/00_definitions.cue   (definitions; valueless)
#      my_domain/cue/10_components.cue     (components + artifacts; params inherit)
#    Each file is `package configflux` and defines `chunk: #Config & {...}`,
#    sharing the same package + version.

# 2. Export CUE -> inheritance-resolved JSON. Needs the pinned cue v0.16.1.
#    Follow the self-contained recipe in section 9, Step 1: emit each chunk
#    raw, wrap the raw chunks, and resolve them through #ResolvePack via a small
#    my_domain/_export.cue driver. The two-file pack produces:
#      my_domain/00_definitions.json
#      my_domain/10_components.json
#    (A single-file pack instead produces one my_domain/config.json.)
#    Do NOT use compiler/cue/export_fixtures.sh for this -- it only regenerates
#    the repo's own in-repo fixtures and will not see my_domain/.

# 3. Compile the resolved JSON into a CMP.
configflux-compiler compile \
  --source 00_definitions.json \
  --source 10_components.json \
  --out out/cmp

# 4. (Optional) Verify and inspect.
configflux-compiler verify \
  --source 00_definitions.json \
  --source 10_components.json
configflux-compiler inspect \
  --source 00_definitions.json \
  --source 10_components.json \
  summary

# 5. Resolve the CMP down to one concrete configuration.
#    Substitute your own facet names and values (section 4).
cfx options --model out/cmp/cmp.manifest.json
cfx resolve --model out/cmp/cmp.manifest.json \
  --select my_facet=my_value \
  --out out/snapshot
```

Step 3 produces your Compiled Model Package under `out/cmp/` — the 150% model.
Step 5 turns it into a resolved snapshot under `out/snapshot/`, the 100% model
for one selection. Section 11 walks through that last stage against a model you
can run today.

### The fastest start: run a shipping example

If you would rather see the pipeline work before authoring anything, run one of
the example packs end to end:

```bash
cd examples/00-service-multi-env && ./run.sh
```

Each example's `run.sh` locates the two binaries and drives them against the
example's already-exported JSON. The shipping examples, in increasing
complexity, are:

- `examples/00-service-multi-env/` -- the hero example: one model resolved
  across dev, staging, and prod, plus a compiled policy that `cfx explain`
  narrates.
- `examples/01-hello-led/` -- bare minimum: one definition, one component,
  single-file CUE chunk.
- `examples/02-sensor-gateway/` -- two-file pack: inheritance, conditional
  overrides, and a conditional component.
- `examples/03-motor-controller/` and `examples/04-fleet-edge-node/` -- larger
  models.

See `examples/README.md` for an overview of the example set.

## 11) Resolve Your First Configuration

A CMP is the **150% model**: every variant your chunks describe, merged and
validated. Resolution picks one point in that space and emits the **100%
model** — the resolved configuration for exactly one product, environment, or
deployment.

Three `cfx` commands cover the loop:

| Command | What it answers |
|---|---|
| `cfx options` | What can I choose, and what is still valid? |
| `cfx resolve` | Give me the resolved configuration for these choices. |
| `cfx explain` | Why is this combination impossible? |

The transcripts below run against the shipping `examples/02-sensor-gateway`
pack: the two-file layout from section 8, the declared facets from section 4,
and the conditional override from section 6. Run them verbatim to see the
shape, then substitute your own pack's JSON and facet names.

First compile the pack, exactly as in section 9:

```bash
configflux-compiler compile \
  --source examples/02-sensor-gateway/00_definitions.json \
  --source examples/02-sensor-gateway/10_components.json \
  --out build
```

### Step 1: See what is selectable

`cfx options` opens the compiled model and lists every facet with the options
currently valid for it. With no choices applied, this is the whole decision
space your model offers:

```console
$ cfx options --model build/cmp.manifest.json
facet bus_type [closed, default: serial]
  ethernet
  serial
facet environment [closed, default: standard]
  high_speed
  standard
```

Both facets read `[closed, default: ...]` because `00_definitions.json`
declares them with `#Facet` (section 4). `closed` means the listed values are
the entire vocabulary; `default` is the arm that auto-binds when the facet is
left unbound. A facet you never declared would appear here too, with its domain
inferred from the conditions that reference it.

Apply a choice with `--select FACET=OPTION`. The chosen facet is marked
`[selected: ...]`:

```console
$ cfx options --model build/cmp.manifest.json --select bus_type=ethernet
facet bus_type [selected: ethernet, default: serial]
  ethernet
facet environment [closed, default: standard]
  high_speed
  standard
```

Here `environment` is unchanged: `sensor_gateway`'s two facets vary
independently, so choosing a bus type rules nothing else out. In a model whose
facets are linked by a rule, the *other* facets' option lists shrink as choices
land — step 4 shows that case.

This is the guided-walk primitive: an interactive tool calls `cfx options`
after every choice, so a user is only ever offered options that can still lead
to a valid configuration. `cfx options` writes nothing — it is a read-only
query, safe to call as often as you like.

### Step 2: Resolve the configuration

Once the facets you care about are chosen, `cfx resolve` runs
open → select → resolve → export in one process. It prints the hash lineage and
the relative path of every exported file, then writes the snapshot under
`--out`:

```console
$ cfx resolve --model build/cmp.manifest.json --select bus_type=ethernet --select environment=high_speed --out snapshot
model_hash: 797d2d277eaf4893f430a11b457e0ebccb977f25827304e27fc5f4c08e80f083
selection_state_hash: 4a42c59c9a305b8cabd5169f1659b55291fc8f5aa35f323df268495ae60177fa
resolve_hash: eab7a9058c7ee71ed3599dad6aa612ea40e11806884cf2c0aa23a9a7d7cdb3e2
resolved_output_hash: 322004d7540db072501aeeb6e9582ebef2bf2d3f59e4ebadbf60fe3e6b6732a7
wrote: generated/config.hpp
wrote: generated/config_artifact_manifest.json
wrote: generated/config_build_flags.cmake
wrote: resolve_result.all.ethernet-high_speed.json
```

Four hashes. The first three are each derived from the one above it:
`model_hash` fingerprints the 150% model, `selection_state_hash` fingerprints
the choices, and `resolve_hash` fingerprints this exact resolved configuration.
`resolved_output_hash` stands apart — it fingerprints the delivered payload
alone, so it changes only when the bytes a consumer receives change. Run the
same command again and all four are identical and the exported bytes are
unchanged — that reproducibility is the contract.

**Bind the free decisions; the solver infers the rest.** You do not have to
select everything the model declares. When the constraints admit exactly one
value for a facet you left unstated, `cfx resolve` binds it for you and prints
it as an `implied:` line before any `defaulted:` lines, recording it under
`implied_choices` in the resolved output. If a rule says the `factory_b` site is
equipped for the `c2` container, then `--select site=factory_b` alone resolves
with the container implied — the declared default never applies, because the
constraints had already decided. Inference only fills a gap you left: an
explicit choice and a context tag both still win over an implied value, and
nothing is implied when more than one value remains admissible. So state the
decisions your deployment actually owns and leave the entailed ones to the
model; `cfx options` will show you which is which as you go.

**Named environments.** Typing `--select` flags is fine while you are exploring,
but a deployment target deserves a name you can version-control. Put your
targets in an **environment manifest** — a small JSON file mapping each name to
its scope, context tags, and choices — and `cfx resolve --manifest
environments.json --environment production` resolves that named target, or
`--all` resolves every target the file names in one pass. The manifest is a
`cfx` input, not a convention you have to unfold yourself. See
[Naming targets](service-integration-guide.md#naming-targets-the-environment-manifest-convention)
in the service integration guide for the schema and the matrix form.

### Step 3: Read the resolved snapshot

The snapshot is what a build or a runtime actually consumes. For the C++
early-binding profile the resolved parameters land in a header:

```console
$ cat snapshot/generated/config.hpp
#pragma once

namespace configflux::buildcfg {
inline constexpr const char* kSensorBusProtocol = "modbus_tcp";
}  // namespace configflux::buildcfg
```

Note the value: `modbus_tcp`, not the base value `modbus_rtu`. Selecting
`bus_type=ethernet` fired the conditional override you authored in section 6.
The same values arrive as CMake definitions for build-system consumers:

```console
$ cat snapshot/generated/config_build_flags.cmake
# Generated by ConfigFlux profile cpp_early_binding_v1
set(CFG_SENSOR_BUS_PROTOCOL "modbus_tcp")
add_compile_definitions(
  CFG_SENSOR_BUS_PROTOCOL_MODBUS_TCP=1
)
```

And `config_artifact_manifest.json` records the resolved artifact ids together
with the hash lineage, so a deployed configuration can always be traced back to
the model it came from. It is written without a trailing newline, so the `echo`
below is only there to put your shell prompt back on a line of its own:

```console
$ cat snapshot/generated/config_artifact_manifest.json; echo
{
  "artifacts": [],
  "model_hash": "797d2d277eaf4893f430a11b457e0ebccb977f25827304e27fc5f4c08e80f083",
  "profile": "cpp_early_binding_v1",
  "resolve_hash": "eab7a9058c7ee71ed3599dad6aa612ea40e11806884cf2c0aa23a9a7d7cdb3e2",
  "schema_version": 5
}
```

The list is empty because this pack declares no artifacts; a model that selects
driver binaries (section 5) lists them here.

### Step 4: Understand an impossible selection

`sensor_gateway`'s two facets vary independently, so every combination
resolves. Asking `cfx explain` about a workable selection says so and exits
with code 3 — there is nothing to explain:

```console
$ cfx explain --model build/cmp.manifest.json --select bus_type=ethernet --select environment=high_speed # exit 3
selection is satisfiable; nothing to explain
```

Models get interesting when they encode a **policy** — a rule spanning two
facets. The hero example ships one, declared as a constraint (section 4) in its
`00_definitions.json` chunk alongside the facets it talks about:

```json
"constraints": {
  "prod_forbids_debug": {
    "condition": "environment != 'prod' || log_level != 'debug'",
    "doc": "Debug logging is not permitted in production."
  }
}
```

The rule has an id of its own, so it stays out of the component graph and can be
named in a diagnostic. `cfx options` and `cfx explain` screen selections against
it: pick `environment=prod` and `log_level=debug` disappears from the offered
options, and asking about that pair produces the explanation below.

Compile that pack:

```bash
configflux-compiler compile \
  --source examples/00-service-multi-env/00_definitions.json \
  --source examples/00-service-multi-env/10_components.json \
  --out build-svc
```

Then ask for the forbidden combination:

```console
$ cfx explain --model build-svc/cmp.manifest.json --select environment=prod --select log_level=debug
cannot select log_level.debug:
  blocked by your earlier choice: environment.prod
  blocked by constraint prod_forbids_debug: environment != 'prod' || log_level != 'debug'
(one minimal explanation; other minimal cores may exist)
```

That is the **unsat core**: the minimal set of choices and rules that cannot
hold together, named in the vocabulary you authored. It does not dump the whole
constraint system — it names the earlier choice, then the constraint that rules
the combination out, by the id you gave it and with its condition quoted back.
Writing the rule down in the model, rather than leaving it in a runbook, is what
buys you this explanation for free.

`cfx explain` exits `0` when it printed a core, `3` when the selection was
satisfiable, and `2` on a usage or IO error. Diagnostics elsewhere in the
pipeline carry stable `E_` codes; `docs/faq.md` covers the ones evaluators hit
most often.

### You now hold a resolved configuration

That is the whole pipeline: CUE chunks → resolved JSON → CMP → snapshot. From
here, `docs/canonical-worked-example.md` replays `cfx options` and
`cfx resolve` against the larger water-pump reference model, and
`examples/00-service-multi-env/run.sh` shows one model resolved across three
environments with a determinism check on every snapshot.

## Related Documentation

- `examples/README.md` -- overview of the shipping example packs
- `examples/00-service-multi-env/` -- the hero example: one model, three
  environments, a compiled policy, and a determinism check
- `compiler/cue/README.md` -- the CUE authoring front-end: schema, export
  script, and verification harness
- `compiler/cue/schema.cue` -- the authoring schema (`#Config`, `#Profile`,
  `#snakeId`, `#ResolvePack`, and the rest of the constraints)
- `docs/canonical-worked-example.md` -- end-to-end operator flow using the
  water pump scenario, with the same `cfx` commands used in section 11
- `docs/faq.md` -- common evaluator questions, including the stable `E_`
  diagnostic codes the tools emit
- `docs/model-spec.md` -- formal model specification (schema, IR, hashes)
- `docs/design.md` -- architecture and pipeline rationale
- `docs/interface-contracts.md` -- application boundary contracts
