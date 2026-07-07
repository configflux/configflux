// s5_building_hvac/smoke -- 00_definitions chunk for the CUE authoring front-end
// (ADR 0021, phase 7; configflux-ujh6). Bootstrapped from ../chunks/00_definitions.toml,
// validated against compiler/cue/schema.cue, and exported to 00_definitions.json by the
// pinned cue binary (compiler/cue/export_fixtures.sh). The differential gate
// (scenario_cue_equivalence_tests.rs) proves the CUE-derived CMP is byte-identical
// to the TOML-derived CMP. `condition`/`overrides` are opaque pass-through data
// (ADR 0021 decision 1): CUE types and emits them verbatim.
package configflux

chunk: #Config & {
  "package": "s5_building_hvac",
  "version": "1.0.0",
  "definitions": {
    "ventilation_profile": {
      "type": "string",
      "lifecycle": "startup",
      "access": "integrator",
      "doc": "Commissioning profile selected for the building HVAC controller"
    },
    "controller_package": {
      "type": "artifact",
      "lifecycle": "construction",
      "access": "developer",
      "doc": "Selected HVAC controller package artifact"
    },
    "airflow_trim_gain": {
      "type": "float",
      "unit": "ratio",
      "lifecycle": "runtime",
      "access": "technician",
      "doc": "Runtime airflow trim gain for balancing after commissioning"
    }
  }
}
