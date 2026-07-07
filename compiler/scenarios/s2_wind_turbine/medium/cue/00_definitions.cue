// s2_wind_turbine/medium -- 00_definitions chunk for the CUE authoring front-end
// (ADR 0021, phase 7; configflux-ujh6). Bootstrapped from ../chunks/00_definitions.toml,
// validated against compiler/cue/schema.cue, and exported to 00_definitions.json by the
// pinned cue binary (compiler/cue/export_fixtures.sh). The differential gate
// (scenario_cue_equivalence_tests.rs) proves the CUE-derived CMP is byte-identical
// to the TOML-derived CMP. `condition`/`overrides` are opaque pass-through data
// (ADR 0021 decision 1): CUE types and emits them verbatim.
package configflux

chunk: #Config & {
  "package": "s2_wind_turbine",
  "version": "1.0.0",
  "definitions": {
    "grid_profile": {
      "type": "string",
      "lifecycle": "startup",
      "access": "integrator",
      "doc": "Grid code commissioning profile"
    },
    "control_driver_slot": {
      "type": "artifact",
      "lifecycle": "construction",
      "access": "developer",
      "doc": "Selected control driver artifact"
    },
    "pitch_trim_gain": {
      "type": "float",
      "unit": "ratio",
      "lifecycle": "runtime",
      "access": "technician",
      "doc": "Runtime pitch trim gain"
    }
  }
}
