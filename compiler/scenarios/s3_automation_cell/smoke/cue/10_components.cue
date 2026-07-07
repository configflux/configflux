// s3_automation_cell/smoke -- 10_components chunk for the CUE authoring front-end
// (ADR 0021, phase 7; configflux-ujh6). Bootstrapped from ../chunks/10_components.toml,
// validated against compiler/cue/schema.cue, and exported to 10_components.json by the
// pinned cue binary (compiler/cue/export_fixtures.sh). The differential gate
// (scenario_cue_equivalence_tests.rs) proves the CUE-derived CMP is byte-identical
// to the TOML-derived CMP. `condition`/`overrides` are opaque pass-through data
// (ADR 0021 decision 1): CUE types and emits them verbatim.
package configflux

chunk: #Config & {
  "package": "s3_automation_cell",
  "version": "1.0.0",
  "artifacts": {
    "swift_ring_driver": {
      "name": "swift_ring_driver",
      "version": "1.0.0",
      "hash": "sha256-swift-ring-driver",
      "source": "artifact://automation/swift_ring_driver.so",
      "target": "/opt/configflux/drivers/swift_ring_driver.so",
      "doc": "Driver for swiftmove + opticore ring topology"
    },
    "swift_ring_safety_driver": {
      "name": "swift_ring_safety_driver",
      "version": "1.0.0",
      "hash": "sha256-swift-ring-safety-driver",
      "source": "artifact://automation/swift_ring_safety_driver.so",
      "target": "/opt/configflux/drivers/swift_ring_safety_driver.so",
      "doc": "Safety-enhanced driver for swiftmove ring topology"
    },
    "belt_star_driver": {
      "name": "belt_star_driver",
      "version": "2.1.0",
      "hash": "sha256-belt-star-driver",
      "source": "artifact://automation/belt_star_driver.so",
      "target": "/opt/configflux/drivers/belt_star_driver.so",
      "doc": "Driver for beltmax + camplus star topology"
    },
    "belt_star_safety_driver": {
      "name": "belt_star_safety_driver",
      "version": "2.1.0",
      "hash": "sha256-belt-star-safety-driver",
      "source": "artifact://automation/belt_star_safety_driver.so",
      "target": "/opt/configflux/drivers/belt_star_safety_driver.so",
      "doc": "Safety-enhanced driver for beltmax star topology"
    }
  },
  "components": {
    "cell_root": {
      "type": "cell"
    },
    "swift_ring_standard": {
      "type": "module",
      "depends_on": [
        "cell_root"
      ],
      "condition": "conveyor_brand == 'swiftmove' && vision_stack == 'opticore' && safety_mode == 'pl_d' && network_topology == 'ring'",
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "swift_ring_standard"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "swift_ring_driver"
        }
      }
    },
    "swift_ring_high_safety": {
      "type": "module",
      "depends_on": [
        "cell_root"
      ],
      "condition": "conveyor_brand == 'swiftmove' && vision_stack == 'opticore' && safety_mode == 'pl_e' && network_topology == 'ring'",
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "swift_ring_high_safety"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "swift_ring_safety_driver"
        }
      }
    },
    "belt_star_standard": {
      "type": "module",
      "depends_on": [
        "cell_root"
      ],
      "condition": "conveyor_brand == 'beltmax' && vision_stack == 'camplus' && safety_mode == 'pl_d' && network_topology == 'star'",
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "belt_star_standard"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "belt_star_driver"
        }
      }
    },
    "belt_star_high_safety": {
      "type": "module",
      "depends_on": [
        "cell_root"
      ],
      "condition": "conveyor_brand == 'beltmax' && vision_stack == 'camplus' && safety_mode == 'pl_e' && network_topology == 'star'",
      "params": {
        "profile": {
          "inherits": "module_profile",
          "value": "belt_star_high_safety"
        },
        "driver": {
          "inherits": "driver_slot",
          "value": "belt_star_safety_driver"
        }
      }
    }
  }
}
