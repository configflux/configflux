// s4_mobile_robot/smoke -- 00_definitions chunk for the CUE authoring front-end
// (ADR 0021, phase 7; configflux-ujh6). Bootstrapped from ../chunks/00_definitions.toml,
// validated against compiler/cue/schema.cue, and exported to 00_definitions.json by the
// pinned cue binary (compiler/cue/export_fixtures.sh). The differential gate
// (scenario_cue_equivalence_tests.rs) proves the CUE-derived CMP is byte-identical
// to the TOML-derived CMP. `condition`/`overrides` are opaque pass-through data
// (ADR 0021 decision 1): CUE types and emits them verbatim.
package configflux

chunk: #Config & {
  "package": "s4_mobile_robot",
  "version": "1.0.0",
  "definitions": {
    "drive_profile": {
      "type": "string",
      "lifecycle": "construction",
      "access": "developer",
      "doc": "Drive control compile profile"
    },
    "drive_driver_slot": {
      "type": "artifact",
      "lifecycle": "construction",
      "access": "developer",
      "doc": "Drive driver artifact"
    },
    "localization_driver_slot": {
      "type": "artifact",
      "lifecycle": "construction",
      "access": "developer",
      "doc": "Localization stack artifact"
    },
    "payload_driver_slot": {
      "type": "artifact",
      "lifecycle": "construction",
      "access": "developer",
      "doc": "Payload module artifact"
    },
    "battery_profile": {
      "type": "string",
      "lifecycle": "construction",
      "access": "developer",
      "doc": "Battery profile for compile-time feature gates"
    },
    "max_speed_commissioning": {
      "type": "float",
      "unit": "mps",
      "lifecycle": "startup",
      "access": "integrator",
      "doc": "Commissioning speed limit"
    },
    "runtime_trim": {
      "type": "float",
      "unit": "ratio",
      "lifecycle": "runtime",
      "access": "technician",
      "doc": "Runtime trim gain"
    }
  }
}
