// s2_wind_turbine/medium -- 10_components chunk for the CUE authoring front-end
// (ADR 0021, phase 7; configflux-ujh6). Bootstrapped from ../chunks/10_components.toml,
// validated against compiler/cue/schema.cue, and exported to 10_components.json by the
// pinned cue binary (compiler/cue/export_fixtures.sh). The differential gate
// (scenario_cue_equivalence_tests.rs) proves the CUE-derived CMP is byte-identical
// to the TOML-derived CMP. `condition`/`overrides` are opaque pass-through data
// (ADR 0021 decision 1): CUE types and emits them verbatim.
package configflux

chunk: #Config & {
  "package": "s2_wind_turbine",
  "version": "1.0.0",
  "artifacts": {
    "async_control_driver": {
      "name": "async_control_driver",
      "version": "3.2.0",
      "hash": "sha256-async-control-driver",
      "source": "artifact://wind/async_control_driver.so",
      "target": "/opt/configflux/wind/async_control_driver.so",
      "doc": "Asynchronous control driver for geared turbines"
    },
    "direct_drive_control_driver": {
      "name": "direct_drive_control_driver",
      "version": "3.4.1",
      "hash": "sha256-direct-drive-control-driver",
      "source": "artifact://wind/direct_drive_control_driver.so",
      "target": "/opt/configflux/wind/direct_drive_control_driver.so",
      "doc": "Control driver for direct-drive turbines"
    }
  },
  "components": {
    "grid_interface": {
      "type": "module"
    },
    "sensor_stack": {
      "type": "module"
    },
    "turbine_controller": {
      "type": "controller",
      "depends_on": [
        "grid_interface",
        "sensor_stack"
      ],
      "params": {
        "grid_profile": {
          "inherits": "grid_profile",
          "value": "ieee_1547",
          "overrides": [
            {
              "condition": "grid_code == 'iec_61400'",
              "value": "iec_61400"
            }
          ]
        },
        "control_driver": {
          "inherits": "control_driver_slot",
          "value": "async_control_driver",
          "overrides": [
            {
              "condition": "gearbox_type == 'direct_drive'",
              "value": "direct_drive_control_driver"
            }
          ]
        },
        "pitch_trim_gain": {
          "inherits": "pitch_trim_gain",
          "value": 0.08
        }
      }
    }
  }
}
