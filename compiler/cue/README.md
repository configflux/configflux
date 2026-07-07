# CUE authoring front-end (L1)

CUE is the **canonical authoring surface** for ConfigFlux scenario models — the
typed front-end for authoring scenarios. Author models as
`.cue`, export them to JSON with the pinned `cue` binary, and feed the JSON to
the compiler. TOML ingestion remains supported as a **regression backstop**
(`Compiler::add_chunk_with_source`) but is no longer the default authoring
format; the standalone `parse_config` helper was retired in phase 8.

The compiler's data model (`compiler/src/schema.rs`) is still the single source
of truth; `condition`, `overrides`, cardinality, and late-binding remain
Rust/solver-owned. At L1, CUE is a typed authoring + validation layer that emits
the model verbatim — it is deliberately **out of the hash path** (entering it,
along with CUE owning `inherits`, is the deferred L2 epic).

## Authoring workflow

1. Write a chunk as `<scenario>/<size>/cue/NN_name.cue`:

   ```cue
   package configflux

   chunk: #Config & {
       package: "my_scenario"
       version: "1.0.0"
       definitions: { /* ... */ }
       components: { /* ... */ }
   }
   ```

2. Export to JSON (and drift-check committed fixtures). The scripts resolve the
   hermetic, Bazel-pinned `cue` by default (no $PATH
   fallback), so just run them:

   ```sh
   compiler/cue/export_fixtures.sh          # regenerate (pinned cue)
   compiler/cue/export_fixtures.sh --check  # drift gate (pinned cue)
   ```

   Set `$CUE` only to opt into a specific binary (e.g. CI uses its own
   sha256-verified download):

   ```sh
   CUE=/path/to/cue compiler/cue/export_fixtures.sh --check
   ```

3. The compiler ingests the JSON through the product path; `add_chunk_auto`
   content-sniffs TOML vs JSON, so the same manifest accepts either.
   `compiler/src/scenario_byte_stability_tests.rs` then pins the resulting CMP
   output for byte-stability across all S1–S5 packs.

## Files

- **`schema.cue`** — `#Config` (the 150% model) and `#Profile` (resolution
  selection domains + default context), mirroring `compiler/src/schema.rs` and
  the compiler's `ScenarioProfile`. Validates authored input structurally: types,
  enums, closed structs (typo protection), required fields, and the recursive
  `overrides` / `#ConditionalBlock` shape.
- **`validate_fixtures.sh`** — schema-vs-corpus verification harness.
- **`export_fixtures.sh`** — exports authored `*.cue` to their committed `*.json`
  siblings (or `--check` verifies they match a fresh export — wired into CI).
- **`testdata/full_surface.json`** — a synthetic, valid `#Config` exercising the
  parts of the schema the real corpus does not (bool values, all four `limits`
  sub-fields, `sil1`/`sil3`/`sil4`, `supervisor`/`super_user`, nested overrides
  at depth ≥2).
- **Worked examples** — every scenario pack carries its authored CUE under
  `compiler/scenarios/<scenario>/<size>/cue/` (S2/smoke is hand-authored and
  idiomatic; the rest were bootstrapped from the legacy TOML).

## Verification

- `validate_fixtures.sh` — positive `#Config` (every scenario chunk, every
  `examples/` config, `full_surface.json`), positive `#Profile` (every
  `profile.toml`), and a battery of negatives (bad enums, wrong value types,
  bad/unknown `limits`, missing required fields, typos). The positive union
  exercises **100% of the schema surface**.
- `scenario_byte_stability_tests.rs` — the byte-stability gate: every S1–S5 pack is driven
  from its CUE-exported JSON through the full compile → resolve → export_software_bom loop, and
  `model_hash`, `selection_state_hash`, `resolve_hash`, and `bom_hash` are pinned byte-for-byte
  against a checked-in baseline fixture.
- CI pins **cue v0.16.1** (sha256-verified), runs `validate_fixtures.sh`, and
  runs `export_fixtures.sh --check` so the committed JSON cannot drift from its
  CUE source.
- Hermetic toolchain: the same **cue v0.16.1** is fetched hermetically through
  Bazel (`MODULE.bazel` → `tools/cue_toolchain.bzl`, per-platform sha256).
  `validate_fixtures.sh` / `export_fixtures.sh` consume it by default (no $PATH
  fallback), and the pinned version is recorded in the SBOM
  (`docs/sbom/configflux-0.1.0.cdx.json`).

## Scope (L1 complete; L2 deferred)

L1 (phases 3–8, done): hand-written schema, fixture validation, pinned-cue CI,
JSON ingestion, model-derived (format-agnostic) chunk hash, the byte-identical
differential gate, full S1–S5 conversion to CUE, and CUE established as the
default authoring surface with TOML kept as a backstop.

Deferred to **L2** (separate, ADR-gated epic): CUE owning `inherits` /
cross-file unification, field→chunk provenance, CUE entering the hash path, the
generated-schema pipeline (`schemars` → JSON Schema → `cue import`), the
`cfx-cue` Go wrapper + hermetic `rules_go` toolchain, and the corresponding
deletion of the TOML scenario corpus + compiler-side validations.

## Notes

- Pinned reference: **cue v0.16.1**.
- `#SafetyLevel` uses `"q_m"` — serde's `snake_case` rendering of the `QM`
  variant (not `"qm"`). Verified against the real corpus.
- `#ConditionalBlock` is `#Parameter` + a required `condition`. Because a closed
  struct rejects extra fields (`close(P) & {condition}` fails), the shared
  fields live in an open template `_paramFields` that each definition closes.
