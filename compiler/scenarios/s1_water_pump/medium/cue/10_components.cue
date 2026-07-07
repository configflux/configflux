// s1_water_pump/medium -- 10_components chunk for the CUE authoring front-end
// (ADR 0021, phase 7; configflux-ujh6). Bootstrapped from ../chunks/10_components.toml,
// validated against compiler/cue/schema.cue, and exported to 10_components.json by the
// pinned cue binary (compiler/cue/export_fixtures.sh). The differential gate
// (scenario_cue_equivalence_tests.rs) proves the CUE-derived CMP is byte-identical
// to the TOML-derived CMP. `condition`/`overrides` are opaque pass-through data
// (ADR 0021 decision 1): CUE types and emits them verbatim.
package configflux

chunk: #Config & {
  "package": "s1_water_pump",
  "version": "1.0.0",
  "artifacts": {
    "hydra_x200_single_driver": {
      "name": "hydra_x200_single_driver",
      "version": "1.0.0",
      "hash": "sha256-hydra-x200-single",
      "source": "artifact://drivers/hydra_x200_single.so",
      "target": "/opt/configflux/drivers/hydra_x200_single.so",
      "doc": "Hydra X200 single-pump driver"
    },
    "hydra_x200_dual_driver": {
      "name": "hydra_x200_dual_driver",
      "version": "1.0.0",
      "hash": "sha256-hydra-x200-dual",
      "source": "artifact://drivers/hydra_x200_dual.so",
      "target": "/opt/configflux/drivers/hydra_x200_dual.so",
      "doc": "Hydra X200 dual-pump driver"
    },
    "aeroflux_a9_driver": {
      "name": "aeroflux_a9_driver",
      "version": "2.3.1",
      "hash": "sha256-aeroflux-a9",
      "source": "artifact://drivers/aeroflux_a9.so",
      "target": "/opt/configflux/drivers/aeroflux_a9.so",
      "doc": "Aeroflux A9 driver"
    }
  },
  "components": {
    "power_bus": {
      "type": "power_module"
    },
    "thermal_control": {
      "type": "controller",
      "depends_on": [
        "power_bus"
      ],
      "params": {
        "max_flow_at_commissioning": {
          "inherits": "safe_flow",
          "value": 42.5,
          "req_id": "req_s1_001"
        },
        "control_driver": {
          "inherits": "driver_slot",
          "value": "hydra_x200_single_driver",
          "req_id": "req_s1_002",
          "overrides": [
            {
              "condition": "cooling_brand == 'hydra' && cooling_model == 'x200' && pump_type == 'dual'",
              "value": "hydra_x200_dual_driver"
            },
            {
              "condition": "cooling_brand == 'aeroflux' && cooling_model == 'a9'",
              "value": "aeroflux_a9_driver"
            }
          ]
        },
        "runtime_trim_gain": {
          "inherits": "trim_gain",
          "value": 0.15,
          "req_id": "req_s1_003"
        }
      }
    },
    "eu_label": {
      "type": "label_module",
      "condition": "region == 'eu'"
    }
  }
}
