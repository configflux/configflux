# Starting from Scratch: Modeling Your Domain with ConfigFlux

This guide walks you through authoring a ConfigFlux configuration model for
a new product domain. It assumes you have built and run the existing examples
(see `docs/canonical-worked-example.md` and the packs under `examples/`) and
now want to model your own equipment, product family, or system.

Models are authored in **CUE**. You write typed `.cue` chunks, export them to
inheritance-resolved JSON, and feed that JSON to the compiler. CUE is the sole
authoring and ingestion format; this guide teaches that flow end to end.

## Prerequisites

- The ConfigFlux compiler binary, built from this repo:

  ```bash
  bazel build //compiler
  ```

  This produces `bazel-bin/compiler/compiler`. The tool identifies itself as
  `configflux-compiler`; in commands you invoke it as `bazel-bin/compiler/compiler`.

- The pinned `cue` binary, **version 0.16.1**. The export step requires this
  exact version; other versions are not supported. Set the `$CUE` environment
  variable to point at it.
- Familiarity with [CUE](https://cuelang.org/) syntax: structs, fields, the
  `&` unification operator, and definitions (`#Name`).
- A clear picture of the "things" your product configures and the choices a
  customer or integrator makes when ordering or commissioning it.

## The Big Picture: Author, Export, Compile

ConfigFlux authoring has three stages. Keep this pipeline in mind as you read
the rest of the guide:

```
  .cue chunks            resolved .json            CMP (compiled model package)
 (you author)   ---->   (export step emits)  ---->   (compiler ingests)
  package configflux      inheritance filled in       cmp.manifest.json, *.cfir
  chunk: #Config & {...}  by #ResolvePack             index.cfir.json, ccm/
```

1. **Author** your model as CUE chunks validated against `compiler/cue/schema.cue`.
2. **Export** the chunks to JSON. The export step runs your chunks through the
   schema's inheritance engine (`#ResolvePack`), so the emitted JSON has every
   inherited field filled in. This is the JSON the compiler ingests.
3. **Compile** the resolved JSON into a Compiled Model Package (CMP) with
   `configflux-compiler compile`.

The fastest way to see all three stages run is to execute a shipping example:

```bash
cd examples/01-hello-led && ./run.sh
```

That script locates the compiler binary and runs compile -> verify -> inspect
against the example's already-exported JSON. Read the rest of this guide to
learn how to build your own.

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
graph is acyclic. The project's dependency model is also designed to reject
diamond shapes (a component reachable via two paths from the same root); keep
your dependencies a simple tree where you can:

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

### Examples of facet design

| Domain | Facets | Values |
|---|---|---|
| Water pump system | `cooling_brand`, `cooling_model`, `pump_type`, `region` | brand names, model numbers, single/dual, us/eu |
| Automation cell | `conveyor_brand`, `vision_stack`, `safety_mode`, `network_topology` | vendor names, pl_d/pl_e, ring/star |
| Building HVAC | `occupancy_class`, `filtration_grade`, `region` | office/hospital, merv13/hepa, us/eu |

### The default context

Every profile must provide a `default_context` that assigns a value to each
facet. This is the starting point for selection and must produce a valid
resolved model. The compiler and interpreter use it as the baseline.

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

## 8) Pack Layout

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
the definitions, and exposes a resolved `definitions` and `components`. The
recipe below wraps your own pack into `#ResolvePack` and emits the resolved,
per-file JSON. It runs entirely against your own directory -- nothing here is
tied to this repository's layout.

> The in-repo helper `compiler/cue/export_fixtures.sh` is **not** the command to
> export your own pack. It is a drift guard/regenerator for the example and
> scenario fixtures that ship *inside this repository*: its pack roots are
> hard-coded to `compiler/scenarios` and `examples`, so it never discovers a
> `my_domain/` you create elsewhere. Use it only to check or regenerate the
> in-repo fixtures (see the end of this section). For your own pack, use the
> self-contained recipe here.

#### Two-file pack: export `00_definitions.json` and `10_components.json`

The export has two steps, mirroring exactly what the in-repo helper does
internally: first emit each chunk *raw*, then resolve both raw chunks together
through `#ResolvePack` and emit the per-file slices. Two steps are needed
because both chunk files bind a top-level `chunk`; exporting each one raw first
lets the driver place them under distinct fields in a single resolving
evaluation.

Step 1a -- emit each chunk raw (these are the un-gap-filled chunks; you do
**not** feed these to the compiler):

```bash
"$CUE" export -e chunk my_domain/cue/00_definitions.cue compiler/cue/schema.cue \
  --out json > my_domain/defs_raw.json
"$CUE" export -e chunk my_domain/cue/10_components.cue compiler/cue/schema.cue \
  --out json > my_domain/comps_raw.json
```

Step 1b -- author a small driver in your pack directory that wraps the two raw
chunks and applies `#ResolvePack`. The field names below
(`defsIn`, `compsIn`, `definitionsOut`, `componentsOut`) match the in-repo
helper's driver, and the slices keep attribution per-file: `00_definitions.json`
carries only the definitions, `10_components.json` carries only the resolved
components (and any artifacts authored in that chunk). Save this as
`my_domain/_export.cue`:

```cue
package configflux

// defsIn/compsIn are supplied on the export command line as data files
// wrapping the raw chunks emitted in step 1a.
defsIn: {...}
compsIn: {...}

_resolved: #ResolvePack & {
    _definitions: defsIn.definitions
    _components:  compsIn.components
}

// Definitions-file slice: the inheritance roots, passed through unchanged.
definitionsOut: {
    package: defsIn.package
    version: defsIn.version
    if defsIn.definitions != _|_ {definitions: defsIn.definitions}
    if defsIn.components != _|_ {components: defsIn.components}
    if defsIn.artifacts != _|_ {artifacts: defsIn.artifacts}
}

// Components-file slice: resolved components (inheritance gap-filled) plus this
// chunk's own artifacts. No definitions leak in -- attribution stays per-file.
componentsOut: {
    package: compsIn.package
    version: compsIn.version
    if compsIn.artifacts != _|_ {artifacts: compsIn.artifacts}
    if compsIn.components != _|_ {components: _resolved.components}
}
```

Step 1c -- wrap the raw chunks as data files and emit the two resolved slices.
The two small wrapper files inject the raw JSON under the `defsIn` / `compsIn`
fields the driver expects:

```bash
{ echo 'package configflux'; printf 'defsIn: '; cat my_domain/defs_raw.json; } \
  > my_domain/_defs_wrap.cue
{ echo 'package configflux'; printf 'compsIn: '; cat my_domain/comps_raw.json; } \
  > my_domain/_comps_wrap.cue

"$CUE" export my_domain/_export.cue my_domain/_defs_wrap.cue \
  my_domain/_comps_wrap.cue compiler/cue/schema.cue \
  -e definitionsOut --out json > my_domain/00_definitions.json
"$CUE" export my_domain/_export.cue my_domain/_defs_wrap.cue \
  my_domain/_comps_wrap.cue compiler/cue/schema.cue \
  -e componentsOut --out json > my_domain/10_components.json
```

`my_domain/00_definitions.json` and `my_domain/10_components.json` are now the
inheritance-resolved JSON the compiler ingests. (The `defs_raw.json`,
`comps_raw.json`, and `_*_wrap.cue` files are scratch; you can delete them or
keep them out of your committed tree.)

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

#### Single-file pack: export one `config.json`

If you authored a single-file pack (one chunk holding both definitions and
components, like `examples/01-hello-led/cue/config.cue`), the resolved output is
one combined `config.json`. The driver uses the helper's single-file field name,
`configOut`. Save this as `my_domain/_export.cue`:

```cue
package configflux

// srcIn is supplied on the export command line as a data file wrapping the raw
// single-file chunk.
srcIn: {...}

_resolved: #ResolvePack & {
    _definitions: srcIn.definitions
    _components:  srcIn.components
}

configOut: {
    package: srcIn.package
    version: srcIn.version
    if srcIn.definitions != _|_ {definitions: srcIn.definitions}
    if srcIn.artifacts != _|_ {artifacts: srcIn.artifacts}
    if srcIn.components != _|_ {components: _resolved.components}
}
```

Then emit the raw chunk, wrap it, and export the resolved config:

```bash
"$CUE" export -e chunk my_domain/cue/config.cue compiler/cue/schema.cue \
  --out json > my_domain/src_raw.json
{ echo 'package configflux'; printf 'srcIn: '; cat my_domain/src_raw.json; } \
  > my_domain/_src_wrap.cue

"$CUE" export my_domain/_export.cue my_domain/_src_wrap.cue \
  compiler/cue/schema.cue \
  -e configOut --out json > my_domain/config.json
```

`my_domain/config.json` is the resolved JSON the compiler ingests.

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
bazel-bin/compiler/compiler compile \
  --source 00_definitions.json \
  --source 10_components.json \
  --out out/cmp
```

`compile` prints a JSON result to stdout. A successful run looks like:

```json
{ "schema_version": 1, "status": "ok",
  "model_hash": "eacc8c3a...5169b4",
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

### Step 3: Verify and inspect (optional)

`verify` runs every validation check without emitting a CMP. Use it during
authoring to catch problems early:

```bash
bazel-bin/compiler/compiler verify \
  --source 00_definitions.json \
  --source 10_components.json
```

Validation covers: JSON parse validity, snake_case id enforcement, definition
inheritance integrity (targets exist, no cycles), component dependency
validation (targets exist, no cycles, no diamonds), and condition compatibility
(dependent components have compatible conditions).

`inspect ... summary` prints an overview of the merged model:

```bash
bazel-bin/compiler/compiler inspect \
  --source 00_definitions.json \
  --source 10_components.json \
  summary
```

Use `inspect` to confirm the merged model looks correct before moving on to
selection and resolution.

## 10) From an Empty Directory to a Compiled CMP

Here is the full sequence for modeling a new domain from scratch. Every command
below is one you run directly; substitute your own pack directory and file
names.

```bash
# 0. One-time: build the compiler.
bazel build //compiler

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
bazel-bin/compiler/compiler compile \
  --source 00_definitions.json \
  --source 10_components.json \
  --out out/cmp

# 4. (Optional) Verify and inspect.
bazel-bin/compiler/compiler verify \
  --source 00_definitions.json \
  --source 10_components.json
bazel-bin/compiler/compiler inspect \
  --source 00_definitions.json \
  --source 10_components.json \
  summary
```

The output of step 3 is your Compiled Model Package under `out/cmp/`. From
there you can move on to selection and resolution against the profile you
designed in section 4.

### The fastest start: run a shipping example

If you would rather see the pipeline work before authoring anything, run one of
the example packs end to end:

```bash
cd examples/01-hello-led && ./run.sh
```

Each example's `run.sh` locates the compiler binary and runs
compile -> verify -> inspect against the example's already-exported JSON. The
shipping examples, in increasing complexity, are:

- `examples/01-hello-led/` -- bare minimum: one definition, one component,
  single-file CUE chunk.
- `examples/02-sensor-gateway/` -- two-file pack: inheritance, conditional
  overrides, and a conditional component.
- `examples/03-motor-controller/` and `examples/04-fleet-edge-node/` -- larger
  models.

See `examples/README.md` for an overview of the example set.

## Related Documentation

- `examples/README.md` -- overview of the shipping example packs
- `compiler/cue/README.md` -- the CUE authoring front-end: schema, export
  script, and verification harness
- `compiler/cue/schema.cue` -- the authoring schema (`#Config`, `#Profile`,
  `#snakeId`, `#ResolvePack`, and the rest of the constraints)
- `docs/canonical-worked-example.md` -- end-to-end operator flow using the
  water pump scenario
- `docs/model-spec.md` -- formal model specification (schema, IR, hashes)
- `docs/design.md` -- architecture and pipeline rationale
- `docs/interface-contracts.md` -- application boundary contracts
