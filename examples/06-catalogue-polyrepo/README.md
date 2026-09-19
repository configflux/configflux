# 06 · One model, four repositories

A model does not have to live in one repository. This example composes one
model from chunks kept in **four separate repositories**: a shared catalogue
that describes the physical containers a site can be equipped with, and three
services that declare what they need from it.

```
repos/
  catalogue/            # owned by whoever maintains the container catalogue
    cue/00_catalogue.cue
    00_catalogue.json   # exported, inheritance-resolved
  vision/               # owned by the vision team
    cue/10_vision.cue
    10_vision.json
  compute/              # owned by the compute team
    cue/10_compute.cue
    10_compute.json
  sorter/               # owned by the sorter team
    cue/20_sorter.cue
    20_sorter.json
```

Each directory stands in for a repository. Nothing in the toolchain fetches or
vendors them — you check them out however you already check out code, and hand
the compiler each one's exported chunk:

```bash
configflux-compiler compile \
  --source repos/catalogue/00_catalogue.json \
  --source repos/vision/10_vision.json \
  --source repos/compute/10_compute.json \
  --source repos/sorter/20_sorter.json \
  --out out/cmp
```

That command is the shortcut for a checkout that holds all four at once. When
they are checked out and built separately, each directory is a **unit** and is
compiled on its own — see [Building one repository at a
time](#building-one-repository-at-a-time) below.

## The catalogue is typed data

`repos/catalogue/cue/00_catalogue.cue` writes the container table **once**, as
a catalogue: named entries, each supplying every declared field.

```cue
catalogues: {
    containers: {
        fields: {
            length_mm: {type: "integer", unit: "mm", doc: "Outside length."}
            width_mm: {type:  "integer", unit: "mm", doc: "Outside width."}
            height_mm: {type: "integer", unit: "mm", doc: "Outside height."}
        }
        entries: {
            c1: {length_mm: 1200, width_mm: 800, height_mm: 1000}
            c2: {length_mm: 800, width_mm:  600, height_mm: 700}
            c3: {length_mm: 600, width_mm:  400, height_mm: 400}
        }
    }
}
```

The compiler checks the table at ingest: every entry supplies exactly the
declared fields, with values of the declared types. Nothing has to be generated
from it by comprehension, because nothing beside it restates it.

A **binding** is one shared choice of an entry. A binding *is* a declared closed
facet whose values are the catalogue's entry ids, so an environment binds it
exactly as it binds `site`, `cfx options` lists it, and `cfx explain` names the
rules over it — nothing downstream special-cases it. Two bindings draw from this
one table, and the pair is the point:

```cue
bindings: {
    line_container: {
        catalogue: "containers"
        derive: site: {factory_a: "c1", factory_b: "c2"}
    }
    sorter_container: {
        catalogue: "containers"
    }
}
```

`line_container`'s `derive` table **decides**. What the example actually runs
for `factory_b` is the three free decisions `environments.json` records:

```bash
cfx resolve --model <model> --select site=factory_b --select sorter_container=c3 --select sorter_lanes=wide
```

That completes, and `line_container` is not one of them: the solver reads the
rule the table lowered to and binds it to `c2`, reporting it as `implied:`
rather than as a choice the operator typed. `sorter_container` has neither a
derive table nor a default, so nothing in the model decides it and an
environment that omits it is refused rather than quietly defaulted. With the
sorter repository in the model, `--select site=factory_b` alone exits `2` with
`E_RESOLVE_FACET_UNBOUND`, naming `sorter_container` as the unbound facet.
That is the split `environments.json` records: it states the **free**
decisions and nothing the model already made.

The one definition this repository declares, `container_dim_mm`, is what the
three services inherit the *shape* of their own lengths from. It is unrelated to
the catalogue table.

### The three levels a catalogue can be declared at

`catalogues` is an ordinary top-level namespace, so any unit may carry one, and
where you put a table is a question about ownership rather than about the tool:

| Level | Example here | When |
|---|---|---|
| A **shared unit** | `containers`, in `repos/catalogue` | Several units draw from the table and none of them owns it |
| The **service's own unit** | `lane_profile`, in `repos/sorter` | The table describes that one service's variants and nobody else reads it |
| The **integration unit** | (not needed in this example) | The table belongs to a particular deployment rather than to any service |

## Three services, in three other repositories

| Repository | Component | Requires | Inherits from the catalogue |
|---|---|---|---|
| `repos/vision` | `vision_service` | `container: line_container` | `roi_margin_mm` ← `container_dim_mm` |
| `repos/compute` | `compute_service` | `container: {binding: line_container, accepts: [c1, c2]}` | `grid_cell_mm` ← `container_dim_mm` |
| `repos/sorter` | `sorter_service` | `container: sorter_container`, `lanes: sorter_lanes` | `lane_width_margin_mm` ← `container_dim_mm` |

A **requirement** is a component's declared need for a binding, named by a
slot. It is the only way a component receives a catalogue entry, so a service
that forgets to declare its need has nothing to read and the mistake cannot be
silent. It is deliberately **not** a `depends_on` relationship: it names a
shared choice, not another component, so it joins no dependency closure and
constrains no build order.

**Sameness is by binding, not by catalogue.** `vision_service` and
`compute_service` name `line_container`, so they receive the same entry by
construction — that is how an author says "these must match". `sorter_service`
draws from the *same* `containers` table through a *different* binding, so it
handles a container of its own. Merging the two into one facet would throw that
away; keeping them apart and writing the agreement as a constraint is what
`sorter_matches_line` is for (see below).

`compute_service` adds an `accepts` list — the entries it can actually work
with. An entry outside every requirement's list is never offered by
`cfx options`, and a forced choice is refused by the requirement's name. With
`c3` in the table that list does real work: `c3` is a legal value of
`line_container` and it is this list alone that keeps the line off it, even
though `vision_service`, which declares no list, would have taken it.

`inherits` is resolved across the **whole pack** at export time, so a parameter
in the vision repository gap-fills its `type`, `unit`, `lifecycle` and `access`
from a definition declared in the catalogue repository. Fields the child
authors win — `roi_margin_mm` keeps its own `doc`.

The three service repositories never reference each other. Each references the
catalogue, and the catalogue references none of them.

### Requiring two bindings to agree

`repos/sorter/cue/20_sorter.cue` carries one more construct, **commented out**:

```cue
constraints: {
    sorter_matches_line: {
        condition: "sorter_container == line_container"
        doc:       "The sorter and the line handle the same container."
    }
}
```

The right-hand side is *unquoted*, which is what makes it another facet's name
rather than the literal value `line_container`. Both bindings stay
independently bindable and keep their own domains; only the constraint ties
them together, and `!=` requires the opposite.

It ships commented out because with it enabled the shipped environments stop
resolving — both bind `sorter_container` to `c3` while `line_container` derives
to `c1` or `c2`, and no container satisfies both rules at once — and an example
whose default state does not resolve teaches nothing. To try it, uncomment the
block and re-export the pack with the command under
[Regenerating the exported JSON](#regenerating-the-exported-json). `run.sh` step
4c exercises the rule itself, by machine and without going through CUE.

It selects what an environment actually states — the site it is, plus the one
container still free to choose:

```bash
cfx resolve --model <model> --select site=factory_a --select sorter_container=c3
```

`line_container` is never typed; the site's derive table decides it. `cfx
resolve` refuses the selection with exit `3` and writes nothing. Because the
contradiction is reached *through* the derived binding, `cfx explain`'s core
names more than the nearest rule: `sorter_matches_line` ties the sorter to the
line, and the site's derive table (`derive:line_container:site=factory_a`) is
what turns the site into a constraint on `line_container` at all. Those two are
the rule and the link, and step 4c asserts both. The core also names compute's
accepts list (`accepts:compute_service.container`), a second route to the same
clash, which the step leaves unasserted because `cfx explain` returns one
minimal core of possibly several. Hand-binding `line_container` would reach the
same refusal, but it is not how an environment is written — an environment
states only its free decisions.

## A service reads its own configuration

This is what the requirement buys. Resolve delivers the catalogue entry the
binding took **inside the requiring component**:

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
    "params": { "...": "..." }
  }
}
```

The vision service reads `requires.container.fields.width_mm`. It never names
the catalogue, never names the binding's other consumers, and does not change
when the plant reorganises which unit owns the table. Scoping the resolve to
one service changes nothing about that: the catalogue is not in the service's
dependency closure, and the entry still arrives.

## Building one repository at a time

The `compile` at the top of this file needs every chunk at once. Four teams
building four repositories on four machines do not have that checkout, and this
is the form for them.

A **unit** is a set of chunks that share one `package` value. It is a folder in
a monorepo or a whole repository — the tool does not care which, because the
`package` field the chunks already carry is the whole definition. Here there
are four: `site_catalogue`, `vision_service`, `compute_service` and
`sorter_service`.

`compile-object` compiles one unit, and nothing else, into an **object**:

```bash
configflux-compiler compile-object \
  --source repos/catalogue/00_catalogue.json \
  --out out/site_catalogue.cfo

configflux-compiler compile-object \
  --source repos/vision/10_vision.json \
  --interface out/site_catalogue.cfo \
  --out out/vision_service.cfo
```

An object directory holds the unit's chunk files, a provenance sidecar, and
`object.json` — the **header**, which is the whole cross-unit contract: what the
unit exports, what it still needs from elsewhere, the clauses it contributes to
the constraint model, and the `object_hash` of every interface it was compiled
against. There is no constraint model and no package index in an object; those
are products of the link. `--interface` reads only the other object's header,
never its chunk files, so the catalogue's parameters are not re-read once per
service.

The catalogue is compiled first and against nothing, because it declares what
the others read. A service that names something no interface supplies is **not**
refused: the reference is recorded in the header's `imports` and settled at link
time. That is what lets each unit build alone.

`link` assembles objects into a package:

```bash
configflux-compiler link \
  --object out/site_catalogue.cfo \
  --object out/vision_service.cfo \
  --object out/compute_service.cfo \
  --object out/sorter_service.cfo \
  --out out/cmp --write-lock configflux.lock
```

It reads the four headers before it opens a single chunk file, and every
cross-unit question is answered there: two objects claiming one unit
(`E_LINK_DUPLICATE_UNIT`), two units exporting one id (`E_LINK_DUPLICATE_ID`),
an import nothing declares (`E_LINK_UNRESOLVED_IMPORT`), a unit compiled against
a different version of an interface than the one being linked
(`E_LINK_INTERFACE_MISMATCH`), and a chunk file whose body no longer hashes to
its own name (`E_LINK_OBJECT_CORRUPT`). Nothing is written under `--out` unless
every check passes.

**The package is the same package.** `compile` groups its `--source` chunks by
`package`, builds one header per unit in memory, and runs those same stages — it
*is* the linker. So the two forms are one code path and produce byte-identical
output, which `run.sh` asserts file by file rather than argues.

### The lock boundary

`--write-lock` records, per unit, the `object_hash` this link used:

```json
{
  "schema_version": 1,
  "objects": {
    "compute_service": { "object_hash": "0dda...", "source": "" },
    "site_catalogue":  { "object_hash": "24d9...", "source": "" },
    "sorter_service":  { "object_hash": "11f7...", "source": "" },
    "vision_service":  { "object_hash": "982c...", "source": "" }
  }
}
```

A later `link --lock configflux.lock` must reproduce exactly that set, or it is
refused: `E_LINK_LOCK_MISMATCH` names the unit and both hashes,
`E_LINK_LOCK_UNLINKED` names a pinned unit that was not linked. That refusal is
the review gate — someone renews the pin deliberately, from the integration
unit, once the change has been looked at.

The pins are **checked, never fetched**. Nothing here downloads an object, and
`source` is a free-text note about where a unit came from that no code path
reads. Bringing the objects to the machine stays the job of your checkout, your
submodule, your artifact store or your CI. A pin format that fetches is a
package manager, and adopting one would put a supply chain inside a
configuration compiler.

## Run it

```bash
bazel build //compiler //cfx      # once
cd examples/06-catalogue-polyrepo
./run.sh
```

`run.sh` self-checks seven things:

1. **Four directories, one model.** The four chunks compile together. Two
   properties ride along in this step. *Identity is content-canonical*: one
   repository is copied to a different path — another checkout, a CI job's
   scratch directory, a vendored copy — and recompiled, and the `model_hash` is
   *identical*, which is what makes hashes comparable across machines. And *the
   compile set is the unit of verification*: `catalogue + vision` compiles (a
   dependency-closed subset is a legitimate, differently-hashed model) while
   `vision` on its own does not.
2. **Per-site resolution, read from each service's own block.** `cfx resolve
   --manifest environments.json --all --scopes
   component:vision_service,component:compute_service,component:sorter_service`
   writes the 2 × 3 matrix and prints every `requires` block. Vision and compute
   get the *same* entry — `c1` at 1200 × 800 × 1000 mm for `factory_a`, `c2` at
   800 × 600 × 700 mm for `factory_b` — while the sorter gets `c3` at
   600 × 400 × 400 mm plus its own lane profile.
3. **Never offered.** `cfx options --select site=factory_b` lists `c2` alone for
   `line_container`.
4. **Three forced conflicts, each refused by name.** `cfx resolve` exits `3` and
   writes nothing; `cfx explain` then names the rule that did it — the derive
   table (`derive:line_container:site=factory_a`) and compute's accepts list
   (`accepts:compute_service.container`). The third forces the equality
   constraint, and it does so the way an environment is written — `site` plus
   the free `sorter_container`, leaving `line_container` to the derive table —
   so the contradiction is reached *through* a derived binding and the core
   names the derive link as well as `sorter_matches_line` itself, rather than
   the nearest rule alone. Note the exit codes run opposite ways: `cfx
   explain` exits `0` when it *has* an explanation and `3` when the selection
   was fine.
5. **One facet, bound twice.** `--select sorter_container=c3 --select
   sorter_container=c1` exits `2` naming the facet and both options, rather than
   silently keeping the last one.
6. **Determinism.** The same selection resolved twice produces byte-identical
   snapshots, and reports `implied: line_container=c2` — a container nobody
   typed.
7. **One unit at a time.** Each repository is compiled into its own object —
   the catalogue against nothing, the three services against the catalogue's
   header — and the four objects are linked with `--write-lock`. The linked
   package is compared against step 1's file by file and is identical, all
   fourteen files, and the lock pins the four units by their `package` names.
   Then two faults only a link can name: the three services linked *without*
   the catalogue (`E_LINK_UNRESOLVED_IMPORT`), and a service carrying the hash
   of a catalogue object that has since been edited
   (`E_LINK_INTERFACE_MISMATCH`). Both exit `2` and write nothing.

Step 4's third case needs a note. The equality constraint ships commented out
in the CUE, and the example test has no `cue` evaluator in its runfiles, so
`run.sh` cannot take the human path of uncommenting and re-exporting. It does
the machine-checkable **equivalent** instead: it injects the same
`constraints.sorter_matches_line` object into a *copy* of the already-exported
`20_sorter.json` and compiles that. The CUE is not re-exported and the
committed JSON on disk is left untouched.

## Regenerating the exported JSON

The committed `*.json` next to each `cue/` directory is produced by the
reference exporter, which resolves inheritance across any number of chunks
against one shared definitions chunk:

```bash
examples/export_pack.sh \
  --schema compiler/cue/schema.cue \
  --definitions examples/06-catalogue-polyrepo/repos/catalogue/cue/00_catalogue.cue \
  --components  examples/06-catalogue-polyrepo/repos/vision/cue/10_vision.cue \
  --components  examples/06-catalogue-polyrepo/repos/compute/cue/10_compute.cue \
  --components  examples/06-catalogue-polyrepo/repos/sorter/cue/20_sorter.cue \
  --out <dir>
```

Two drift guards exist in this repository and they cover different layouts —
do not confuse them. `compiler/cue/export_fixtures.sh --check` covers the
two-file and single-file packs under `examples/*/cue`. This example's chunks
are nested one level deeper, one per repository, so its guard is
`//examples:export_pack_test`, which requires a fresh `export_pack.sh` run to
be **byte-identical** to the committed files.

## One honest limit

**A repository cannot be verified on its own.** `vision` requires
`line_container`, a binding declared in the catalogue repository, and compiling
`vision` alone fails with `E_REQUIRES_INVALID` naming the component, the slot
and the binding. This is the tool refusing to certify a fragment whose shared
choice it has never seen: a repository is verified together with the exported
chunks that declare what it requires. A dependency-closed subset is fine — it is
simply a different, differently-hashed model.

## What to notice

- **Composition is the default, not a mode.** `--source` is repeatable and the
  merged root is synthetic, so the four chunks may carry four different
  `package` values: that value names the unit each chunk belongs to, and the
  compile groups by it. Nothing cross-checks a `version`. What *is* enforced:
  ids must be unique across every chunk, every `depends_on` target must be in
  the compile set, and every requirement must resolve to a declared binding.
- **One table, no restatement.** A binding's value domain *is* the catalogue's
  entry ids and the entries carry their own values, so there is one place to
  edit and nothing to keep in sync by hand.
- **Declared need, delivered value.** A service says what it needs and reads
  what it was given. Neither half mentions where the table lives.
- **Sameness is a binding, not a table.** Two components share an entry because
  they name one binding — not because they read one catalogue. Two bindings over
  the same table are two independent decisions, and tying them together is a
  constraint you write on purpose.
- **Two ways to build one model, one code path.** Handing every chunk to
  `compile` is the shortcut for a checkout that holds them all. Compiling each
  repository into an object and linking the objects is what four separate
  checkouts can actually do, and it produces the same bytes — so choosing
  between them is a question about your build, never about your model. Only the
  second form can name a missing or stale unit, because only it is ever handed
  an incomplete set.
- **Ownership follows the repository.** The catalogue team owns the containers,
  the two bindings and the site rule; each service team owns its own parameters,
  its own requirements, and — where the table is nobody else's business, as the
  sorter's lane profiles are — its own catalogue.

## Files

- [`repos/catalogue/cue/00_catalogue.cue`](repos/catalogue/cue/00_catalogue.cue)
  — the `containers` catalogue, the `line_container` and `sorter_container`
  bindings (one derived from the site, one free), the `site` facet, and the
  `container_dim_mm` definition.
- [`repos/vision/cue/10_vision.cue`](repos/vision/cue/10_vision.cue),
  [`repos/compute/cue/10_compute.cue`](repos/compute/cue/10_compute.cue) — two
  service chunks over the same binding, one of which narrows it with `accepts`.
- [`repos/sorter/cue/20_sorter.cue`](repos/sorter/cue/20_sorter.cue) — a third
  service, with a catalogue and a binding of its own, two `requires` slots, and
  the commented-out `sorter_matches_line` equality constraint.
- `repos/*/*.json` — the exported, inheritance-resolved chunks the compiler
  ingests.
- [`environments.json`](environments.json) — the two named sites and their free
  decisions, and nothing else.
- [`run.sh`](run.sh) — the seven-step walkthrough above.
