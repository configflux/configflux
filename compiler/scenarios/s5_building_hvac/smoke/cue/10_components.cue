// s5_building_hvac/smoke -- 10_components chunk for the CUE authoring front-end
// (ADR 0021, phase 7; configflux-ujh6). Bootstrapped from ../chunks/10_components.toml,
// validated against compiler/cue/schema.cue, and exported to 10_components.json by the
// pinned cue binary (compiler/cue/export_fixtures.sh). The differential gate
// (scenario_cue_equivalence_tests.rs) proves the CUE-derived CMP is byte-identical
// to the TOML-derived CMP. `condition`/`overrides` are opaque pass-through data
// (ADR 0021 decision 1): CUE types and emits them verbatim.
package configflux

chunk: #Config & {
  "package": "s5_building_hvac",
  "version": "1.0.0",
  "artifacts": {
    "office_standard_controller": {
      "name": "office_standard_controller",
      "version": "1.2.0",
      "hash": "sha256-office-standard-controller",
      "source": "artifact://hvac/office_standard_controller.so",
      "target": "/opt/configflux/hvac/office_standard_controller.so",
      "doc": "Standard office HVAC controller package"
    },
    "hospital_hepa_us_controller": {
      "name": "hospital_hepa_us_controller",
      "version": "2.0.0",
      "hash": "sha256-hospital-hepa-us-controller",
      "source": "artifact://hvac/hospital_hepa_us_controller.so",
      "target": "/opt/configflux/hvac/hospital_hepa_us_controller.so",
      "doc": "US hospital HVAC controller package for HEPA isolation zones"
    },
    "hospital_hepa_eu_controller": {
      "name": "hospital_hepa_eu_controller",
      "version": "2.1.0",
      "hash": "sha256-hospital-hepa-eu-controller",
      "source": "artifact://hvac/hospital_hepa_eu_controller.so",
      "target": "/opt/configflux/hvac/hospital_hepa_eu_controller.so",
      "doc": "EU hospital HVAC controller package for HEPA isolation zones"
    }
  },
  "components": {
    "power_distribution": {
      "type": "module"
    },
    "air_handler": {
      "type": "module"
    },
    "pressure_monitor": {
      "type": "module"
    },
    "climate_controller": {
      "type": "controller",
      "depends_on": [
        "power_distribution",
        "air_handler",
        "pressure_monitor"
      ],
      "params": {
        "ventilation_profile": {
          "inherits": "ventilation_profile",
          "value": "office_comfort_vav",
          "req_id": "req_s5_001",
          "overrides": [
            {
              "condition": "occupancy_class == 'hospital' && filtration_grade == 'hepa'",
              "value": "acute_care_isolation"
            }
          ]
        },
        "controller_package": {
          "inherits": "controller_package",
          "value": "office_standard_controller",
          "req_id": "req_s5_002",
          "overrides": [
            {
              "condition": "occupancy_class == 'hospital' && filtration_grade == 'hepa' && region == 'us'",
              "value": "hospital_hepa_us_controller"
            },
            {
              "condition": "occupancy_class == 'hospital' && filtration_grade == 'hepa' && region == 'eu'",
              "value": "hospital_hepa_eu_controller"
            }
          ]
        },
        "airflow_trim_gain": {
          "inherits": "airflow_trim_gain",
          "value": 0.07,
          "req_id": "req_s5_003"
        }
      }
    },
    "isolation_monitor": {
      "type": "module",
      "condition": "occupancy_class == 'hospital' && filtration_grade == 'hepa'"
    }
  }
}
